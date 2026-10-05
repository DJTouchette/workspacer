//! Review changes: a Git-style diff with the repository's files on the right.
//! The explorer shows either the changed files (status from `git.status`) or
//! the whole project tree; choosing a changed file shows its diff, choosing
//! any file in the tree opens it in the editor. Every read runs on the hub
//! (`git.status`, `git.diff`, `fs.listEntries`, `fs.read`), and paths are
//! joined under the repository root the hub reports, never outside it.
use super::*;
use gpui::{AnyElement, ListHorizontalSizingBehavior};
use std::collections::BTreeSet;
use wks_native::{
    diff::{self, Kind},
    features::Request,
    links,
};

const EXPLORER_WIDTH: f32 = 300.;
const ROW_HEIGHT: f32 = 20.;

#[derive(Default)]
pub(super) struct ReviewUi {
    /// The right-hand list shows every project file rather than the changes.
    pub all_files: bool,
    pub expanded: BTreeSet<String>,
    /// Repository-relative path whose diff is shown.
    pub selected: Option<String>,
    /// The `changes` state already used to pick a first file.
    seen: u64,
    parsed: Option<(u64, Arc<diff::Diff>, usize)>,
    pub scroll: gpui::UniformListScrollHandle,
}

/// One row of `git status`, with its paths resolved.
#[derive(Clone, Debug)]
struct Change {
    path: String,
    absolute: Option<String>,
    staged: String,
    unstaged: String,
    letter: char,
}

impl Change {
    fn untracked(&self) -> bool {
        self.letter == '?'
    }
    fn has_unstaged(&self) -> bool {
        self.untracked() || !self.unstaged.trim().is_empty()
    }
    fn has_staged(&self) -> bool {
        !self.untracked() && !self.staged.trim().is_empty()
    }
}

