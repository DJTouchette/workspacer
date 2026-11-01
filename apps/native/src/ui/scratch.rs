//! The Scratch page: a project's personal files kept outside git — test
//! harnesses, repros, SQL meant to be run by hand — shared by every worktree
//! and kept when worktrees are removed (`docs/features/scratch.md`).
//!
//! The page shows one project: a folder, or a named project of several
//! folders (`docs/features/projects.md`), whose folders' scratch is listed in
//! a section each. It reads and marks. It never runs a file and never writes
//! one's contents: agents save files (`save_scratch` or straight into
//! `$WORKSPACER_SCRATCH`), and here the user sees what is waiting on them,
//! reads it, and records that it ran. The model lives in
//! `wks_native::scratch`.
use super::chrome::interactive_control;
use super::*;
use gpui::AnyElement;
use wks_native::features::Request;
use wks_native::scratch::{self, Content, Filter, Item, Listing, Status};
use wks_native::{jobs, timing::now_ms, tool_preview};

pub(super) const SCRATCH_DESCRIPTION: &str = "Personal files kept outside git and shared by every worktree: harnesses, repros and SQL meant to be run by hand.";
const NO_PROJECT: &str = "Open an agent or a project to see its scratch.";
const NO_SCRATCH: &str = "Nothing in this project's scratch yet. Agents keep repros, one-off scripts and harnesses here; ask one to save something.";
const NO_OPEN: &str = "Nothing open. Everything here is done or archived; switch to All to see it.";
const NO_PENDING: &str = "Nothing is waiting on you.";
const FOOTER: &str = "Workspacer never runs scratch files. Pending means a file is waiting for you to run it by hand; mark it done when you have.";
const UNRECORDED: &str = "No purpose recorded. An agent wrote this straight into the folder.";
/// Projects the switcher offers.
const SWITCHER_PROJECTS: usize = 8;

const STATUS_ACTIONS: [(Status, &str, &str); 4] = [
    (Status::Pending, "pending", "Mark pending"),
    (Status::Done, "done", "Mark done"),
    (Status::Archived, "archived", "Archive"),
    (Status::Draft, "draft", "Back to draft"),
];

/// One listed file and the folder whose scratch holds it.
struct Row<'a> {
    cwd: &'a str,
    listing: &'a Listing,
    item: &'a Item,
}

impl Workspace {
    /// Where the page looks when it opens: a project a badge asked for, the
    /// sidebar's project, the open conversation's project (or folder), the
    /// last folder chosen, else the first project the hub knows. Fixed for as
    /// long as the page stays open, so selecting another session does not
    /// swap the list underneath the user.
    pub(super) fn open_scratch(&mut self, cx: &mut Context<Self>) {
        self.extras.scratch_open = None;
        let groups = self.project_groups();
        let root = self.worktree_root().to_owned();
        let preset = self.extras.scratch_preset.take();
        let selected = self
            .view
            .sessions
            .iter()
            .position(|s| Some(&s.id) == self.view.selected.as_ref());
        self.extras.scratch_project = preset
            .or_else(|| self.project_filter.clone())
            .or_else(|| {
                let ix = selected?;
                match groups.of_session(&self.view.sessions, ix, &root) {
                    Some(group) => Some(super::project_groups::group_filter(&group.id)),
                    None => Some(self.view.sessions[ix].cwd.clone()).filter(|c| !c.is_empty()),
                }
            })
            .or_else(|| Some(self.projects.cwd.clone()).filter(|cwd| !cwd.is_empty()))
            .or_else(|| {
                groups
                    .groups
                    .first()
                    .map(|g| super::project_groups::group_filter(&g.id))
            })
            .or_else(|| self.known_projects().first().map(|p| p.path.clone()));
        self.refresh_scratch(cx);
    }

    /// The folders the page lists: a project's, or the one folder.
    pub(super) fn scratch_folders(&self) -> Vec<String> {
        let Some(target) = self.extras.scratch_project.as_deref() else {
            return Vec::new();
        };
        match target.strip_prefix("group:") {
            Some(id) => self
                .project_groups()
                .get(id)
                .map(|g| g.folders.clone())
                .unwrap_or_default(),
            None => vec![target.to_owned()],
        }
    }

