//! Session history: every session the hub knows about, grouped by project
//! and searchable from the page itself. Grouping, ordering and matching live
//! in `wks_native::history`; this is the page, its keyboard cursor and the
//! per-group collapse and "Show more" state.
use super::*;
use gpui::AnyElement;
use gpui_component::input::{InputEvent, MoveDown, MoveUp};
use std::collections::HashSet;
use wks_native::history::{self, Entry, Group, GroupKind};
use wks_native::projects;

const CLEARED_TOOLTIP: &str =
    "Cleared from the sidebar on this device · show it under its parent again";
const RECENT_FOOTER: &str = "Names are saved on this device. Archives are shared with every client of this hub, web included. Archiving keeps the conversation and does not stop an agent.";
const SEARCH_PLACEHOLDER: &str = "Search by name, project, folder, provider or model";
/// Rows a group shows before its "Show more".
pub(super) const GROUP_ROWS: usize = 6;
/// Rows each "Show more" adds.
const MORE_ROWS: usize = 25;

pub(super) struct RecentUi {
    pub query: Entity<InputState>,
    /// Groups the user folded, by [`Group::key`]. A search shows every
    /// group with a match regardless.
    pub collapsed: HashSet<String>,
    /// Rows a group shows after "Show more", by [`Group::key`].
    pub shown: HashMap<String, usize>,
    /// Index into the rows on screen ([`RecentPage::rows`]).
    pub cursor: usize,
    pub scroll: gpui::ScrollHandle,
    /// The cursor moved by keyboard: scroll its row into view once.
    pub reveal: std::cell::Cell<bool>,
    _watch: gpui::Subscription,
}

impl RecentUi {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER));
        let _watch = cx.subscribe_in(
            &query,
            window,
            |this: &mut Workspace, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.extras.recent.cursor = 0;
                    this.extras.recent.scroll.set_offset(gpui::Point::default());
                    cx.notify();
                }
                InputEvent::PressEnter { secondary: false } => this.open_recent_cursor(window, cx),
                _ => {}
            },
        );
        Self {
            query,
            collapsed: HashSet::new(),
            shown: HashMap::new(),
            cursor: 0,
            scroll: gpui::ScrollHandle::new(),
            reveal: Default::default(),
            _watch,
        }
    }
}

/// What the page shows right now.
pub(super) struct RecentPage {
    /// Groups after the archive toggle and the search, newest first.
    pub groups: Vec<Group>,
    /// The rows on screen, `(group, entry)`, in screen order: what the
    /// keyboard cursor walks. Folded groups and rows behind "Show more" are
    /// not on screen.
    pub rows: Vec<(usize, usize)>,
    /// Sessions under the archive toggle before the search.
    pub total: usize,
    pub searching: bool,
}

impl RecentPage {
    pub fn matches(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }
}

impl Workspace {
    /// The history rows under the All / Archived toggle, as `history` entries.
    pub(super) fn recent_entries(&self) -> Vec<Entry> {
        let Some(rows) = self
            .view
            .requests
            .get("recent")
            .and_then(|s| s.value.as_array())
        else {
            return Vec::new();
        };
        let live = &self.view.sessions;
        let root = self.worktree_root();
        rows.iter()
            .filter_map(|row| {
                let mut session = Session::default();
                session.merge(row);
                if session.id.is_empty() || self.archived(&session.id) != self.extras.show_archived
                {
                    return None;
                }
                let at = live.iter().position(|l| l.id == session.id);
                // `sessions.recent` carries no model or lineage; the live fleet
                // knows both for the sessions it still holds.
                let mut home = None;
                if let Some(ix) = at {
                    let known = &live[ix];
                    if session.model.is_empty() {
                        session.model = known.model.clone();
                    }
                    if session.runtime_model.is_empty() {
                        session.runtime_model = known.runtime_model.clone();
                    }
                    let tree = |cwd: &str| {
                        projects::in_worktree_root(cwd, root)
                            || projects::worktree_repo(cwd, root).is_some()
                    };
                    if tree(&session.cwd) {
                        home = wks_native::navigation::ancestors(live, ix)
                            .into_iter()
                            .map(|a| live[a].cwd.as_str())
                            .find(|cwd| !cwd.is_empty() && !tree(cwd))
                            .map(str::to_owned);
                    }
                }
                Some(Entry {
                    title: self.session_title(&session),
                    updated_at: row["updatedAt"].as_i64().unwrap_or(0),
                    home,
                    session,
                })
            })
            .collect()
    }

    fn worktree_root(&self) -> &str {
        self.projects
            .registry
            .as_deref()
            .and_then(|r| r["worktreeRoot"].as_str())
            .unwrap_or("")
    }