impl Workspace {
    /// The project being reviewed and the root its status paths hang from.
    fn review_paths(&self) -> Option<(String, String)> {
        let state = self.view.requests.get("changes");
        let cwd = state
            .and_then(|state| match &state.request {
                Request::Changes { cwd } => Some(cwd.clone()),
                _ => None,
            })
            .or_else(|| self.selected_session().map(|s| s.cwd.clone()))?;
        // Older hubs do not report the work-tree root; the cwd then is it.
        let root = state
            .and_then(|s| s.value["root"].as_str())
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| cwd.clone());
        Some((cwd, root))
    }

    fn review_changes(&self, root: &str) -> Vec<Change> {
        let value = self
            .view
            .requests
            .get("changes")
            .map(|s| s.value.clone())
            .unwrap_or_default();
        value["files"]
            .as_array()
            .into_iter()
            .flatten()
            .take(5000)
            .filter_map(|file| {
                let path = file["path"].as_str()?.to_owned();
                let staged = file["staged"].as_str().unwrap_or(" ").to_owned();
                let unstaged = file["unstaged"].as_str().unwrap_or(" ").to_owned();
                Some(Change {
                    absolute: links::join_within(root, &path),
                    letter: diff::status_letter(&staged, &unstaged),
                    path,
                    staged,
                    unstaged,
                })
            })
            .collect()
    }

    /// Show `change`'s diff: the working tree when it has one, else staged.
    fn show_diff(&mut self, change: &Change, staged: Option<bool>, cx: &mut Context<Self>) {
        let Some((cwd, _)) = self.review_paths() else {
            return;
        };
        let staged = staged.unwrap_or(!change.has_unstaged());
        self.review.selected = Some(change.path.clone());
        self.review
            .scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.request(
            Request::Diff {
                cwd,
                path: change.path.clone(),
                staged,
                untracked: change.untracked() && !staged,
            },
            cx,
        );
    }

    /// Open a project file in the editor, owned by the selected session.
    fn review_open_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let links::Link::File(target) = links::tool_file("", path, None) else {
            return;
        };
        let owner = self.view.selected.clone().unwrap_or_default();
        if let Err(message) = self.request_preview(&owner, target, cx) {
            self.extras.notice = message;
            cx.notify();
        }
    }

    /// When the change list arrives, show the first change's diff (or keep
    /// the one being read if it is still changed).
    pub(super) fn sync_review(&mut self, cx: &mut Context<Self>) {
        if self.screen != Screen::Changes {
            return;
        }
        let Some(state) = self.view.requests.get("changes") else {
            return;
        };
        if state.loading || state.error.is_some() || state.number <= self.review.seen {
            return;
        }
        self.review.seen = state.number;
        let Some((_, root)) = self.review_paths() else {
            return;
        };
        let changes = self.review_changes(&root);
        let keep = self
            .review
            .selected
            .as_ref()
            .and_then(|s| changes.iter().find(|c| &c.path == s))
            .cloned();
        match keep.or_else(|| changes.first().cloned()) {
            Some(change) => {
                let staged = self
                    .view
                    .requests
                    .get("diff")
                    .and_then(|d| match &d.request {
                        Request::Diff { path, staged, .. } if path == &change.path => Some(*staged),
                        _ => None,
                    });
                self.show_diff(&change, staged, cx)
            }
            None => self.review.selected = None,
        }
        if self.review.all_files {
            self.explore(&root, cx);
        }
    }

    pub(super) fn render_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let p = self.appearance.palette();
        let short = window.viewport_size().height < px(620.);
        let caption = chrome::custom_caption();
        let back = self
            .quiet_button("feature-back", "Back to chat", IconName::ArrowLeft, true)
            .debug_selector(|| "feature-back".into())
            .on_click(cx.listener(|this, _, window, cx| this.back_from_feature(window, cx)));
        let paths = self.review_paths();
        let state = self.view.requests.get("changes").cloned();
        let branch = state
            .as_ref()
            .and_then(|s| s.value["branch"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Repository".into());
        let trailing = paths.clone().map(|(cwd, root)| {
            let session = self.view.selected.clone().unwrap_or_default();
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    self.quiet_button("review-editor", "Open editor", IconName::FolderOpen, true)
                        .debug_selector(|| "review-editor".into())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_editor(&session, &root, window, cx)
                        })),
                )
                .child(
                    self.quiet_button(
                        "changes-refresh",
                        "Refresh",
                        IconName::Redo,
                        self.view.connected,
                    )
                    .when(self.view.connected, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.request(Request::Changes { cwd: cwd.clone() }, cx);
                            if let Some((_, root)) = this.review_paths()
                                && this.review.all_files
                            {
                                this.refresh_explorer(&root, cx);
                            }
                        }))
                    }),
                )
                .into_any_element()
        });
        let description = paths
            .as_ref()
            .map(|(_, root)| SharedString::from(format!("{root} · {branch}")));
        let header = self.page_header(
            Some(back),
            Some("REVIEW"),
            "Changes",
            description,
            trailing,
            short,
        );
        let notice = (!self.extras.notice.is_empty()).then(|| {
            let tone = chrome::notice_tone(&self.extras.notice);
            chrome::notice_line(self.extras.notice.clone(), tone, p, "feature-notice")
                .debug_selector(|| "feature-notice".into())
        });
        let body: AnyElement = match paths {
            None => div()
                .p_6()
                .text_color(rgb(p.muted))
                .child("Select a session or request a project review first.")
                .into_any_element(),
            Some((_, root)) => {
                let changes = self.review_changes(&root);
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .child(self.render_diff_panel(&changes, &root, cx))
                    .child(self.render_review_files(&changes, &root, cx))
                    .into_any_element()
            }
        };
        div()
            .id("review-view")
            .debug_selector(|| "review-view".into())
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(p.chat))
            .flex()
            .flex_col()
            .px(px(if short { 12. } else { 20. }))
            .pt(px(if caption {
                chrome::PAGE_CAPTION_INSET
            } else if short {
                12.
            } else {
                20.
            }))
            .pb(px(if short { 12. } else { 20. }))
            .gap_3()
            .children(chrome::page_drag_strip())
            .child(header)
            .children(notice)
            .child(self.feature_message("changes"))
            .child(body)
    }

    fn render_diff_panel(&mut self, changes: &[Change], root: &str, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let panel = chrome::card(p)
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col();
        let loaded = self
            .view
            .requests
            .get("changes")
            .is_some_and(|s| !s.loading && s.error.is_none());
        let Some(selected) = self.review.selected.clone() else {
            return panel.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child(if loaded && changes.is_empty() {
                        "No uncommitted changes. Switch to All files to browse and edit the project."
                    } else {
                        "Choose a changed file on the right to see its diff."
                    }),
            );
        };
        let change = changes.iter().find(|c| c.path == selected).cloned();
        let state = self
            .view
            .requests
            .get("diff")
            .filter(|s| matches!(&s.request, Request::Diff { path, .. } if path == &selected))
            .cloned();
        let staged_view = matches!(
            state.as_ref().map(|s| &s.request),
            Some(Request::Diff { staged: true, .. })
        );
        let parsed = state
            .as_ref()
            .filter(|s| !s.loading && s.error.is_none())
            .map(|s| match &self.review.parsed {
                Some((number, parsed, widest)) if *number == s.number => (parsed.clone(), *widest),
                _ => {
                    let parsed = Arc::new(diff::parse(s.value["diff"].as_str().unwrap_or("")));
                    let widest = parsed
                        .rows
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, r)| r.text.len())
                        .map_or(0, |(ix, _)| ix);
                    self.review.parsed = Some((s.number, parsed.clone(), widest));
                    (parsed, widest)
                }
            });
        let (name, dir) = match selected.rsplit_once(['/', '\\']) {
            Some((dir, name)) => (name.to_owned(), format!("{dir}/")),
            None => (selected.clone(), String::new()),
        };
        let side = |id: &'static str, label: &'static str, active: bool, enabled: bool| {
            chrome::interactive_control(div().id(id), p, enabled)
                .debug_selector(move || id.into())
                .px_2()
                .py(px(2.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .when(active, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
                .when(!active, |d| {
                    d.text_color(rgb(if enabled { p.muted } else { p.disabled }))
                })
                .child(label)
        };
        let open_target = change
            .as_ref()
            .filter(|c| c.letter != 'D')
            .and_then(|c| c.absolute.clone());
        let mut header = div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.code_header))
            .when_some(change.as_ref(), |d, c| {
                d.child(
                    div()
                        .font_family(mono_font())
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(explorer::status_color(c.letter, p)))
                        .child(c.letter.to_string()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .overflow_hidden()
                    .child(
                        div()
                            .debug_selector(|| "review-diff-title".into())
                            .flex_shrink_0()
                            .text_size(px(chrome::scale::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(dir),
                    ),
            )
            .when_some(parsed.as_ref(), |d, (parsed, _)| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .gap_1()
                        .font_family(mono_font())
                        .text_size(px(chrome::scale::CAPTION))
                        .child(
                            div()
                                .text_color(rgb(p.success))
                                .child(format!("+{}", parsed.added)),
                        )
                        .child(
                            div()
                                .text_color(rgb(p.error))
                                .child(format!("−{}", parsed.removed)),
                        ),
                )
            });
        if let Some(change) = change.clone()
            && change.has_unstaged()
            && change.has_staged()
        {
            let (a, b) = (change.clone(), change.clone());
            header = header.child(
                div()
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(p.control_radius + 2.))
                    .bg(rgb(p.surface))
                    .child(
                        side("review-side-unstaged", "Unstaged", !staged_view, true).on_click(
                            cx.listener(move |this, _, _, cx| this.show_diff(&a, Some(false), cx)),
                        ),
                    )
                    .child(
                        side("review-side-staged", "Staged", staged_view, true).on_click(
                            cx.listener(move |this, _, _, cx| this.show_diff(&b, Some(true), cx)),
                        ),
                    ),
            );
        } else if change.as_ref().is_some_and(Change::has_staged) {
            header = header.child(side("review-side-staged", "Staged", true, true));
        }
        header = header.child(
            self.quiet_button(
                "review-open-file",
                "Open in editor",
                IconName::File,
                open_target.is_some(),
            )
            .debug_selector(|| "review-open-file".into())
            .when_some(open_target, |d, target| {
                d.on_click(cx.listener(move |this, _, _, cx| this.review_open_file(&target, cx)))
            }),
        );
        let _ = root;
        let panel = panel.child(header);
        let Some(state) = state else {
            return panel.child(
                div()
                    .p_4()
                    .text_color(rgb(p.muted))
                    .child("Loading the diff…"),
            );
        };
        if state.loading || state.error.is_some() {
            return panel.child(div().px_4().py_2().child(self.feature_message("diff")));
        }
        let Some((parsed, widest)) = parsed else {
            return panel;
        };
        if parsed.rows.is_empty() || parsed.binary {
            return panel.child(
                div()
                    .p_4()
                    .text_size(px(chrome::scale::META))
                    .text_color(rgb(p.muted))
                    .child(if parsed.binary {
                        "Binary file — no text diff to show."
                    } else {
                        "No text diff. The file may be binary, a mode change only, or changed since the last refresh."
                    }),
            );
        }
        let count = parsed.rows.len();
        let rows = parsed.clone();
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        panel
            .child(
                uniform_list(
                    "review-diff",
                    count,
                    cx.processor(move |_this, range: std::ops::Range<usize>, _, _| {
                        range
                            .map(|ix| diff_row(&rows.rows[ix], ix, p))
                            .collect::<Vec<_>>()
                    }),
                )
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(widest))
                .track_scroll(self.review.scroll.clone())
                .debug_selector(|| "review-diff".into())
                .flex_1()
                .min_h_0()
                .bg(rgb(p.code_block))
                .font_family(mono)
                .text_size(px(chrome::scale::META)),
            )
            .when(parsed.truncated, |d| {
                d.child(div().px_4().py_2().child(chrome::notice_line(
                    format!(
                        "Showing the first {} diff lines. Open the file to see all of it.",
                        diff::MAX_ROWS
                    ),
                    chrome::Tone::Info,
                    p,
                    "diff-cap",
                )))
            })
    }

    fn render_review_files(
        &mut self,
        changes: &[Change],
        root: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let all = self.review.all_files;
        let segment = |id: &'static str, label: String, active: bool| {
            chrome::interactive_control(div().id(id), p, true)
                .debug_selector(move || id.into())
                .flex_1()
                .flex()
                .justify_center()
                .px_2()
                .py(px(3.))
                .rounded(px(p.control_radius))
                .text_size(px(chrome::scale::CAPTION))
                .when(active, |d| {
                    d.bg(rgb(p.surface))
                        .text_color(rgb(p.text))
                        .shadow(chrome::floating_shadow(p))
                })
                .when(!active, |d| d.text_color(rgb(p.muted)))
                .child(label)
        };
        let root_for_all = root.to_owned();
        let toggle = div()
            .m_2()
            .p(px(2.))
            .flex()
            .gap(px(2.))
            .rounded(px(p.control_radius + 2.))
            .bg(rgb(p.selected))
            .child(
                segment(
                    "review-mode-changed",
                    format!("Changed ({})", changes.len()),
                    !all,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.review.all_files = false;
                    cx.notify();
                })),
            )
            .child(
                segment(
                    "review-mode-all",
                    if self.explorer.show_ignored {
                        "All files".to_owned()
                    } else {
                        "Files".to_owned()
                    },
                    all,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.review.all_files = true;
                    this.explore(&root_for_all, cx);
                    cx.notify();
                })),
            );
        let list: AnyElement = if all {
            let tree = self.explorer.tree(root, &self.review.expanded);
            let mut marks: explorer::Marks = HashMap::new();
            for change in changes {
                let Some(absolute) = &change.absolute else {
                    continue;
                };
                let color = explorer::status_color(change.letter, p);
                marks.insert(absolute.clone(), (change.letter, color));
                let mut dir = links::parent(absolute);
                while links::within(root, &dir)
                    && dir.len() > root.trim_end_matches(['/', '\\']).len()
                {
                    marks.entry(dir.clone()).or_insert(('•', p.warning));
                    let parent = links::parent(&dir);
                    if parent == dir {
                        break;
                    }
                    dir = parent;
                }
            }
            let view = cx.entity().downgrade();
            let on_row: explorer::OnRow = std::rc::Rc::new(move |row, _, cx| {
                let row = row.clone();
                let _ = view.update(cx, |this, cx| {
                    if row.entry.dir {
                        if !this.review.expanded.remove(&row.entry.path) {
                            this.review.expanded.insert(row.entry.path.clone());
                            this.explore(&row.entry.path, cx);
                        }
                        cx.notify();
                    } else {
                        this.review_open_file(&row.entry.path, cx);
                    }
                });
            });
            let show_ignored = self.explorer.show_ignored;
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .pb_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child("Choose a file to edit it."),
                        )
                        .child(
                            self.button_style(
                                "review-files-ignored",
                                if show_ignored {
                                    "Hide ignored"
                                } else {
                                    "Show ignored"
                                },
                                true,
                                false,
                            )
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .debug_selector(|| "review-files-ignored".into())
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.set_show_ignored(!show_ignored, cx),
                            )),
                        ),
                )
                .child(explorer::render_tree(
                    "review-tree",
                    &tree,
                    None,
                    &marks,
                    p,
                    on_row,
                ))
                .into_any_element()
        } else if changes.is_empty() {
            div()
                .px_3()
                .py_2()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(
                    if self.view.requests.get("changes").is_some_and(|s| s.loading) {
                        "Reading changes…"
                    } else {
                        "No uncommitted changes."
                    },
                )
                .into_any_element()
        } else {
            let selected = self.review.selected.clone();
            div()
                .flex()
                .flex_col()
                .py_1()
                .children(changes.iter().enumerate().map(|(ix, change)| {
                    let active = selected.as_ref() == Some(&change.path);
                    let (name, dir) = match change.path.rsplit_once(['/', '\\']) {
                        Some((dir, name)) => (name.to_owned(), dir.to_owned()),
                        None => (change.path.clone(), String::new()),
                    };
                    let color = explorer::status_color(change.letter, p);
                    let clicked = change.clone();
                    div()
                        .id(("review-change", ix))
                        .debug_selector({
                            let name = format!("review-change-{}", change.path);
                            move || name
                        })
                        .px_3()
                        .py(px(5.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .when(active, |d| d.bg(rgb(p.selected)))
                        .hover(|s| s.bg(rgb(p.selected)))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.show_diff(&clicked, None, cx)),
                        )
                        .child(
                            Icon::new(IconName::File)
                                .size(px(13.))
                                .flex_shrink_0()
                                .text_color(rgb(if active { p.accent } else { p.muted })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(chrome::scale::META))
                                        .child(name),
                                )
                                .when(!dir.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .truncate()
                                            .font_family(mono_font())
                                            .text_size(px(10.))
                                            .text_color(rgb(p.muted))
                                            .child(dir),
                                    )
                                }),
                        )
                        .when(change.has_staged() && change.has_unstaged(), |d| {
                            d.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child("staged + unstaged"),
                            )
                        })
                        .when(change.has_staged() && !change.has_unstaged(), |d| {
                            d.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.accent))
                                    .child("staged"),
                            )
                        })
                        .child(
                            div()
                                .w(px(12.))
                                .flex_shrink_0()
                                .font_family(mono_font())
                                .text_size(px(11.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(color))
                                .child(change.letter.to_string()),
                        )
                }))
                .into_any_element()
        };
        chrome::card(p)
            .debug_selector(|| "review-files".into())
            .w(px(EXPLORER_WIDTH))
            .flex_shrink_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(toggle)
            .child(
                div()
                    .id("review-files-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
    }
}

