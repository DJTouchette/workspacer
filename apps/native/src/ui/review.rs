//! Review changes: a Git-style diff with the repository on the right.
//! The right-hand panel, which can be hidden, shows the changed files (status
//! from `git.status`), the whole project tree, or recent commits. Changed
//! files are staged, unstaged, committed and pushed from it; choosing one
//! shows its diff, choosing a commit shows its patch, and choosing any file in
//! the tree opens it in the editor. Every read and write runs on the hub
//! (`git.*`, `fs.*`), and paths are joined under the repository root the hub
//! reports, never outside it.
use super::*;
use gpui::{AnyElement, KeyDownEvent, ListHorizontalSizingBehavior};
use gpui_component::Sizable;
use std::collections::BTreeSet;
use wks_native::{
    diff::{self, Kind},
    features::{GitAction, Request, RequestState},
    links,
};

const EXPLORER_WIDTH: f32 = 300.;
const ROW_HEIGHT: f32 = 20.;
const CHANGE_ROW_HEIGHT: f32 = 36.;
const COMMIT_ROW_HEIGHT: f32 = 40.;

/// What the right-hand panel lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Changed,
    Files,
    Commits,
}

#[derive(Default)]
pub(super) struct ReviewUi {
    pub mode: Mode,
    pub expanded: BTreeSet<String>,
    /// Repository-relative path whose diff is shown.
    pub selected: Option<String>,
    /// Hash of the commit whose patch is shown under Commits.
    pub commit: Option<String>,
    /// The `changes` state already used to pick a first file.
    seen: u64,
    /// The last `git-action` answer acted on.
    action_seen: u64,
    /// The last status the hub answered, parsed once. A refresh keeps showing
    /// it (for the same folder) until the new answer lands, so the list never
    /// blanks and nothing is re-parsed per frame.
    status: Option<Status>,
    parsed: Option<(u64, Arc<diff::Diff>, usize)>,
    pub scroll: gpui::UniformListScrollHandle,
    list_scroll: gpui::UniformListScrollHandle,
}

struct Status {
    cwd: String,
    root: String,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u64,
    behind: u64,
    changes: Arc<Vec<Change>>,
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

fn parse_changes(value: &serde_json::Value, root: &str) -> Vec<Change> {
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
            .or_else(|| self.review_status_for(&cwd).map(|s| s.root.clone()))
            .unwrap_or_else(|| cwd.clone());
        Some((cwd, root))
    }

    fn review_status_for(&self, cwd: &str) -> Option<&Status> {
        self.review.status.as_ref().filter(|s| s.cwd == cwd)
    }

    fn review_status(&self) -> Option<&Status> {
        let (cwd, _) = self.review_paths()?;
        self.review_status_for(&cwd)
    }

    fn review_changes(&self) -> Arc<Vec<Change>> {
        self.review_status()
            .map(|s| s.changes.clone())
            .unwrap_or_default()
    }

    fn git_busy(&self) -> bool {
        self.view
            .requests
            .get("git-action")
            .is_some_and(|s| s.loading)
    }

    fn git_action(&mut self, action: GitAction, cx: &mut Context<Self>) {
        if self.git_busy() {
            return;
        }
        self.request(Request::Git(action), cx);
    }