    pub(super) fn refresh_scratch(&mut self, cx: &mut Context<Self>) {
        let cwds = self.scratch_folders();
        if cwds.is_empty() {
            return;
        }
        self.request(Request::ScratchList { cwds }, cx);
        if let Some((cwd, path)) = self.extras.scratch_open.clone() {
            self.request(Request::ScratchRead { cwd, path }, cx);
        }
    }

    fn pick_scratch_project(&mut self, target: String, cx: &mut Context<Self>) {
        self.extras.scratch_project = Some(target);
        self.extras.scratch_open = None;
        self.refresh_scratch(cx);
    }

    /// The newest lists the hub returned, from a read or from a mark (which
    /// answers with the refreshed lists), with the folder each one is for.
    pub(super) fn scratch_listings(&self) -> Option<Vec<(String, Listing)>> {
        let state = ["scratch", "scratch-mark"]
            .iter()
            .filter_map(|key| self.view.requests.get(*key))
            .filter(|s| !s.loading && s.error.is_none() && s.value["lists"].is_array())
            .max_by_key(|s| s.number)?;
        let cwds = match &state.request {
            Request::ScratchList { cwds } => cwds.clone(),
            Request::ScratchMark { relist, .. } => relist.clone(),
            _ => return None,
        };
        let lists = state.value["lists"].as_array()?;
        Some(
            cwds.into_iter()
                .zip(lists)
                .filter_map(|(cwd, list)| Some((cwd, scratch::parse_list(list)?)))
                .collect(),
        )
    }

    fn scratch_busy(&self) -> bool {
        self.view
            .requests
            .get("scratch-mark")
            .is_some_and(|s| s.loading)
    }

    fn toggle_scratch(
        &mut self,
        cwd: &str,
        item: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (cwd.to_owned(), item.path.clone());
        if self.extras.scratch_open.as_ref() == Some(&key) {
            self.extras.scratch_open = None;
            cx.notify();
            return;
        }
        self.extras.scratch_open = Some(key);
        let purpose = item.purpose.clone().unwrap_or_default();
        let note = item.note.clone().unwrap_or_default();
        self.extras
            .scratch_purpose
            .update(cx, |input, cx| input.set_value(purpose, window, cx));
        self.extras
            .scratch_note
            .update(cx, |input, cx| input.set_value(note, window, cx));
        self.request(
            Request::ScratchRead {
                cwd: cwd.to_owned(),
                path: item.path.clone(),
            },
            cx,
        );
        cx.notify();
    }

    fn mark_scratch(
        &mut self,
        cwd: String,
        path: String,
        patch: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let relist = self.scratch_folders();
        if relist.is_empty() {
            return;
        }
        self.request(
            Request::ScratchMark {
                cwd,
                path,
                patch,
                relist,
            },
            cx,
        );
    }

    /// The purpose/note editor's Save: sends only what changed.
    fn save_scratch_text(&mut self, cwd: String, item: &Item, cx: &mut Context<Self>) {
        let purpose = self.extras.scratch_purpose.read(cx).value().to_string();
        let note = self.extras.scratch_note.read(cx).value().to_string();
        match scratch::text_patch(item, &purpose, &note) {
            Some(patch) => self.mark_scratch(cwd, item.path.clone(), patch, cx),
            None => {
                self.extras.notice = "Nothing changed.".into();
                cx.notify();
            }
        }
    }

