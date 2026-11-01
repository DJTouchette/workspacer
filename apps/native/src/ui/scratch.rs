//! The Scratch page: a project's personal files kept outside git — test
//! harnesses, repros, SQL meant to be run by hand — shared by every worktree
//! and kept when worktrees are removed (`docs/features/scratch.md`).
//!
//! The page reads and marks. It never runs a file and never writes one's
//! contents: agents save files (`save_scratch` or straight into
//! `$WORKSPACER_SCRATCH`), and here the user sees what is waiting on them,
//! reads it, and records that it ran. The model lives in
//! `wks_native::scratch`.
use super::*;
use wks_native::features::Request;
use wks_native::scratch::{self, Content, Filter, Item, Listing, Status};
use wks_native::{jobs, tool_preview};

pub(super) const SCRATCH_DESCRIPTION: &str = "Personal files for this project, kept outside git and shared by every worktree: harnesses, repros and SQL meant to be run by hand.";
const NO_PROJECT: &str = "Open an agent or a project to see its scratch.";
const NO_SCRATCH: &str = "Nothing in this project's scratch yet. Agents keep repros, one-off scripts and harnesses here; ask one to save something.";
const NO_OPEN: &str = "Nothing open. Everything here is done or archived; switch to All to see it.";
const NO_PENDING: &str = "Nothing is waiting on you.";
const FOOTER: &str = "Workspacer never runs scratch files. Pending means a file is waiting for you to run it by hand; mark it done when you have.";
const UNRECORDED: &str = "No purpose recorded. An agent wrote this straight into the folder.";
/// Projects the switcher offers besides the one shown.
const SWITCHER_PROJECTS: usize = 8;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

const STATUS_ACTIONS: [(Status, &str, &str); 4] = [
    (Status::Pending, "pending", "Mark pending"),
    (Status::Done, "done", "Mark done"),
    (Status::Archived, "archived", "Archive"),
    (Status::Draft, "draft", "Back to draft"),
];

impl Workspace {
    /// Where the page looks when it opens: the filtered project, else the
    /// open conversation's folder, else the last project chosen, else the
    /// first project the hub knows. Fixed for
    /// as long as the page stays open, so selecting another session does not
    /// swap the list underneath the user.
    pub(super) fn open_scratch(&mut self, cx: &mut Context<Self>) {
        self.extras.scratch_open = None;
        self.extras.scratch_cwd = self
            .project_filter
            .clone()
            .or_else(|| {
                self.selected_session()
                    .map(|s| s.cwd.clone())
                    .filter(|cwd| !cwd.is_empty())
            })
            .or_else(|| Some(self.projects.cwd.clone()).filter(|cwd| !cwd.is_empty()))
            .or_else(|| self.known_projects().first().map(|p| p.path.clone()));
        self.refresh_scratch(cx);
    }

    pub(super) fn refresh_scratch(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.extras.scratch_cwd.clone() else {
            return;
        };
        self.request(Request::ScratchList { cwd: cwd.clone() }, cx);
        if let Some(path) = self.extras.scratch_open.clone() {
            self.request(Request::ScratchRead { cwd, path }, cx);
        }
    }

    fn pick_scratch_project(&mut self, path: String, cx: &mut Context<Self>) {
        self.extras.scratch_cwd = Some(path);
        self.extras.scratch_open = None;
        self.refresh_scratch(cx);
    }

    /// The newest list the hub returned, from a read or from a mark (which
    /// answers with the refreshed list).
    pub(super) fn scratch_listing(&self) -> Option<Listing> {
        ["scratch", "scratch-mark"]
            .iter()
            .filter_map(|key| self.view.requests.get(*key))
            .filter(|s| !s.loading && s.error.is_none() && s.value["items"].is_array())
            .max_by_key(|s| s.number)
            .and_then(|s| scratch::parse_list(&s.value))
    }

