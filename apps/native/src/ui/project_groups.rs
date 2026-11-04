//! Projects as named groups of folders (`docs/features/projects.md`): the
//! Projects page's project cards and editor, the sidebar filter for a whole
//! project, and the pending-scratch counts shown beside projects. The model
//! and the membership rules live in `wks_native::project_groups`.
//!
//! A group filter is held in `project_filter` as `group:<id>`, so every
//! screen that already follows the sidebar's project keeps working; the
//! helpers here are how they ask what it holds.
use super::chrome::interactive_control;
use super::*;
use wks_native::features::Request;
use wks_native::project_groups::{self, Draft, Group, Groups};

const GROUP_FILTER: &str = "group:";
/// Folders offered as one-click additions in the editor.
const SUGGESTED_FOLDERS: usize = 8;

pub(super) fn group_filter(id: &str) -> String {
    format!("{GROUP_FILTER}{id}")
}

impl Workspace {
    /// The newest list the hub returned, from a read or from a write (which
    /// answers with the refreshed list).
    pub(super) fn project_groups(&self) -> Groups {
        [
            "project-groups",
            "project-group-save",
            "project-group-remove",
        ]
        .iter()
        .filter_map(|key| self.view.requests.get(*key))
        .filter(|s| !s.loading && s.error.is_none() && s.value["groups"].is_array())
        .max_by_key(|s| s.number)
        .and_then(|s| project_groups::parse(&s.value))
        .unwrap_or_default()
    }

    /// Whether `feature_message(key)` has anything to say.
    pub(super) fn has_feature_message(&self, key: &str) -> bool {
        self.view
            .requests
            .get(key)
            .is_some_and(|s| s.loading || s.error.as_ref().is_some_and(|e| !e.is_empty()))
    }

    /// The project list has been read (or there is nothing to read from), so
    /// an empty one means no projects rather than not yet or not at all.
    pub(super) fn groups_loaded(&self) -> bool {
        self.view
            .requests
            .get("project-groups")
            .is_none_or(|s| !s.loading && s.error.is_none())
    }

    /// Projects and pending scratch counts, read with the folder registry on
    /// every (re)connection and visit to Projects.
    pub(super) fn load_project_groups(&mut self, cx: &mut Context<Self>) {
        if self.demo || !self.view.connected {
            return;
        }
        for (key, request) in [
            ("project-groups", Request::ProjectGroups),
            ("scratch-pending", Request::ScratchPending),
        ] {
            if !self.view.requests.get(key).is_some_and(|s| s.loading) {
                self.request(request, cx);
            }
        }
    }

    /// `(scratch root, pending)` for every scratch holding pending files.
    pub(super) fn pending_roots(&self) -> Vec<(String, usize)> {
        self.view
            .requests
            .get("scratch-pending")
            .filter(|s| s.error.is_none())
            .map(|s| project_groups::pending_roots(&s.value))
            .unwrap_or_default()
    }

    pub(super) fn pending_total(&self) -> usize {
        self.pending_roots().iter().map(|(_, n)| n).sum()
    }

    pub(super) fn pending_in(&self, folders: &[String]) -> usize {
        project_groups::pending_in(&self.pending_roots(), folders)
    }

    /// The group the sidebar is filtered to, if the filter is a project.
    pub(super) fn filter_group(&self) -> Option<Group> {
        let id = self.project_filter.as_deref()?.strip_prefix(GROUP_FILTER)?;
        self.project_groups().get(id).cloned()
    }

    /// The folders the sidebar's project filter covers: a group's folders,
    /// or the one filtered folder.
    pub(super) fn filter_folders(&self) -> Vec<String> {
        match self.project_filter.as_deref() {
            Some(f) if f.starts_with(GROUP_FILTER) => {
                self.filter_group().map(|g| g.folders).unwrap_or_default()
            }
            Some(path) => vec![path.to_owned()],
            None => Vec::new(),
        }
    }