    pub(super) fn render_scratch(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let page = div().flex().flex_col().gap_3();
        if self.extras.scratch_project.is_none() {
            return page.child(empty_line(NO_PROJECT, p));
        }
        let listings = self.scratch_listings();
        let loading = self.view.requests.get("scratch").is_some_and(|s| s.loading);
        let page = page
            .child(self.scratch_project_bar(listings.as_deref(), cx))
            .child(self.feature_message("scratch"))
            .child(self.feature_message("scratch-mark"));
        let Some(listings) = listings else {
            return page;
        };
        let filter = self.extras.scratch_filter;
        let rows: Vec<Row> = listings
            .iter()
            .flat_map(|(cwd, listing)| {
                listing
                    .items
                    .iter()
                    .map(move |item| Row { cwd, listing, item })
            })
            .collect();
        let open = rows.iter().filter(|r| r.item.status.open()).count();
        let pending = rows
            .iter()
            .filter(|r| r.item.status == Status::Pending)
            .count();
        let page = page.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(self.segmented(
                    "scratch-filter",
                    vec![
                        (Filter::Open, format!("Open {open}")),
                        (Filter::Pending, format!("Pending {pending}")),
                        (Filter::All, format!("All {}", rows.len())),
                    ],
                    filter,
                    |this, filter, _, cx| {
                        this.extras.scratch_filter = filter;
                        cx.notify();
                    },
                    cx,
                ))
                .when(pending > 0, |d| {
                    d.child(
                        chrome::notice_line(
                            if pending == 1 {
                                "1 file is waiting for you to run it.".to_owned()
                            } else {
                                format!("{pending} files are waiting for you to run them.")
                            },
                            chrome::Tone::Warning,
                            p,
                        )
                        .debug_selector(|| "scratch-pending".into()),
                    )
                }),
        );
        let sectioned = listings.len() > 1;
        let mut ix = 0;
        let mut body = div().flex().flex_col().gap_3();
        for (cwd, listing) in &listings {
            let shown: Vec<&Row> = rows
                .iter()
                .filter(|r| std::ptr::eq(r.listing, listing) && filter.admits(r.item))
                .collect();
            if sectioned {
                body = body.child(self.scratch_section(cwd, listing, cx));
            }
            body = body
                .when_some(listing.index_error.clone(), |d, error| {
                    d.child(chrome::notice_line(
                        format!("The scratch index could not be read, so purposes and statuses are missing: {error}"),
                        chrome::Tone::Warning,
                        p,
                    ))
                })
                .when(listing.truncated, |d| {
                    d.child(chrome::notice_line(
                        "This scratch holds more files than the page lists.",
                        chrome::Tone::Info,
                        p,
                    ))
                });
            for row in shown {
                body = body.child(self.render_scratch_item(ix, row, window, cx));
                ix += 1;
            }
        }
        let empty = match filter {
            _ if rows.is_empty() => NO_SCRATCH,
            Filter::Pending => NO_PENDING,
            _ => NO_OPEN,
        };
        page.when(ix == 0 && !loading, |d| {
            d.child(empty_line(empty, p).debug_selector(|| "scratch-empty".into()))
        })
        .child(body)
        .child(
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(FOOTER),
        )
    }

    /// A folder's heading inside a multi-folder project.
    fn scratch_section(&self, cwd: &str, listing: &Listing, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let pending = listing.pending();
        div()
            .pt_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Folder)
                    .size(px(12.))
                    .text_color(rgb(p.muted)),
            )
            .child(
                div()
                    .text_size(px(chrome::scale::META))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.project_name(cwd)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(listing.root.clone()),
            )
            .when(pending > 0, |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.warning))
                        .child(format!("{pending} pending")),
                )
            })
            .child(div().flex_1())
            .children(self.folder_actions(&listing.files, cx))
    }

    /// Copy (and, when the hub is on this computer, open) a scratch folder.
    fn folder_actions(&self, folder: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if folder.is_empty() {
            return Vec::new();
        }
        let id = SharedString::from(format!("scratch-copy-folder-{}", folder.len()));
        let copy = folder.to_owned();
        let mut out = vec![
            self.quiet_button(id, "Copy folder path", IconName::Copy, true)
                .debug_selector(|| "scratch-copy-folder".into())
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                }))
                .into_any_element(),
        ];
        if self.extras.local_paths {
            let open = folder.to_owned();
            out.push(
                self.quiet_button(
                    SharedString::from(format!("scratch-reveal-folder-{}", folder.len())),
                    "Show folder",
                    IconName::FolderOpen,
                    true,
                )
                .on_click(
                    cx.listener(move |_, _, _, cx| {
                        cx.open_with_system(std::path::Path::new(&open))
                    }),
                )
                .into_any_element(),
            );
        }
        out
    }

    /// The project shown — its mark, name and where its files live — and the
    /// other projects the user can switch to.
    fn scratch_project_bar(
        &self,
        listings: Option<&[(String, Listing)]>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let target = self.extras.scratch_project.clone().unwrap_or_default();
        let groups = self.project_groups();
        let (identity, name) = match target.strip_prefix("group:").and_then(|id| groups.get(id)) {
            Some(group) => (group.as_known(), group.name.clone()),
            None => {
                let known = self
                    .known_projects()
                    .into_iter()
                    .find(|k| wks_native::projects::same_dir(&k.path, &target));
                let name = self.project_name(&target);
                (
                    known.unwrap_or_else(|| {
                        let mut k = wks_native::project_groups::Group {
                            id: String::new(),
                            name: name.clone(),
                            folders: vec![],
                            color: None,
                            icon: None,
                        }
                        .as_known();
                        k.path = target.clone();
                        k
                    }),
                    name,
                )
            }
        };
        // Projects first, then folders in no project; worktree checkouts and
        // folders only a session knows stay out of the way.
        let mut choices: Vec<(String, wks_native::projects::KnownProject)> = groups
            .groups
            .iter()
            .map(|g| (super::project_groups::group_filter(&g.id), g.as_known()))
            .collect();
        choices.extend(
            self.known_projects()
                .into_iter()
                .filter(|k| k.listed("") && !groups.grouped(&k.path))
                .map(|k| (k.path.clone(), k)),
        );
        let chips = choices
            .into_iter()
            .take(SWITCHER_PROJECTS)
            .enumerate()
            .map(|(ix, (choice, known))| {
                let active = choice == target
                    || (!choice.starts_with("group:")
                        && wks_native::projects::same_dir(&choice, &target));
                interactive_control(div().id(("scratch-project", ix)), p, !active)
                    .debug_selector(move || format!("scratch-project-{ix}"))
                    .pl_1()
                    .pr_2()
                    .py(px(2.))
                    .rounded(px(p.control_radius))
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(chrome::scale::CAPTION))
                    .when(active, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
                    .when(!active, |d| {
                        d.text_color(rgb(p.muted))
                            .hover(move |s| s.text_color(rgb(p.text)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pick_scratch_project(choice.clone(), cx)
                            }))
                    })
                    .child(self.project_mark(Some(&known), &known.path, 16.))
                    .child(known.title().to_owned())
            })
            .collect::<Vec<_>>();
        let single = listings
            .filter(|l| l.len() == 1)
            .and_then(|l| l.first())
            .map(|(_, l)| (l.root.clone(), l.files.clone()));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(self.project_mark(Some(&identity), &identity.path, 24.))
                    .child(
                        div()
                            .text_size(px(chrome::scale::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .debug_selector(|| "scratch-project-name".into())
                            .child(name),
                    )
                    .when_some(single.clone(), |d, (root, _)| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font())
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .debug_selector(|| "scratch-root".into())
                                .child(root),
                        )
                    })
                    .child(div().flex_1())
                    .when_some(single, |d, (_, files)| {
                        d.children(self.folder_actions(&files, cx))
                    }),
            )
            .when(chips.len() > 1, |d| {
                d.child(div().flex().flex_wrap().gap_1().children(chips))
            })
    }

    fn render_scratch_item(
        &self,
        ix: usize,
        row: &Row,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let item = row.item;
        let expanded = self
            .extras
            .scratch_open
            .as_ref()
            .is_some_and(|(cwd, path)| cwd == row.cwd && path == &item.path);
        let toggle = item.clone();
        let toggle_cwd = row.cwd.to_owned();
        let mut meta = scratch::size_label(item.size);
        if let Some(at) = item.modified_at {
            meta = format!("{meta} · {}", jobs::since(at, now_ms()));
        }
        chrome::card(p)
            .debug_selector(move || format!("scratch-row-{ix}"))
            .flex()
            .flex_col()
            .when(item.status == Status::Pending, |d| {
                d.border_color(rgb(p.warning))
            })
            .child(
                div()
                    .id(("scratch-row", ix))
                    .px_4()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .hover(move |s| s.bg(gpui::Hsla::from(rgb(p.selected)).opacity(0.5)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_scratch(&toggle_cwd, &toggle, window, cx)
                    }))
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(rgb(p.muted)),
                    )
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
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(mono_font())
                                            .text_size(px(chrome::scale::BODY))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(!item.status.open(), |d| {
                                                d.text_color(rgb(p.muted))
                                            })
                                            .child(item.path.clone()),
                                    )
                                    .child(self.scratch_status(item))
                                    .when(item.keep, |d| {
                                        d.child(tag(IconName::Heart, "kept", p.muted))
                                    })
                                    .when(item.sensitive, |d| {
                                        d.child(tag(IconName::EyeOff, "sensitive", p.warning))
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .when(item.purpose.is_none(), |d| d.italic())
                                    .child(
                                        item.purpose.clone().unwrap_or_else(|| UNRECORDED.into()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(meta),
                    ),
            )
            .when(expanded, |d| {
                d.child(self.scratch_detail(ix, row, window, cx))
            })
    }

    fn scratch_status(&self, item: &Item) -> Div {
        let p = self.appearance.palette();
        let color = match item.status {
            Status::Pending => p.warning,
            Status::Done => p.success,
            Status::Archived => p.disabled,
            Status::Draft | Status::Unrecorded => p.muted,
        };
        div()
            .flex_shrink_0()
            .text_size(px(chrome::scale::CAPTION))
            .text_color(rgb(color))
            .child(item.status.label())
    }

    /// Actions, the purpose/note editor, and the file's text.
    fn scratch_detail(
        &self,
        ix: usize,
        row: &Row,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let item = row.item;
        let cwd = row.cwd.to_owned();
        let enabled = self.view.connected && !self.demo && !self.scratch_busy();
        let absolute = row.listing.absolute(item);
        let mut actions = div().flex().flex_wrap().items_center().gap_1();
        for (status, wire, label) in STATUS_ACTIONS {
            if status == item.status
                || (status == Status::Draft && item.status == Status::Unrecorded)
            {
                continue;
            }
            let path = item.path.clone();
            let mark_cwd = cwd.clone();
            let button = if status == Status::Done && item.status == Status::Pending {
                self.primary_button(format!("scratch-mark-{wire}-{ix}"), label, enabled)
            } else {
                self.button(format!("scratch-mark-{wire}-{ix}"), label, enabled)
            };
            actions = actions.child(
                button
                    .debug_selector(move || format!("scratch-mark-{wire}-{ix}"))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(patch) = scratch::status_patch(status) {
                                this.mark_scratch(mark_cwd.clone(), path.clone(), patch, cx);
                            }
                        }))
                    }),
            );
        }
        let keep_path = item.path.clone();
        let keep_cwd = cwd.clone();
        let keep = !item.keep;
        actions = actions.child(
            self.button(
                format!("scratch-keep-{ix}"),
                if item.keep { "Stop keeping" } else { "Keep" },
                enabled,
            )
            .debug_selector(move || format!("scratch-keep-{ix}"))
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.mark_scratch(
                        keep_cwd.clone(),
                        keep_path.clone(),
                        serde_json::json!({"keep": keep}),
                        cx,
                    )
                }))
            }),
        );
        let copy = absolute.clone();
        actions = actions.child(
            self.quiet_button(
                format!("scratch-copy-{ix}"),
                "Copy path",
                IconName::Copy,
                true,
            )
            .debug_selector(move || format!("scratch-copy-{ix}"))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
            })),
        );
        if self.extras.local_paths {
            let open = absolute.clone();
            let reveal = absolute;
            actions = actions
                .child(
                    self.quiet_button(format!("scratch-open-{ix}"), "Open", IconName::File, true)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.open_with_system(std::path::Path::new(&open))
                        })),
                )
                .child(
                    self.quiet_button(
                        format!("scratch-reveal-{ix}"),
                        "Show in folder",
                        IconName::FolderOpen,
                        true,
                    )
                    .on_click(
                        cx.listener(move |_, _, _, cx| {
                            cx.reveal_path(std::path::Path::new(&reveal))
                        }),
                    ),
                );
        }
        let save_item = item.clone();
        let save_cwd = cwd.clone();
        let editor = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(super::projects::section_label("PURPOSE", p))
            .child(Input::new(&self.extras.scratch_purpose).disabled(!enabled))
            .child(super::projects::section_label("NOTE", p))
            .child(Input::new(&self.extras.scratch_note).disabled(!enabled))
            .child(
                div().flex().child(
                    self.button(
                        format!("scratch-save-{ix}"),
                        "Save purpose and note",
                        enabled,
                    )
                    .debug_selector(move || format!("scratch-save-{ix}"))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.save_scratch_text(save_cwd.clone(), &save_item, cx)
                        }))
                    }),
                ),
            );
        let mut facts = Vec::new();
        if let Some(session) = &item.session_id {
            facts.push(format!("saved by session {session}"));
        }
        if let (Some(at), true) = (item.status_at, item.status != Status::Unrecorded) {
            facts.push(format!(
                "{} {}",
                item.status.label(),
                jobs::since(at, now_ms())
            ));
        }
        div()
            .px_4()
            .pb_4()
            .flex()
            .flex_col()
            .gap_3()
            .border_t_1()
            .border_color(rgb(p.border))
            .pt_3()
            .child(actions)
            .when(!facts.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(facts.join(" · ")),
                )
            })
            .child(editor)
            .child(self.scratch_content(ix, row, window, cx))
    }

    /// The open file's text, read-only and highlighted by its extension.
    fn scratch_content(
        &self,
        ix: usize,
        row: &Row,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let item = row.item;
        let state = self.view.requests.get("scratch-read").filter(|s| {
            matches!(&s.request, Request::ScratchRead { cwd, path } if path == &item.path && cwd == row.cwd)
        });
        let Some(state) = state else {
            return div();
        };
        if state.loading || state.error.is_some() {
            return self.feature_message("scratch-read");
        }
        match scratch::parse_read(&state.value) {
            Some(Content::Withheld(reason)) => {
                chrome::notice_line(reason, chrome::Tone::Warning, p)
                    .debug_selector(|| "scratch-withheld".into())
            }
            Some(Content::Text { text, clipped }) => div()
                .flex()
                .flex_col()
                .gap_1()
                .debug_selector(|| "scratch-content".into())
                .when(clipped, |d| {
                    d.child(chrome::notice_line(
                        format!(
                            "Showing the first {} KB; the whole file is on disk.",
                            scratch::PREVIEW_BYTES / 1024
                        ),
                        chrome::Tone::Info,
                        p,
                    ))
                })
                .child(
                    TextView::markdown(
                        SharedString::from(format!("scratch-content-{ix}")),
                        tool_preview::fenced(tool_preview::language(&item.path), &text),
                        window,
                        cx,
                    )
                    .style(gpui_component::text::TextViewStyle {
                        highlight_theme: gpui_component::Theme::global(cx).highlight_theme.clone(),
                        is_dark: gpui_component::Theme::global(cx).mode.is_dark(),
                        ..Default::default()
                    })
                    .selectable(true),
                ),
            None => chrome::notice_line(
                "The hub returned no text for this file.",
                chrome::Tone::Error,
                p,
            ),
        }
    }
}

fn tag(icon: IconName, text: &'static str, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(chrome::scale::CAPTION))
        .text_color(rgb(color))
        .child(Icon::new(icon).size(px(11.)))
        .child(text)
}

fn empty_line(text: &'static str, p: Palette) -> Div {
    div()
        .py_6()
        .text_center()
        .text_size(px(chrome::scale::META))
        .text_color(rgb(p.muted))
        .child(text)
}