    fn commit_staged(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((cwd, _)) = self.review_paths() else {
            return;
        };
        let message = self
            .extras
            .commit_message
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if message.is_empty() {
            self.extras.notice = "Write a commit message first.".into();
            self.extras
                .commit_message
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        self.git_action(GitAction::Commit { cwd, message }, cx);
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

    fn show_commit(&mut self, hash: &str, cx: &mut Context<Self>) {
        let Some((cwd, _)) = self.review_paths() else {
            return;
        };
        self.review.commit = Some(hash.to_owned());
        self.review
            .scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.request(
            Request::CommitDiff {
                cwd,
                hash: hash.to_owned(),
            },
            cx,
        );
    }

    fn set_review_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.review.mode == mode {
            return;
        }
        let was_commits = self.review.mode == Mode::Commits;
        self.review.mode = mode;
        let Some((cwd, root)) = self.review_paths() else {
            cx.notify();
            return;
        };
        match mode {
            Mode::Files => self.explore(&root, cx),
            Mode::Commits => {
                self.request(Request::Log { cwd }, cx);
                if let Some(hash) = self.review.commit.clone() {
                    self.show_commit(&hash, cx);
                }
            }
            Mode::Changed => {}
        }
        // The diff slot held a commit's patch; put the change back.
        if was_commits {
            let changes = self.review_changes();
            let keep = self
                .review
                .selected
                .as_ref()
                .and_then(|s| changes.iter().find(|c| &c.path == s))
                .or_else(|| changes.first())
                .cloned();
            match keep {
                Some(change) => self.show_diff(&change, None, cx),
                None => self.review.selected = None,
            }
        }
        cx.notify();
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

    /// A git write finished: the status is always re-read (even a failed
    /// write may have changed the index), a landed commit leaves the message
    /// box, and an open commit list picks up new or pushed commits.
    fn finish_git_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.view.requests.get("git-action") else {
            return;
        };
        if state.loading || state.number <= self.review.action_seen {
            return;
        }
        self.review.action_seen = state.number;
        let Request::Git(action) = state.request.clone() else {
            return;
        };
        let ok = state.error.is_none();
        let cwd = action.cwd().to_owned();
        if ok
            && matches!(action, GitAction::Commit { .. } | GitAction::Push { .. })
            && self.review.mode == Mode::Commits
        {
            self.request(Request::Log { cwd: cwd.clone() }, cx);
        }
        self.request(Request::Changes { cwd }, cx);
        if !ok {
            return;
        }
        match &action {
            GitAction::Commit { message, .. } => {
                if self.extras.commit_message.read(cx).value().trim() == message {
                    self.extras
                        .commit_message
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.extras.notice = "Committed.".into();
            }
            GitAction::Push { .. } => self.extras.notice = "Pushed.".into(),
            _ => {}
        }
    }

    /// Review open on a session whose turn just ended: the agent may have
    /// changed files, so the status (and an open commit list) is re-read
    /// instead of showing what was true before it worked.
    pub(super) fn refresh_review_after_turn(&mut self, next: &View, cx: &mut Context<Self>) {
        if self.screen != Screen::Changes {
            return;
        }
        let Some(id) = next
            .selected
            .as_ref()
            .filter(|id| self.view.selected.as_ref() == Some(*id))
        else {
            return;
        };
        let was = self.view.sessions.iter().find(|s| &s.id == id);
        let now = next.sessions.iter().find(|s| &s.id == id);
        if !was.is_some_and(|s| s.working()) || now.is_none_or(|s| s.working()) {
            return;
        }
        let Some((cwd, _)) = self.review_paths() else {
            return;
        };
        if self.review.mode == Mode::Commits {
            self.request(Request::Log { cwd: cwd.clone() }, cx);
        }
        self.request(Request::Changes { cwd }, cx);
    }

    /// When the change list arrives, show the first change's diff (or keep
    /// the one being read if it is still changed).
    pub(super) fn sync_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Changes {
            return;
        }
        self.finish_git_action(window, cx);
        let Some(state) = self.view.requests.get("changes") else {
            return;
        };
        if state.loading || state.error.is_some() || state.number <= self.review.seen {
            return;
        }
        self.review.seen = state.number;
        let Request::Changes { cwd } = &state.request else {
            return;
        };
        let value = state.value.clone();
        let root = value["root"]
            .as_str()
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| cwd.clone());
        let changes = Arc::new(parse_changes(&value, &root));
        self.review.status = Some(Status {
            cwd: cwd.clone(),
            root: root.clone(),
            branch: value["branch"].as_str().map(str::to_owned),
            upstream: value["upstream"].as_str().map(str::to_owned),
            ahead: value["ahead"].as_u64().unwrap_or(0),
            behind: value["behind"].as_u64().unwrap_or(0),
            changes: changes.clone(),
        });
        let keep = self
            .review
            .selected
            .as_ref()
            .and_then(|s| changes.iter().find(|c| &c.path == s))
            .cloned();
        if self.review.mode == Mode::Commits {
            // The diff slot shows a commit; only remember which change is next.
            self.review.selected = keep.or_else(|| changes.first().cloned()).map(|c| c.path);
            return;
        }
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
        if self.review.mode == Mode::Files {
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
        let branch = self
            .review_status()
            .and_then(|s| s.branch.clone())
            .unwrap_or_else(|| "Repository".into());
        let panel_open = self.settings.review_panel;
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
                        "review-panel",
                        if panel_open {
                            "Hide files"
                        } else {
                            "Show files"
                        },
                        if panel_open {
                            IconName::PanelRightClose
                        } else {
                            IconName::PanelRightOpen
                        },
                        true,
                    )
                    .debug_selector(|| "review-panel".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.review_panel = !panel_open;
                        this.save_settings(cx);
                        cx.notify();
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
                            if this.review.mode == Mode::Commits {
                                this.request(Request::Log { cwd: cwd.clone() }, cx);
                            }
                            if let Some((_, root)) = this.review_paths()
                                && this.review.mode == Mode::Files
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
                let changes = self.review_changes();
                let diff = if self.review.mode == Mode::Commits {
                    self.render_commit_panel(cx)
                } else {
                    self.render_diff_panel(&changes, cx)
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .child(diff)
                    .when(panel_open, |d| {
                        d.child(self.render_review_files(&changes, &root, window, cx))
                    })
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
            .child(self.feature_message("git-action"))
            .child(body)
    }

    fn diff_panel_shell(&self) -> Div {
        let p = self.appearance.palette();
        chrome::card(p)
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col()
    }

    fn diff_placeholder(&self, text: &'static str) -> Div {
        let p = self.appearance.palette();
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .text_size(px(chrome::scale::META))
            .text_color(rgb(p.muted))
            .child(text)
    }

    /// The parsed rows of a loaded diff, parsed once per answer.
    fn parsed_diff(&mut self, state: Option<&RequestState>) -> Option<(Arc<diff::Diff>, usize)> {
        let s = state.filter(|s| !s.loading && s.error.is_none())?;
        Some(match &self.review.parsed {
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
        })
    }

    fn diff_header(&self) -> Div {
        let p = self.appearance.palette();
        div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.code_header))
    }