    fn scratch_busy(&self) -> bool {
        self.view
            .requests
            .get("scratch-mark")
            .is_some_and(|s| s.loading)
    }

    fn toggle_scratch(&mut self, item: &Item, window: &mut Window, cx: &mut Context<Self>) {
        if self.extras.scratch_open.as_deref() == Some(item.path.as_str()) {
            self.extras.scratch_open = None;
            cx.notify();
            return;
        }
        self.extras.scratch_open = Some(item.path.clone());
        let purpose = item.purpose.clone().unwrap_or_default();
        let note = item.note.clone().unwrap_or_default();
        self.extras
            .scratch_purpose
            .update(cx, |input, cx| input.set_value(purpose, window, cx));
        self.extras
            .scratch_note
            .update(cx, |input, cx| input.set_value(note, window, cx));
        if let Some(cwd) = self.extras.scratch_cwd.clone() {
            self.request(
                Request::ScratchRead {
                    cwd,
                    path: item.path.clone(),
                },
                cx,
            );
        }
        cx.notify();
    }

    fn mark_scratch(&mut self, path: String, patch: serde_json::Value, cx: &mut Context<Self>) {
        let Some(cwd) = self.extras.scratch_cwd.clone() else {
            return;
        };
        self.request(Request::ScratchMark { cwd, path, patch }, cx);
    }

    /// The purpose/note editor's Save: sends only what changed.
    fn save_scratch_text(&mut self, item: &Item, cx: &mut Context<Self>) {
        let purpose = self.extras.scratch_purpose.read(cx).value().to_string();
        let note = self.extras.scratch_note.read(cx).value().to_string();
        match scratch::text_patch(item, &purpose, &note) {
            Some(patch) => self.mark_scratch(item.path.clone(), patch, cx),
            None => {
                self.extras.notice = "Nothing changed.".into();
                cx.notify();
            }
        }
    }