    /// A folder to start from for the filter: the filtered folder, or a
    /// project's first folder.
    pub(super) fn filter_folder(&self) -> Option<String> {
        self.filter_folders().into_iter().next()
    }

    /// Whether the session at `ix` is in the filtered project. A folder
    /// filter matches the folder exactly (workers stay visible through their
    /// lineage); a project matches any of its folders, worktrees included.
    pub(super) fn filter_holds(&self, groups: &Groups, ix: usize) -> bool {
        let sessions = &self.view.sessions;
        match self.project_filter.as_deref() {
            None => true,
            Some(f) if f.starts_with(GROUP_FILTER) => {
                let id = &f[GROUP_FILTER.len()..];
                groups
                    .of_session(sessions, ix, self.worktree_root())
                    .is_some_and(|g| g.id == id)
            }
            Some(path) => wks_native::projects::same_dir(&sessions[ix].cwd, path),
        }
    }

    /// Filter the sidebar to a project and show its newest session, or the
    /// New Agent form in its first folder when nothing runs there yet.
    pub(super) fn open_group(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.project_groups();
        let Some(group) = groups.get(id).cloned() else {
            return;
        };
        self.project_filter = Some(group_filter(id));
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        let root = self.worktree_root();
        let session = (0..self.view.sessions.len())
            .find(|ix| {
                groups
                    .of_session(&self.view.sessions, *ix, root)
                    .is_some_and(|g| g.id == id)
            })
            .map(|ix| self.view.sessions[ix].id.clone());
        match session {
            Some(session) => {
                self.show_screen(Screen::Conversation, window, cx);
                self.project_filter = Some(group_filter(id));
                self.command(Command::Select(session), cx);
            }
            None => {
                if let Some(folder) = group.folders.first().cloned() {
                    self.new_agent_in_project(folder, window, cx);
                    self.project_filter = Some(group_filter(id));
                }
            }
        }
        cx.notify();
    }

    // ---- editor ---------------------------------------------------------