/// One numbered diff line: old and new line numbers, marker, text.
fn diff_row(row: &diff::Row, ix: usize, p: Palette) -> gpui::Stateful<Div> {
    let (bg, fg, marker) = match row.kind {
        Kind::Added => (
            Some(gpui::Hsla::from(rgb(p.success)).opacity(0.14)),
            p.prose,
            "+",
        ),
        Kind::Removed => (
            Some(gpui::Hsla::from(rgb(p.error)).opacity(0.14)),
            p.prose,
            "−",
        ),
        Kind::Hunk => (
            Some(gpui::Hsla::from(rgb(p.accent)).opacity(0.10)),
            p.accent,
            "",
        ),
        Kind::Meta | Kind::File => (None, p.muted, ""),
        Kind::Note => (None, p.muted, ""),
        Kind::Context => (None, p.prose, ""),
    };
    let number = |n: Option<u32>| {
        div()
            .w(px(44.))
            .flex_shrink_0()
            .pr_2()
            .flex()
            .justify_end()
            .text_color(rgb(p.disabled))
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    div()
        .id(("diff-row", ix))
        // Tints span the row, not just its text.
        .w_full()
        .min_w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .whitespace_nowrap()
        .when_some(bg, |d, bg| d.bg(bg))
        .child(number(row.old))
        .child(number(row.new))
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(rgb(match row.kind {
                    Kind::Added => p.success,
                    Kind::Removed => p.error,
                    _ => p.muted,
                }))
                .child(marker),
        )
        .child(
            div()
                .pr_4()
                .text_color(rgb(fg))
                .child(if row.text.is_empty() {
                    " ".to_owned()
                } else {
                    row.text.clone()
                }),
        )
}