    pub(super) fn recent_page(&self, cx: &App) -> RecentPage {
        let entries = self.recent_entries();
        let total = entries.len();
        let query = self.extras.recent.query.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let groups = history::search(
            history::group(entries, &self.known_projects(), self.worktree_root()),
            &query,
        );
        let ui = &self.extras.recent;
        let mut rows = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            if !searching && ui.collapsed.contains(&group.key) {
                continue;
            }
            let limit = ui.shown.get(&group.key).copied().unwrap_or(GROUP_ROWS);
            rows.extend((0..group.entries.len().min(limit)).map(|e| (g, e)));
        }
        RecentPage {
            groups,
            rows,
            total,
            searching,
        }
    }

    /// Opening the page: refresh, and put the cursor in the search field.
    pub(super) fn enter_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.recent.cursor = 0;
        self.extras.recent.scroll.set_offset(gpui::Point::default());
        self.load_projects(cx);
        self.extras
            .recent
            .query
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn searching_recent(&self, window: &Window, cx: &App) -> bool {
        self.screen == Screen::Recent
            && !self.new_session
            && self
                .extras
                .recent
                .query
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
    }

    /// Esc on the page clears a search first; `true` when it did.
    pub(super) fn clear_recent_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.screen != Screen::Recent
            || self.new_session
            || self.extras.recent.query.read(cx).value().is_empty()
        {
            return false;
        }
        self.extras
            .recent
            .query
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
        true
    }

    pub(super) fn move_recent_cursor(&mut self, step: isize, cx: &mut Context<Self>) {
        let len = self.recent_page(cx).rows.len();
        if len == 0 {
            return;
        }
        let ui = &mut self.extras.recent;
        ui.cursor = (ui.cursor.min(len - 1) as isize + step).rem_euclid(len as isize) as usize;
        ui.reveal.set(true);
        cx.notify();
    }

    pub(super) fn recent_cursor_edge(&mut self, last: bool, cx: &mut Context<Self>) {
        let len = self.recent_page(cx).rows.len();
        let ui = &mut self.extras.recent;
        ui.cursor = if last { len.saturating_sub(1) } else { 0 };
        ui.reveal.set(true);
        cx.notify();
    }

    /// Enter: open the session under the cursor.
    pub(super) fn open_recent_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.recent_page(cx);
        let Some(&(g, e)) = page.rows.get(
            self.extras
                .recent
                .cursor
                .min(page.rows.len().saturating_sub(1)),
        ) else {
            return;
        };
        let session = page.groups[g].entries[e].session.clone();
        self.open_recent(session, window, cx);
    }

    fn open_recent(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        if !self.view.connected {
            return;
        }
        self.show_screen(Screen::Conversation, window, cx);
        self.command(Command::OpenRecent(Box::new(session)), cx);
    }

    fn toggle_recent_group(&mut self, key: &str, cx: &mut Context<Self>) {
        let collapsed = &mut self.extras.recent.collapsed;
        if !collapsed.remove(key) {
            collapsed.insert(key.to_owned());
        }
        cx.notify();
    }

    /// The page's items below its header: toolbar, status, groups, footer.
    pub(super) fn render_recent(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.appearance.palette();
        let page = self.recent_page(cx);
        // Until the first answer there is nothing to call empty.
        let loading = self.view.requests.get("recent").is_none_or(|s| s.loading);
        let cursor = self
            .extras
            .recent
            .cursor
            .min(page.rows.len().saturating_sub(1));
        let cursor_row = page.rows.get(cursor).copied();
        let known = self.known_projects();
        let now = wks_native::timing::now_ms();
        let mut items: Vec<AnyElement> = Vec::new();
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .debug_selector(|| "history-search".into())
                    // Arrow keys move through the rows while the field keeps
                    // focus, like the project picker.
                    .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                        if this.searching_recent(window, cx) {
                            this.move_recent_cursor(-1, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                        if this.searching_recent(window, cx) {
                            this.move_recent_cursor(1, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        Input::new(&self.extras.recent.query)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .size(px(14.))
                                    .text_color(rgb(p.muted)),
                            )
                            .cleanable(true),
                    ),
            )
            .child(self.segmented(
                "history-filter",
                vec![
                    (false, "All sessions".to_owned()),
                    (true, "Archived".to_owned()),
                ],
                self.extras.show_archived,
                |this, archived, _, cx| {
                    this.extras.show_archived = archived;
                    this.extras.recent.cursor = 0;
                    cx.notify();
                },
                cx,
            ));
        items.push(toolbar.into_any_element());
        items.push(self.render_recent_summary(&page, cx).into_any_element());
        items.push(self.feature_message("recent").into_any_element());
        if page.groups.is_empty() && !loading {
            items.push(
                super::features::empty_note(
                    match (page.searching, self.extras.show_archived) {
                        (true, _) => "No sessions match this search. Esc clears it.",
                        (false, true) => {
                            "No archived sessions. Archive a session to tuck it away without stopping it."
                        }
                        (false, false) => "No sessions yet. Refresh if you expected some.",
                    },
                    p,
                )
                .into_any_element(),
            );
        }
        let mut row_ix = 0;
        for (g, group) in page.groups.iter().enumerate() {
            let collapsed = !page.searching && self.extras.recent.collapsed.contains(&group.key);
            let project = (group.kind == GroupKind::Project)
                .then(|| {
                    known
                        .iter()
                        .find(|k| projects::same_dir(&k.path, &group.path))
                })
                .flatten();
            items.push(
                self.render_recent_group(g, group, project, collapsed, page.searching, now, cx)
                    .into_any_element(),
            );
            if collapsed {
                continue;
            }
            let limit = self
                .extras
                .recent
                .shown
                .get(&group.key)
                .copied()
                .unwrap_or(GROUP_ROWS);
            for (e, entry) in group.entries.iter().take(limit).enumerate() {
                let selected = cursor_row == Some((g, e));
                if selected && self.extras.recent.reveal.replace(false) {
                    self.extras.recent.scroll.scroll_to_item(items.len());
                }
                items.push(
                    self.render_recent_row(row_ix, entry, group, selected, now, cx)
                        .into_any_element(),
                );
                row_ix += 1;
            }
            let hidden = group.entries.len().saturating_sub(limit);
            if hidden > 0 {
                let key = group.key.clone();
                let more = hidden.min(MORE_ROWS);
                items.push(
                    div()
                        .pl(px(30.))
                        .pb_1()
                        .flex()
                        .child(
                            self.button(
                                format!("history-more-{g}"),
                                format!(
                                    "Show {more} more{}",
                                    if more < hidden {
                                        format!(" of {hidden}")
                                    } else {
                                        String::new()
                                    }
                                ),
                                true,
                            )
                            .debug_selector(move || format!("history-more-{g}"))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let shown = this
                                        .extras
                                        .recent
                                        .shown
                                        .entry(key.clone())
                                        .or_insert(GROUP_ROWS);
                                    *shown += MORE_ROWS;
                                    cx.notify();
                                },
                            )),
                        )
                        .into_any_element(),
                );
            }
        }
        items.push(
            div()
                .pt_4()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child(RECENT_FOOTER)
                .into_any_element(),
        );
        items
    }

    /// "142 sessions in 12 projects", or how many a search found, with
    /// fold-all and clear-search actions.
    fn render_recent_summary(&self, page: &RecentPage, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let groups = plural(page.groups.len(), "project", "projects");
        let text = if page.searching {
            format!(
                "{} of {} in {groups}",
                plural(page.matches(), "match", "matches"),
                page.total
            )
        } else {
            format!("{} in {groups}", plural(page.total, "session", "sessions"))
        };
        let all_folded = !page.groups.is_empty()
            && page
                .groups
                .iter()
                .all(|g| self.extras.recent.collapsed.contains(&g.key));
        let keys: Vec<String> = page.groups.iter().map(|g| g.key.clone()).collect();
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(28.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .debug_selector(|| "history-summary".into())
                    .child(text),
            )
            .when(page.searching, |d| {
                d.child(
                    self.quiet_button(
                        "history-clear-search",
                        "Clear search",
                        IconName::Close,
                        true,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_recent_search(window, cx);
                    })),
                )
            })
            .when(!page.searching && page.groups.len() > 1, |d| {
                d.child(
                    self.quiet_button(
                        "history-fold-all",
                        if all_folded {
                            "Expand all"
                        } else {
                            "Collapse all"
                        },
                        if all_folded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        true,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let collapsed = &mut this.extras.recent.collapsed;
                        if all_folded {
                            for key in &keys {
                                collapsed.remove(key);
                            }
                        } else {
                            collapsed.extend(keys.iter().cloned());
                        }
                        this.extras.recent.cursor = 0;
                        cx.notify();
                    })),
                )
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_recent_group(
        &self,
        g: usize,
        group: &Group,
        project: Option<&KnownProject>,
        collapsed: bool,
        searching: bool,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let key = group.key.clone();
        let count = if searching {
            format!("{} of {}", group.entries.len(), group.total)
        } else {
            group.total.to_string()
        };
        let hint = match group.kind {
            GroupKind::NoFolder => String::new(),
            GroupKind::Worktrees => format!("{} · worktrees", group.path),
            GroupKind::Project | GroupKind::Folder => group.path.clone(),
        };
        // A search shows every group with a match open; folding waits for it.
        chrome::interactive_control(div().id(("history-group", g)), p, !searching)
            .debug_selector(move || format!("history-group-{g}"))
            .when(g > 0, |d| d.mt_3())
            .mb_1()
            .px_2()
            .py_1()
            .rounded(px(p.control_radius))
            .flex()
            .items_center()
            .gap_2()
            .when(!searching, |d| {
                d.hover(|s| s.bg(rgb(p.selected)))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_recent_group(&key, cx)))
            })
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(12.))
                .text_color(rgb(if searching { p.disabled } else { p.muted })),
            )
            .child(self.project_mark(project, &group.path, 20.))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(260.))
                    .truncate()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(group.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(hint),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child(history::age(group.updated_at, now)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_2()
                    .rounded_full()
                    .bg(rgb(p.selected))
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(if searching { p.accent } else { p.text }))
                    .debug_selector(move || format!("history-count-{g}"))
                    .child(count),
            )
    }

    fn render_recent_row(
        &self,
        ix: usize,
        entry: &Entry,
        group: &Group,
        selected: bool,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        let s = &entry.session;
        let open = s.clone();
        let resume = s.clone();
        let id = s.id.clone();
        // Inside its group the header names the folder; a row says only
        // where it differs: a subfolder, or which worktree.
        let place = recent_place(&s.cwd, group, self.worktree_root());
        let age = history::age(entry.updated_at, now);
        let dot = || div().flex_shrink_0().text_color(rgb(p.disabled)).child("·");
        chrome::card(p)
            .id(("recent-row", ix))
            .debug_selector(move || format!("recent-row-{ix}"))
            .ml(px(30.))
            .mb_1()
            .px_4()
            .py_3()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .when(selected, |d| {
                d.border_color(rgb(p.accent)).bg(rgb(p.selected))
            })
            .child(
                div()
                    // Actions wrap below a title that would otherwise
                    // be truncated to a word in narrow windows.
                    .flex_1()
                    .min_w(px(220.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(chrome::scale::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(entry.title.clone()),
                            )
                            .child(div().flex_shrink_0().child(session_badge(
                                s,
                                p,
                                self.view.connected,
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(div().flex_shrink_0().child(chrome::model_badge(s, p, 11.)))
                            .when(!age.is_empty(), |d| {
                                d.child(dot()).child(div().flex_shrink_0().child(age))
                            })
                            .when(!place.is_empty(), |d| {
                                d.child(dot()).child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(mono_font())
                                        .child(place),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    // A child cleared from the sidebar on this device
                    // comes back here; that is not an archive restore.
                    .when(self.clear_marked(&s.id), |d| {
                        let id = s.id.clone();
                        d.child(
                            self.button("unclear-recent", "Show in sidebar", true)
                                .tooltip(|window, cx| {
                                    gpui_component::tooltip::Tooltip::new(CLEARED_TOOLTIP)
                                        .build(window, cx)
                                })
                                .on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.unclear_session(&id, cx)
                                    }),
                                ),
                        )
                    })
                    .child(
                        self.button(
                            "archive-recent",
                            if self.archived(&s.id) {
                                "Restore"
                            } else {
                                "Archive"
                            },
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_archive(&id, cx))),
                    )
                    .when(
                        s.stopped() && matches!(s.provider.as_str(), "claude" | "codex"),
                        |d| {
                            d.child(
                                self.button("resume-recent", "Resume…", self.view.connected)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.resume_session(&resume, window, cx)
                                    })),
                            )
                        },
                    )
                    .child(
                        self.primary_button("open-recent", "Open", self.view.connected)
                            .when(self.view.connected, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_recent(open.clone(), window, cx)
                                }))
                            }),
                    ),
            )
    }
}

/// Where a row's folder sits relative to its group: empty when it is the
/// group's own folder, `worktree <name>` for a worktree checkout, the
/// subpath inside the group's folder, else the whole path.
pub(super) fn recent_place(cwd: &str, group: &Group, worktree_root: &str) -> String {
    if cwd.trim().is_empty() || projects::same_dir(cwd, &group.path) {
        return String::new();
    }
    if projects::worktree_repo(cwd, worktree_root).is_some()
        || projects::in_worktree_root(cwd, worktree_root)
    {
        return format!("worktree {}", projects::basename(cwd));
    }
    let key = projects::project_key(cwd);
    let base = projects::project_key(&group.path);
    if !base.is_empty()
        && key.len() > base.len() + 1
        && key.is_char_boundary(base.len())
        && key.as_bytes()[base.len()] == b'/'
        && projects::same_dir(&key[..base.len()], &base)
    {
        return key[base.len() + 1..].to_owned();
    }
    key
}
