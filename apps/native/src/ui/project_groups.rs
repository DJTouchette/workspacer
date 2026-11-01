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
    fn group_sessions(&self, groups: &Groups) -> HashMap<String, (usize, usize)> {
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
    pub(super) fn render_project_groups(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
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
                        self.quiet_button("new-project-group", "New project", IconName::Plus, can_write)
                            .debug_selector(|| "new-project-group".into())
                            .when(can_write && self.extras.group_draft.is_none(), |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.edit_group(None, window, cx)
                                }))
                            }),
                    ),
            )
            .child(self.feature_message("project-groups"))
            .when(editing_new, |d| d.child(self.render_group_editor(cx)))
            .when(groups.groups.is_empty() && !editing_new, |d| {
                d.child(
                    div()
                        .py_2()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child("Group folders that belong together, such as an app's api, web and infra repositories, into one project."),
                )
            })
            .children(groups.groups.iter().enumerate().map(|(ix, group)| {
                if editing(&group.id) {
                    self.render_group_editor(cx)
                } else {
                    self.render_group_card(ix, group, counts.get(&group.id).copied(), can_write, cx)
                }
            }))
    }

    fn render_group_card(
        &self,
        ix: usize,
        group: &Group,
        sessions: Option<(usize, usize)>,
        can_write: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let (all, live) = sessions.unwrap_or_default();
        let pending = self.pending_in(&group.folders);
        let folders = group.folders.len();
        let mut meta = format!("{folders} folder{}", if folders == 1 { "" } else { "s" });
        if all > 0 {
            meta = format!(
                "{meta} · {live} running · {all} session{}",
                if all == 1 { "" } else { "s" }
            );
        }
        let open = group.id.clone();
        let spawn = group.folders.first().cloned();
        let edit = group.clone();
        let scratch = group.id.clone();
        let can_spawn = self.view.connected && !self.demo && spawn.is_some();
        chrome::card(p)
            .debug_selector(move || format!("project-group-{ix}"))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("project-group", ix))
                    .px_4()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .hover(move |s| s.bg(gpui::Hsla::from(rgb(p.selected)).opacity(0.5)))
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
                                                .px_2()
                                                .rounded_full()
                                                .bg(gpui::Hsla::from(rgb(p.warning)).opacity(0.15))
                                                .text_size(px(chrome::scale::CAPTION))
                                                .text_color(rgb(p.warning))
                                                .cursor_pointer()
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
                                div()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(meta),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                self.button(
                                    format!("project-group-new-agent-{ix}"),
                                    "New agent",
                                    can_spawn,
                                )
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
                                self.icon_button(
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
            .child(div().px_4().pb_3().flex().flex_col().gap(px(2.)).children(
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
            ))
    }

    fn render_group_editor(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
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
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Folder)
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font())
                        .text_size(px(chrome::scale::CAPTION))
                        .child(folder.clone()),
                )
                .child(
                    self.icon_button(
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
        let chips = suggestions.into_iter().enumerate().map(|(ix, path)| {
            let add = path.clone();
            interactive_control(div().id(("group-folder-suggest", ix)), p, enabled)
                .debug_selector(move || format!("group-folder-suggest-{ix}"))
                .px_2()
                .py(px(2.))
                .rounded(px(p.control_radius))
                .bg(rgb(p.surface))
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .hover(|s| s.text_color(rgb(p.text)))
                .flex()
                .items_center()
                .gap_1()
                .child(Icon::new(IconName::Plus).size(px(10.)))
                .child(wks_native::projects::basename(&path).to_owned())
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        this.add_group_folder(add.clone(), window, cx)
                    }))
                })
        });
        let existing = draft.id.is_some();
        let delete_label = if self.extras.group_confirm_delete {
            "Delete project (folders stay)"
        } else {
            "Delete"
        };
        chrome::card(p)
            .debug_selector(|| "project-group-editor".into())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
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
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(200.))
                            .child(field("NAME", &self.extras.group_name)),
                    )
                    .child(
                        div()
                            .w(px(96.))
                            .child(field("ICON", &self.extras.group_icon)),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .child(field("COLOUR", &self.extras.group_color)),
                    ),
            )
            .child(super::projects::section_label("FOLDERS", p))
            .when(draft.folders.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child("No folders yet. Add the repositories this project spans."),
                )
            })
            .children(folders)
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
                        self.button("group-folder-add", "Add folder", enabled)
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
            .child(div().flex().flex_wrap().gap_1().children(chips))
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
                            self.danger_button("project-group-delete", delete_label, enabled)
                                .debug_selector(|| "project-group-delete".into())
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(|this, _, _, cx| this.delete_group(cx)))
                                }),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        self.button("project-group-cancel", "Cancel", !busy)
                            .debug_selector(|| "project-group-cancel".into())
                            .when(!busy, |d| {
                                d.on_click(
                                    cx.listener(|this, _, _, cx| this.close_group_editor(cx)),
                                )
                            }),
                    )
                    .child(
                        self.primary_button("project-group-save", "Save project", enabled)
                            .debug_selector(|| "project-group-save".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.save_group(cx)))
                            }),
                    ),
            )
    }
}