    pub(super) fn edit_group(
        &mut self,
        group: Option<&Group>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = group.map(Draft::of).unwrap_or_default();
        for (input, value) in [
            (&self.extras.group_name, draft.name.clone()),
            (&self.extras.group_icon, draft.icon.clone()),
            (&self.extras.group_color, draft.color.clone()),
            (&self.extras.group_folder, String::new()),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.extras.group_draft = Some(draft);
        self.extras.group_confirm_delete = false;
        self.extras.group_error.clear();
        self.extras
            .group_name
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_group_editor(&mut self, cx: &mut Context<Self>) {
        self.extras.group_draft = None;
        self.extras.group_confirm_delete = false;
        self.extras.group_error.clear();
        cx.notify();
    }

    fn add_group_folder(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.extras.group_draft.as_mut() else {
            return;
        };
        let path = path.trim().to_owned();
        if !wks_native::launch::absolute_directory(&path) {
            self.extras.group_error = "Enter an absolute folder path.".into();
        } else if !draft.add_folder(&path) {
            self.extras.group_error = "That folder is already in this project.".into();
        } else {
            self.extras.group_error.clear();
            self.extras
                .group_folder
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    fn save_group(&mut self, cx: &mut Context<Self>) {
        let Some(mut draft) = self.extras.group_draft.clone() else {
            return;
        };
        draft.name = self.extras.group_name.read(cx).value().to_string();
        draft.icon = self.extras.group_icon.read(cx).value().to_string();
        draft.color = self.extras.group_color.read(cx).value().to_string();
        match draft.params(self.project_groups().revision) {
            Ok(params) => {
                self.extras.group_error.clear();
                self.extras.group_save_sent = true;
                self.request(Request::SaveProjectGroup { params }, cx);
            }
            Err(error) => self.extras.group_error = error.to_string(),
        }
        cx.notify();
    }

    fn delete_group(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.extras.group_draft.as_ref().and_then(|d| d.id.clone()) else {
            return;
        };
        if !self.extras.group_confirm_delete {
            self.extras.group_confirm_delete = true;
            cx.notify();
            return;
        }
        let revision = self.project_groups().revision;
        self.extras.group_save_sent = true;
        self.request(
            Request::RemoveProjectGroup {
                id: id.clone(),
                revision,
            },
            cx,
        );
        if self
            .project_filter
            .as_deref()
            .is_some_and(|f| f == group_filter(&id))
        {
            self.project_filter = None;
        }
    }

    /// Close the editor once its save or delete lands; keep it open (with
    /// the hub's reason) when the hub refuses.
    pub(super) fn sync_project_groups(&mut self, next: &View) {
        if !self.extras.group_save_sent {
            return;
        }
        let settled = ["project-group-save", "project-group-remove"]
            .iter()
            .filter_map(|key| next.requests.get(*key))
            .filter(|s| !s.loading)
            .max_by_key(|s| s.number);
        let still_loading = ["project-group-save", "project-group-remove"]
            .iter()
            .any(|key| next.requests.get(*key).is_some_and(|s| s.loading));
        let Some(state) = settled.filter(|_| !still_loading) else {
            return;
        };
        self.extras.group_save_sent = false;
        match &state.error {
            Some(error) => {
                self.extras.group_error = error
                    .strip_prefix("conflict:")
                    .map(|_| {
                        "This project changed elsewhere. Close the editor and open it again."
                            .to_owned()
                    })
                    .unwrap_or_else(|| error.clone());
            }
            None => {
                let removed = matches!(state.request, Request::RemoveProjectGroup { .. });
                self.extras.group_draft = None;
                self.extras.group_confirm_delete = false;
                self.extras.group_error.clear();
                self.projects.notice = if removed {
                    "Project deleted. Its folders are still listed below.".into()
                } else {
                    "Project saved.".into()
                };
            }
        }
    }

    // ---- rendering ------------------------------------------------------

    /// Session counts (all, running) per group id.
    pub(super) fn group_sessions(&self, groups: &Groups) -> HashMap<String, (usize, usize)> {
        let root = self.worktree_root();
        let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
        for ix in 0..self.view.sessions.len() {
            if let Some(group) = groups.of_session(&self.view.sessions, ix, root) {
                let entry = counts.entry(group.id.clone()).or_default();
                entry.0 += 1;
                if !self.view.sessions[ix].stopped() {
                    entry.1 += 1;
                }
            }
        }
        counts
    }

    /// The Projects page's projects: a card per group, the New project
    /// action, and the editor when it is open.
    pub(super) fn render_project_groups(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let groups = self.project_groups();
        let can_write = self.view.connected
            && !self.demo
            && self.view.may_call("projectGroups.save")
            && !self.extras.group_save_sent;
        let counts = self.group_sessions(&groups);
        let editing_new = self
            .extras
            .group_draft
            .as_ref()
            .is_some_and(|d| d.id.is_none());
        let editing = |id: &str| {
            self.extras
                .group_draft
                .as_ref()
                .and_then(|d| d.id.as_deref())
                == Some(id)
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .debug_selector(|| "project-groups".into())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(super::projects::section_label("PROJECTS", p))
                    .child(
                        // One editor at a time: New project rests while one is open.
                        self.pquiet(
                            "new-project-group",
                            "New project",
                            IconName::Plus,
                            can_write && self.extras.group_draft.is_none(),
                        )
                            .debug_selector(|| "new-project-group".into())
                            .when(can_write && self.extras.group_draft.is_none(), |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.edit_group(None, window, cx)
                                }))
                            }),
                    ),
            )
            // Only a read in flight or failed takes a line; an empty one
            // would still take the gap.
            .when(self.has_feature_message("project-groups"), |d| {
                d.child(self.feature_message("project-groups"))
            })
            .when(editing_new, |d| d.child(self.render_group_editor(cx)))
            .when(
                groups.groups.is_empty() && !editing_new && self.groups_loaded(),
                |d| {
                    d.child(
                        div()
                            .debug_selector(|| "project-groups-empty".into())
                            .px_3()
                            .py(px(10.))
                            .rounded(px(p.panel_radius))
                            .border_1()
                            .border_dashed()
                            .border_color(rgb(p.border))
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Icon::new(IconName::FolderClosed)
                                    .size(px(16.))
                                    .text_color(rgb(p.muted)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(chrome::scale::META))
                                    .text_color(rgb(p.muted))
                                    .child("No projects yet. Group an app's api, web and infra folders into one project to filter, launch and read scratch across them."),
                            ),
                    )
                },
            )
            .children(groups.groups.iter().enumerate().map(|(ix, group)| {
                if editing(&group.id) {
                    self.render_group_editor(cx)
                } else {
                    self.render_group_card(
                        ix,
                        group,
                        counts.get(&group.id).copied(),
                        can_write,
                        compact,
                        cx,
                    )
                }
            }))
    }

    fn render_group_card(
        &self,
        ix: usize,
        group: &Group,
        sessions: Option<(usize, usize)>,
        can_write: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let (all, live) = sessions.unwrap_or_default();
        let pending = self.pending_in(&group.folders);
        let folders = group.folders.len();
        let mut meta = format!("{folders} folder{}", if folders == 1 { "" } else { "s" });
        if all > 0 {
            meta = format!("{meta} · {all} session{}", if all == 1 { "" } else { "s" });
        }
        let open = group.id.clone();
        let spawn = group.folders.first().cloned();
        let edit = group.clone();
        let scratch = group.id.clone();
        let can_spawn = self.view.connected && !self.demo && spawn.is_some();
        // A fill like the folder cards below, not another bordered box.
        div()
            .rounded(px(p.panel_radius))
            .bg(rgb(p.surface))
            .overflow_hidden()
            .debug_selector(move || format!("project-group-{ix}"))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("project-group", ix))
                    .pl_3()
                    .pr_2()
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(p.selected)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_group(&open, window, cx)),
                    )
                    .child(self.project_mark(Some(&group.as_known()), &group.as_known().path, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(chrome::scale::BODY))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(group.name.clone()),
                                    )
                                    .when(pending > 0, |d| {
                                        d.child(
                                            div()
                                                .id(("project-group-pending", ix))
                                                .debug_selector(move || {
                                                    format!("project-group-pending-{ix}")
                                                })
                                                .flex_shrink_0()
                                                .px(px(6.))
                                                .rounded_full()
                                                .bg(gpui::Hsla::from(rgb(p.warning)).opacity(0.15))
                                                .text_size(px(chrome::scale::CAPTION))
                                                .text_color(rgb(p.warning))
                                                .cursor_pointer()
                                                .hover(move |s| {
                                                    s.bg(gpui::Hsla::from(rgb(p.warning))
                                                        .opacity(0.28))
                                                })
                                                .tooltip(|window, cx| {
                                                    gpui_component::tooltip::Tooltip::new(
                                                        "Open this project's scratch",
                                                    )
                                                    .build(window, cx)
                                                })
                                                .child(format!("{pending} pending"))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.extras.scratch_preset =
                                                            Some(group_filter(&scratch));
                                                        this.open_feature(
                                                            Screen::Scratch,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        )
                                    }),
                            )
                            .child(
                                // Running work in the busy colour, as on the
                                // folder cards; the rest is detail.
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .when(live > 0, |d| {
                                        d.child(
                                            div()
                                                .flex_shrink_0()
                                                .size(px(6.))
                                                .rounded_full()
                                                .bg(rgb(p.busy)),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .text_color(rgb(p.busy))
                                                .child(format!("{live} running")),
                                        )
                                        .child("·")
                                    })
                                    .child(div().min_w_0().truncate().child(meta)),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                if compact {
                                    self.picon(
                                        format!("project-group-new-agent-{ix}"),
                                        "New agent in this project",
                                        IconName::Plus,
                                        can_spawn,
                                    )
                                } else {
                                    self.pquiet(
                                        format!("project-group-new-agent-{ix}"),
                                        "New agent",
                                        IconName::Plus,
                                        can_spawn,
                                    )
                                }
                                .debug_selector(move || format!("project-group-new-agent-{ix}"))
                                .when_some(
                                    spawn.filter(|_| can_spawn),
                                    |d, folder| {
                                        let id = group.id.clone();
                                        d.on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.new_agent_in_project(folder.clone(), window, cx);
                                            this.project_filter = Some(group_filter(&id));
                                        }))
                                    },
                                ),
                            )
                            .child(
                                self.picon(
                                    format!("project-group-edit-{ix}"),
                                    "Edit project: name, icon, colour and folders",
                                    IconName::Settings,
                                    can_write,
                                )
                                .debug_selector(move || format!("project-group-edit-{ix}"))
                                .when(can_write, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.edit_group(Some(&edit), window, cx)
                                    }))
                                }),
                            ),
                    ),
            )
            // Folders sit under the name, past the mark.
            .child(
                div()
                    .pl(px(56.))
                    .pr_4()
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(
                        // Each folder by name first; the path is detail, cut to fit.
                        group.folders.iter().map(|folder| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(px(11.))
                                        .text_color(rgb(p.muted)),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(chrome::scale::META))
                                        .child(self.project_name(folder)),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(mono_font())
                                        .text_size(px(chrome::scale::CAPTION))
                                        .text_color(rgb(p.muted))
                                        .child(folder.clone()),
                                )
                        }),
                    ),
            )
    }

    pub(super) fn render_group_editor(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let Some(draft) = self.extras.group_draft.clone() else {
            return div();
        };
        let busy = self.extras.group_save_sent;
        let enabled = self.view.connected && !self.demo && !busy;
        let groups = self.project_groups();
        let other_group = |path: &str| {
            groups
                .groups
                .iter()
                .find(|g| {
                    Some(&g.id) != draft.id.as_ref()
                        && g.folders
                            .iter()
                            .any(|f| wks_native::projects::same_dir(f, path))
                })
                .map(|g| g.name.clone())
        };
        let suggestions: Vec<String> = self
            .known_projects()
            .into_iter()
            .filter(|k| k.listed("") && !k.path.starts_with("group:"))
            .filter(|k| {
                !draft
                    .folders
                    .iter()
                    .any(|f| wks_native::projects::same_dir(f, &k.path))
            })
            .filter(|k| other_group(&k.path).is_none())
            .map(|k| k.path)
            .take(SUGGESTED_FOLDERS)
            .collect();
        let field = |label: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(super::projects::section_label(label, p))
                .child(Input::new(input).disabled(!enabled))
        };
        let folders = draft.folders.iter().enumerate().map(|(ix, folder)| {
            let remove = folder.clone();
            div()
                .pl_2()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    Icon::new(IconName::Folder)
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(chrome::scale::META))
                        .child(self.project_name(folder)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font())
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(folder.clone()),
                )
                .child(
                    self.picon(
                        format!("group-folder-remove-{ix}"),
                        "Remove from this project (the folder itself is untouched)",
                        IconName::Close,
                        enabled,
                    )
                    .debug_selector(move || format!("group-folder-remove-{ix}"))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(draft) = this.extras.group_draft.as_mut() {
                                draft.remove_folder(&remove);
                            }
                            cx.notify();
                        }))
                    }),
                )
        });
        let suggestions_empty = suggestions.is_empty();
        let chips = suggestions.into_iter().enumerate().map(|(ix, path)| {
            let add = path.clone();
            interactive_control(div().id(("group-folder-suggest", ix)), p, enabled)
                .debug_selector(move || format!("group-folder-suggest-{ix}"))
                .h(px(24.))
                .px_2()
                .rounded_full()
                .bg(chrome::tonal(p, 0.06))
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .hover(move |s| s.bg(chrome::tonal(p, 0.12)).text_color(rgb(p.text)))
                .flex()
                .items_center()
                .gap_1()
                .child(Icon::new(IconName::Plus).size(px(10.)))
                .child(self.project_name(&path))
                .tooltip({
                    let path = SharedString::from(path.clone());
                    move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(path.clone()).build(window, cx)
                    }
                })
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        this.add_group_folder(add.clone(), window, cx)
                    }))
                })
        });
        let existing = draft.id.is_some();
        let confirming = self.extras.group_confirm_delete;
        let delete_label = if confirming {
            "Delete project"
        } else {
            "Delete"
        };
        // The mark follows the form as typed.
        let name = self.extras.group_name.read(cx).value().trim().to_owned();
        let icon = self.extras.group_icon.read(cx).value().trim().to_owned();
        let colour = self.extras.group_color.read(cx).value().trim().to_owned();
        let colour = colour
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok());
        let preview = Group {
            id: draft.id.clone().unwrap_or_default(),
            name: if name.is_empty() {
                "New project".into()
            } else {
                name.clone()
            },
            folders: draft.folders.clone(),
            color: colour,
            icon: (!icon.is_empty()).then_some(icon),
        }
        .as_known();
        chrome::card(p)
            .debug_selector(|| "project-group-editor".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.project_mark(Some(&preview), &preview.path, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(chrome::scale::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if existing {
                                        "Edit project"
                                    } else {
                                        "New project"
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child("A project groups folders; the folders themselves stay where they are."),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(200.))
                            .child(field("Name", &self.extras.group_name)),
                    )
                    .child(
                        div()
                            .w(px(96.))
                            .child(field("Icon", &self.extras.group_icon)),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .child(field("Colour", &self.extras.group_color)),
                    ),
            )
            .child(div().pt_1().child(super::projects::section_label("Folders", p)))
            .when(draft.folders.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child("No folders yet. Add the repositories this project spans."),
                )
            })
            .when(!draft.folders.is_empty(), |d| {
                d.child(div().flex().flex_col().children(folders))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.))
                            .child(Input::new(&self.extras.group_folder).disabled(!enabled)),
                    )
                    .child(
                        self.pbutton("group-folder-add", "Add folder", enabled)
                            .debug_selector(|| "group-folder-add".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    let path =
                                        this.extras.group_folder.read(cx).value().to_string();
                                    this.add_group_folder(path, window, cx)
                                }))
                            }),
                    ),
            )
            .when(!suggestions_empty, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .pr_1()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child("Suggested"),
                        )
                        .children(chips),
                )
            })

            .when(!self.extras.group_error.is_empty(), |d| {
                d.child(
                    chrome::notice_line(self.extras.group_error.clone(), chrome::Tone::Error, p)
                        .debug_selector(|| "project-group-error".into()),
                )
            })
            .when(busy, |d| {
                d.child(chrome::notice_line("Saving…", chrome::Tone::Loading, p))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .when(existing, |d| {
                        d.child(
                            self.pdanger("project-group-delete", delete_label, enabled)
                                .debug_selector(|| "project-group-delete".into())
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| this.delete_group(cx)))
                                }),
                        )
                    })
                    // Asked twice: the second press says what it keeps.
                    .when(existing && confirming, |d| {
                        d.child(
                            div()
                                .pl_2()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.warning))
                                .child("Press again to delete; folders stay."),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        self.pbutton("project-group-cancel", "Cancel", !busy)
                            .debug_selector(|| "project-group-cancel".into())
                            .when(!busy, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.close_group_editor(cx)),
                                )
                            }),
                    )
                    .child(
                        self.pprimary("project-group-save", "Save project", enabled)
                            .debug_selector(|| "project-group-save".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.save_group(cx)))
                            }),
                    ),
            )
    }
}