    pub(super) fn render_scratch(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let page = div().flex().flex_col().gap_3();
        if self.extras.scratch_cwd.is_none() {
            return page.child(empty_line(NO_PROJECT, p));
        }
        let listing = self.scratch_listing();
        let loading = self.view.requests.get("scratch").is_some_and(|s| s.loading);
        let page = page
            .child(self.scratch_project_bar(listing.as_ref(), cx))
            .child(self.feature_message("scratch"))
            .child(self.feature_message("scratch-mark"));
        let Some(listing) = listing else {
            return page;
        };
        let filter = self.extras.scratch_filter;
        let open = listing.items.iter().filter(|i| i.status.open()).count();
        let pending = listing.pending();
        let page = page
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        self.segmented(
                            "scratch-filter",
                            vec![
                                (Filter::Open, format!("Open {open}")),
                                (Filter::Pending, format!("Pending {pending}")),
                                (Filter::All, format!("All {}", listing.items.len())),
                            ],
                            filter,
                            |this, filter, _, cx| {
                                this.extras.scratch_filter = filter;
                                cx.notify();
                            },
                            cx,
                        ),
                    )
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
            )
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
        let shown: Vec<(usize, &Item)> = listing
            .items
            .iter()
            .filter(|item| filter.admits(item))
            .enumerate()
            .collect();
        let empty = match filter {
            _ if listing.items.is_empty() => NO_SCRATCH,
            Filter::Pending => NO_PENDING,
            _ => NO_OPEN,
        };
        page.when(shown.is_empty() && !loading, |d| {
            d.child(empty_line(empty, p).debug_selector(|| "scratch-empty".into()))
        })
        .children(
            shown
                .iter()
                .map(|(ix, item)| self.render_scratch_item(*ix, item, &listing, window, cx)),
        )
        .child(
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(FOOTER),
        )
    }

    /// The project shown, where its files live, and the other projects the
    /// user can switch to.
    fn scratch_project_bar(&self, listing: Option<&Listing>, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let cwd = self.extras.scratch_cwd.clone().unwrap_or_default();
        let shown = |path: &str| {
            wks_native::projects::same_dir(path, &cwd)
                || listing.is_some_and(|l| wks_native::projects::same_dir(path, &l.root))
        };
        let known = self.known_projects();
        let chips = known
            .iter()
            .take(SWITCHER_PROJECTS)
            .enumerate()
            .map(|(ix, project)| {
                let active = shown(&project.path);
                let path = project.path.clone();
                div()
                    .id(("scratch-project", ix))
                    .debug_selector(move || format!("scratch-project-{ix}"))
                    .px_2()
                    .py(px(2.))
                    .rounded(px(p.control_radius))
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_size(px(chrome::scale::CAPTION))
                    .cursor_pointer()
                    .when(active, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
                    .when(!active, |d| d.text_color(rgb(p.muted)))
                    .child(project.title().to_owned())
                    .when(!active, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_scratch_project(path.clone(), cx)
                        }))
                    })
            })
            .collect::<Vec<_>>();
        let folder = listing.map(|l| l.files.clone()).filter(|f| !f.is_empty());
        let local = self.extras.local_paths;
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
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .debug_selector(|| "scratch-root".into())
                            .child(listing.map_or(cwd.clone(), |l| l.root.clone())),
                    )
                    .when_some(folder, |d, folder| {
                        let copy = folder.clone();
                        d.child(
                            self.quiet_button(
                                "scratch-copy-folder",
                                "Copy folder path",
                                IconName::Copy,
                                true,
                            )
                            .debug_selector(|| "scratch-copy-folder".into())
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                },
                            )),
                        )
                        .when(local, |d| {
                            d.child(
                                self.quiet_button(
                                    "scratch-reveal-folder",
                                    "Show folder",
                                    IconName::FolderOpen,
                                    true,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.open_with_system(std::path::Path::new(&folder))
                                    },
                                )),
                            )
                        })
                    }),
            )
            .when(chips.len() > 1, |d| {
                d.child(div().flex().flex_wrap().gap_1().children(chips))
            })
    }

    fn render_scratch_item(
        &self,
        ix: usize,
        item: &Item,
        listing: &Listing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let expanded = self.extras.scratch_open.as_deref() == Some(item.path.as_str());
        let toggle = item.clone();
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_scratch(&toggle, window, cx)
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
                                        d.child(tag(IconName::Star, "kept", p.muted))
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
                d.child(self.scratch_detail(ix, item, listing, window, cx))
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
        item: &Item,
        listing: &Listing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let enabled = self.view.connected && !self.demo && !self.scratch_busy();
        let absolute = listing.absolute(item);
        let mut actions = div().flex().flex_wrap().items_center().gap_1();
        for (status, wire, label) in STATUS_ACTIONS {
            if status == item.status
                || (status == Status::Draft && item.status == Status::Unrecorded)
            {
                continue;
            }
            let path = item.path.clone();
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
                                this.mark_scratch(path.clone(), patch, cx);
                            }
                        }))
                    }),
            );
        }
        let keep_path = item.path.clone();
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
                    this.mark_scratch(keep_path.clone(), serde_json::json!({"keep": keep}), cx)
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
        let editor =
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(field_label("Purpose", p))
                .child(Input::new(&self.extras.scratch_purpose).disabled(!enabled))
                .child(field_label("Note (for example, when and where it ran)", p))
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
                                this.save_scratch_text(&save_item, cx)
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
            .child(self.scratch_content(ix, item, window, cx))
    }

    /// The open file's text, read-only and highlighted by its extension.
    fn scratch_content(
        &self,
        ix: usize,
        item: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let state = self.view.requests.get("scratch-read").filter(
            |s| matches!(&s.request, Request::ScratchRead { path, .. } if path == &item.path),
        );
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

fn field_label(text: &'static str, p: Palette) -> Div {
    div()
        .text_size(px(chrome::scale::CAPTION))
        .text_color(rgb(p.muted))
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