    fn diff_counts(parsed: &diff::Diff, p: Palette) -> Div {
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
            )
    }

    /// The diff rows under `panel`, or the state that stands in for them.
    fn diff_body(
        &self,
        panel: Div,
        state: Option<&RequestState>,
        parsed: Option<(Arc<diff::Diff>, usize)>,
        open_hint: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
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
                        "Showing the first {} diff lines. {open_hint}",
                        diff::MAX_ROWS
                    ),
                    chrome::Tone::Info,
                    p,
                    "diff-cap",
                )))
            })
    }

    fn render_diff_panel(&mut self, changes: &[Change], cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let panel = self.diff_panel_shell();
        let loaded = self.review_status().is_some();
        let Some(selected) = self.review.selected.clone() else {
            return panel.child(self.diff_placeholder(if loaded && changes.is_empty() {
                "No uncommitted changes. Switch to Files to browse and edit the project, or Commits to read what landed."
            } else {
                "Choose a changed file on the right to see its diff."
            }));
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
        let parsed = self.parsed_diff(state.as_ref());
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
        let mut header = self
            .diff_header()
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
                d.child(Self::diff_counts(parsed, p))
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
        self.diff_body(
            panel.child(header),
            state.as_ref(),
            parsed,
            "Open the file to see all of it.",
            cx,
        )
    }

    fn commit_rows(&self) -> Vec<(String, String, i64)> {
        self.view
            .requests
            .get("git-log")
            .filter(|s| !s.loading && s.error.is_none())
            .and_then(|s| s.value["commits"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|c| {
                Some((
                    c["hash"].as_str()?.to_owned(),
                    c["subject"].as_str().unwrap_or("").to_owned(),
                    c["authoredAt"].as_i64().unwrap_or(0) * 1000,
                ))
            })
            .collect()
    }

    fn render_commit_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let panel = self.diff_panel_shell();
        let Some(hash) = self.review.commit.clone() else {
            return panel
                .child(self.diff_placeholder("Choose a commit on the right to see its changes."));
        };
        let subject = self
            .commit_rows()
            .into_iter()
            .find(|(h, ..)| h == &hash)
            .map(|(_, subject, _)| subject)
            .unwrap_or_default();
        let state = self
            .view
            .requests
            .get("diff")
            .filter(|s| matches!(&s.request, Request::CommitDiff { hash: h, .. } if h == &hash))
            .cloned();
        let parsed = self.parsed_diff(state.as_ref());
        let header = self
            .diff_header()
            .child(
                div()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.accent))
                    .child(hash.clone()),
            )
            .child(
                div()
                    .debug_selector(|| "review-commit-title".into())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(subject),
            )
            .when_some(parsed.as_ref(), |d, (parsed, _)| {
                d.child(Self::diff_counts(parsed, p))
            });
        self.diff_body(
            panel.child(header),
            state.as_ref(),
            parsed,
            "The rest of this commit is not shown.",
            cx,
        )
    }

    fn render_review_files(
        &mut self,
        changes: &Arc<Vec<Change>>,
        root: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let mode = self.review.mode;
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
                    mode == Mode::Changed,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_review_mode(Mode::Changed, cx))),
            )
            .child(
                segment(
                    "review-mode-all",
                    if self.explorer.show_ignored {
                        "All files".to_owned()
                    } else {
                        "Files".to_owned()
                    },
                    mode == Mode::Files,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_review_mode(Mode::Files, cx))),
            )
            .child(
                segment(
                    "review-mode-commits",
                    "Commits".to_owned(),
                    mode == Mode::Commits,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_review_mode(Mode::Commits, cx))),
            );
        let card = chrome::card(p)
            .debug_selector(|| "review-files".into())
            .w(px(EXPLORER_WIDTH))
            .flex_shrink_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(toggle);
        match mode {
            Mode::Files => card.child(
                div()
                    .id("review-files-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_review_tree(changes, root, cx)),
            ),
            Mode::Commits => card.child(self.render_commit_list(cx)),
            Mode::Changed => self.render_change_list(card, changes, root, window, cx),
        }
    }

    fn render_review_tree(
        &mut self,
        changes: &[Change],
        root: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.appearance.palette();
        let tree = self.explorer.tree(root, &self.review.expanded);
        let mut marks: explorer::Marks = HashMap::new();
        for change in changes {
            let Some(absolute) = &change.absolute else {
                continue;
            };
            let color = explorer::status_color(change.letter, p);
            marks.insert(absolute.clone(), (change.letter, color));
            let mut dir = links::parent(absolute);
            while links::within(root, &dir) && dir.len() > root.trim_end_matches(['/', '\\']).len()
            {
                // A folder already marked had its ancestors marked too.
                if marks.contains_key(&dir) {
                    break;
                }
                marks.insert(dir.clone(), ('•', p.warning));
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_show_ignored(!show_ignored, cx)
                        })),
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
    }

    fn small_action(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        enabled: bool,
    ) -> Stateful<Div> {
        self.button_style(id, label, enabled, false)
            .flex_shrink_0()
            .px_2()
            .py_1()
    }

    /// The changed files (virtualized, so a huge status stays cheap), with
    /// stage controls above and the commit box below.
    fn render_change_list(
        &mut self,
        card: Div,
        changes: &Arc<Vec<Change>>,
        root: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let busy = self.git_busy() || !self.view.connected;
        let staged = changes.iter().filter(|c| c.has_staged()).count();
        let unstaged = changes.iter().filter(|c| c.has_unstaged()).count();
        let status = self.review_status();
        let (ahead, behind) = status.map_or((0, 0), |s| (s.ahead, s.behind));
        let unpublished = status.is_some_and(|s| s.upstream.is_none() && s.branch.is_some());
        let cwd = self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default();
        let loading = self.view.requests.get("changes").is_some_and(|s| s.loading);
        let list: AnyElement = if changes.is_empty() {
            div()
                .flex_1()
                .px_3()
                .py_2()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(if loading && status.is_none() {
                    "Reading changes…"
                } else {
                    "No uncommitted changes."
                })
                .into_any_element()
        } else {
            let rows = changes.clone();
            uniform_list(
                "review-changes",
                rows.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|ix| this.change_row(ix, &rows[ix], busy, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(self.review.list_scroll.clone())
            .debug_selector(|| "review-changes".into())
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        let (stage_root, unstage_root) = (root.to_owned(), root.to_owned());
        let (stage_cwd, unstage_cwd) = (cwd.clone(), cwd.clone());
        let bulk = div()
            .px_3()
            .pb_1()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(chrome::scale::CAPTION))
            .text_color(rgb(p.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("{staged} staged")),
            )
            .child(
                self.small_action("review-stage-all", "Stage all", !busy && unstaged > 0)
                    .debug_selector(|| "review-stage-all".into())
                    .when(!busy && unstaged > 0, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.git_action(
                                GitAction::Stage {
                                    cwd: stage_cwd.clone(),
                                    path: Some(stage_root.clone()),
                                },
                                cx,
                            )
                        }))
                    }),
            )
            .child(
                self.small_action("review-unstage-all", "Unstage all", !busy && staged > 0)
                    .debug_selector(|| "review-unstage-all".into())
                    .when(!busy && staged > 0, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.git_action(
                                GitAction::Unstage {
                                    cwd: unstage_cwd.clone(),
                                    path: Some(unstage_root.clone()),
                                },
                                cx,
                            )
                        }))
                    }),
            );
        let can_commit = !busy && staged > 0;
        let can_push = !busy && (ahead > 0 || unpublished);
        // The first push of an untracked branch publishes and tracks it.
        let push_label: SharedString = if unpublished {
            "Publish branch".into()
        } else {
            format!("Push ↑{ahead}").into()
        };
        let push_cwd = cwd.clone();
        let committing = self.view.requests.get("git-action").is_some_and(|s| {
            s.loading && matches!(s.request, Request::Git(GitAction::Commit { .. }))
        });
        let composer = div()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .border_t_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .debug_selector(|| "review-commit-message".into())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let k = &event.keystroke;
                        if k.key == "enter" && (k.modifiers.control || k.modifiers.platform) {
                            cx.stop_propagation();
                            this.commit_staged(window, cx);
                        }
                    }))
                    .child(Input::new(&self.extras.commit_message).small()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.primary_button(
                            "review-commit",
                            if committing {
                                "Committing…".to_owned()
                            } else if staged > 0 {
                                format!("Commit {staged} staged")
                            } else {
                                "Commit".to_owned()
                            },
                            can_commit,
                        )
                        .flex_1()
                        .flex()
                        .justify_center()
                        .debug_selector(|| "review-commit".into())
                        .when(can_commit, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| this.commit_staged(window, cx)),
                            )
                        }),
                    )
                    .when(ahead > 0 || unpublished, |d| {
                        d.child(
                            self.button("review-push", push_label, can_push)
                                .debug_selector(|| "review-push".into())
                                .when(can_push, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.git_action(
                                            GitAction::Push {
                                                cwd: push_cwd.clone(),
                                            },
                                            cx,
                                        )
                                    }))
                                }),
                        )
                    }),
            )
            .when(staged == 0 && !changes.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child("Stage files to commit them."),
                )
            })
            .when(behind > 0, |d| {
                d.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.warning))
                        .child(format!(
                            "{behind} commit{} behind upstream; pull in a terminal first.",
                            if behind == 1 { "" } else { "s" }
                        )),
                )
            });
        let _ = window;
        card.child(bulk).child(list).child(composer)
    }

    fn change_row(
        &mut self,
        ix: usize,
        change: &Change,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let active = self.review.selected.as_ref() == Some(&change.path);
        let (name, dir) = match change.path.rsplit_once(['/', '\\']) {
            Some((dir, name)) => (name.to_owned(), dir.to_owned()),
            None => (change.path.clone(), String::new()),
        };
        let color = explorer::status_color(change.letter, p);
        let clicked = change.clone();
        // A change with anything unstaged stages; a fully staged one unstages.
        let stage = change.has_unstaged();
        let cwd = self.review_paths().map(|(cwd, _)| cwd).unwrap_or_default();
        let action = if stage {
            GitAction::Stage {
                cwd,
                path: Some(change.path.clone()),
            }
        } else {
            GitAction::Unstage {
                cwd,
                path: Some(change.path.clone()),
            }
        };
        let toggle = self
            .icon_button(
                SharedString::from(format!("review-stage-{ix}")),
                if stage { "Stage" } else { "Unstage" },
                if stage {
                    IconName::Plus
                } else {
                    IconName::Minus
                },
                !busy,
            )
            .size(px(22.))
            .debug_selector({
                let name = format!("review-stage-{}", change.path);
                move || name
            })
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.git_action(action.clone(), cx)
                }))
            });
        div()
            .id(("review-change", ix))
            .debug_selector({
                let name = format!("review-change-{}", change.path);
                move || name
            })
            .w_full()
            .h(px(CHANGE_ROW_HEIGHT))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(p.selected)))
            .hover(|s| s.bg(rgb(p.selected)))
            .on_click(cx.listener(move |this, _, _, cx| this.show_diff(&clicked, None, cx)))
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
                        .child("partly staged"),
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
            .child(toggle)
    }

    fn render_commit_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.appearance.palette();
        let state = self.view.requests.get("git-log");
        if state.is_none_or(|s| s.loading) {
            return div()
                .px_3()
                .py_2()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child("Reading commits…")
                .into_any_element();
        }
        if state.is_some_and(|s| s.error.is_some()) {
            return div()
                .px_3()
                .child(self.feature_message("git-log"))
                .into_any_element();
        }
        let commits = Arc::new(self.commit_rows());
        if commits.is_empty() {
            return div()
                .px_3()
                .py_2()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child("No commits yet.")
                .into_any_element();
        }
        // `git log` is newest first, so the first `ahead` are not pushed.
        let ahead = self.review_status().map_or(0, |s| s.ahead as usize);
        let now = wks_native::timing::now_ms();
        uniform_list(
            "review-commits",
            commits.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|ix| {
                        let (hash, subject, at) = &commits[ix];
                        this.commit_row(ix, hash, subject, *at, ix < ahead, now, cx)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "review-commits".into())
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_row(
        &mut self,
        ix: usize,
        hash: &str,
        subject: &str,
        at: i64,
        unpushed: bool,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let active = self.review.commit.as_deref() == Some(hash);
        let clicked = hash.to_owned();
        div()
            .id(("review-commit", ix))
            .debug_selector({
                let name = format!("review-commit-{hash}");
                move || name
            })
            .w_full()
            .h(px(COMMIT_ROW_HEIGHT))
            .px_3()
            .flex()
            .flex_col()
            .justify_center()
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(p.selected)))
            .hover(|s| s.bg(rgb(p.selected)))
            .on_click(cx.listener(move |this, _, _, cx| this.show_commit(&clicked, cx)))
            .child(
                div()
                    .truncate()
                    .text_size(px(chrome::scale::META))
                    .child(subject.to_owned()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(
                        div()
                            .font_family(mono_font())
                            .text_color(rgb(if active { p.accent } else { p.muted }))
                            .child(hash.to_owned()),
                    )
                    .child(wks_native::history::age(at, now))
                    .when(unpushed, |d| {
                        d.child(div().text_color(rgb(p.warning)).child("not pushed"))
                    }),
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
