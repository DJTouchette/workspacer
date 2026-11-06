//! Session history: every session the hub knows about, grouped by project
//! and searchable from the page itself. Grouping, ordering and matching live
//! in `wks_native::history`; this is the page, its keyboard cursor and the
//! per-group collapse and "Show more" state.
use super::*;
use gpui::AnyElement;
use gpui_component::input::{InputEvent, MoveDown, MoveUp};
use serde_json::Value;
use std::collections::HashSet;
use wks_native::features::Request;
use wks_native::history::{self, Entry, Group, GroupKind};
use wks_native::projects;

const CLEARED_TOOLTIP: &str =
    "Cleared from the sidebar on this device · show it under its parent again";
const RECENT_FOOTER: &str = "History keeps every session. Hide takes a session off every sidebar on this hub, web included, and keeps it here; a running agent is ended first, so resuming starts it fresh. Delete removes a stopped session from history for good. Names are saved on this device.";
/// How long typing pauses before the hub is asked to search too.
const SEARCH_DEBOUNCE_MS: u64 = 300;
const SEARCH_PLACEHOLDER: &str = "Search by name, project, folder, provider or model";
/// Rows a group shows before its "Show more".
pub(super) const GROUP_ROWS: usize = 6;
/// Rows each "Show more" adds.
const MORE_ROWS: usize = 25;
/// Where a row's title (and its group's name) starts: past the fold
/// chevron and the project mark.
const ROW_TITLE_X: f32 = 58.;
/// The hover group of one row: its actions show while it is hovered.
const ROW_GROUP: &str = "history-row";

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
    /// The debounced hub search waiting for typing to pause.
    pub search_task: Option<gpui::Task<()>>,
    /// The row whose "Delete…" was pressed: it asks to confirm first.
    pub confirm_delete: Option<String>,
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
                    this.schedule_history_search(cx);
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
            search_task: None,
            confirm_delete: None,
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
    /// Whether the hub answered with `sessions.history`, which pages, searches
    /// server-side and can delete. An older hub's `sessions.recent` cannot.
    pub(super) fn history_available(&self) -> bool {
        self.view
            .requests
            .get("recent")
            .is_some_and(|s| s.value["history"] == true)
    }

    /// Whether `id` is hidden: the shared archive once it is known (with any
    /// click still on its way), else what the history row itself said.
    fn history_hidden(&self, id: &str, row: Option<&Value>) -> bool {
        if self.archived(id) {
            return true;
        }
        if self.view.session_archive.is_some() {
            return false;
        }
        let row = row.or_else(|| {
            self.view
                .requests
                .get("recent")
                .and_then(|s| history_rows(&s.value))
                .and_then(|rows| rows.iter().find(|r| r["sessionId"].as_str() == Some(id)))
        });
        row.is_some_and(|row| row["hidden"] == true)
    }

    /// The history rows under the All / Hidden toggle, as `history` entries,
    /// plus the ids the hub's own search matched for `query` that the loaded
    /// list does not hold.
    pub(super) fn recent_entries(&self, query: &str) -> (Vec<Entry>, HashSet<String>) {
        let mut rows: Vec<&Value> = self
            .view
            .requests
            .get("recent")
            .and_then(|s| history_rows(&s.value))
            .map(|rows| rows.iter().collect())
            .unwrap_or_default();
        // A hub search for exactly this query adds what it found, even rows
        // the loaded pages do not reach and fields this page cannot match.
        let mut matched = HashSet::new();
        if !query.trim().is_empty()
            && let Some(found) = self
                .view
                .requests
                .get("recent-search")
                .filter(|s| s.value["query"].as_str() == Some(query.trim()))
                .and_then(|s| history_rows(&s.value))
        {
            let known: HashSet<&str> = rows
                .iter()
                .filter_map(|row| row["sessionId"].as_str())
                .collect();
            for row in found {
                if let Some(id) = row["sessionId"].as_str() {
                    matched.insert(id.to_owned());
                    if !known.contains(id) {
                        rows.push(row);
                    }
                }
            }
        }
        let live = &self.view.sessions;
        let root = self.worktree_root();
        let entries = rows
            .into_iter()
            .filter_map(|row| {
                let mut session = Session::default();
                session.merge(row);
                if session.id.is_empty()
                    || self.extras.show_archived && !self.history_hidden(&session.id, Some(row))
                {
                    return None;
                }
                let at = live.iter().position(|l| l.id == session.id);
                // History rows come from the hub's store and read as stopped;
                // the live fleet knows the state, model and lineage of the
                // sessions it still holds.
                let mut home = None;
                if let Some(ix) = at {
                    let known = &live[ix];
                    if !known.state.is_empty() {
                        session.state = known.state.clone();
                    }
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
            .collect();
        (entries, matched)
    }

    /// Ask the hub to search too once typing pauses. Only `sessions.history`
    /// can; on an older hub the page's own search is all there is.
    fn schedule_history_search(&mut self, cx: &mut Context<Self>) {
        self.extras.recent.search_task = None;
        if !self.history_available() {
            return;
        }
        self.extras.recent.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SEARCH_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                let query = this.extras.recent.query.read(cx).value().trim().to_owned();
                if !query.is_empty() {
                    this.request(
                        Request::HistoryPage {
                            query,
                            cursor: None,
                        },
                        cx,
                    );
                }
            });
        }));
    }

    /// The next page of the unfiltered list, when the hub has more.
    pub(super) fn history_next(&self) -> Option<String> {
        self.view
            .requests
            .get("recent")
            .filter(|s| s.value["history"] == true)
            .and_then(|s| s.value["next"].as_str())
            .map(str::to_owned)
    }

    fn load_older_history(&mut self, cx: &mut Context<Self>) {
        if let Some(cursor) = self.history_next() {
            self.request(
                Request::HistoryPage {
                    query: String::new(),
                    cursor: Some(cursor),
                },
                cx,
            );
        }
    }

    /// "Delete…" asks first; the second press deletes for good.
    fn delete_from_history(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.extras.recent.confirm_delete.as_deref() == Some(id) {
            self.extras.recent.confirm_delete = None;
            self.request(
                Request::ForgetSession {
                    session: id.to_owned(),
                },
                cx,
            );
        } else {
            self.extras.recent.confirm_delete = Some(id.to_owned());
        }
        cx.notify();
    }

    pub(super) fn worktree_root(&self) -> &str {
        self.projects
            .registry
            .as_deref()
            .and_then(|r| r["worktreeRoot"].as_str())
            .unwrap_or("")
    }

    pub(super) fn recent_page(&self, cx: &App) -> RecentPage {
        let query = self.extras.recent.query.read(cx).value().to_string();
        let (entries, matched) = self.recent_entries(&query);
        let total = entries.len();
        let searching = !query.trim().is_empty();
        let groups = history::search_with(
            history::group(entries, &self.known_projects(), self.worktree_root()),
            &query,
            |entry| matched.contains(&entry.session.id),
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
        let p = self.palette();
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
                                    .size(px(icon_size::LG))
                                    .text_color(rgb(p.muted)),
                            )
                            .cleanable(true)
                            // The height of the filter pills beside it.
                            .min_h(px(36.)),
                    ),
            )
            .child(self.segmented(
                "history-filter",
                vec![
                    (false, "All sessions".to_owned()),
                    (true, "Hidden".to_owned()),
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
        items.push(self.feature_message("session-forget").into_any_element());
        if page.groups.is_empty() && !loading {
            items.push(self.render_recent_empty(&page, cx).into_any_element());
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
                self.render_recent_group(g, group, project, collapsed, page.searching, cx)
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
                        // The label lines up with the row titles above it.
                        .pl(px(ROW_TITLE_X - 10.))
                        .pb_1()
                        .flex()
                        .child(
                            chrome::interactive_control(
                                div().id(SharedString::from(format!("history-more-{g}"))),
                                p,
                                true,
                            )
                            .debug_selector(move || format!("history-more-{g}"))
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded(px(p.control_radius))
                            .text_size(px(chrome::scale::META))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(p.accent))
                            .hover(move |s| s.bg(chrome::tonal(p, alpha::SUBTLE)))
                            .child(Icon::new(IconName::ChevronDown).size(px(icon_size::SM)))
                            .child(format!(
                                "Show {more} more{}",
                                if more < hidden {
                                    format!(" of {hidden}")
                                } else {
                                    String::new()
                                }
                            ))
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
        if !page.searching && self.history_next().is_some() {
            let loading_more = self
                .view
                .requests
                .get("recent-more")
                .is_some_and(|s| s.loading);
            let enabled = self.view.connected && !loading_more;
            items.push(
                div()
                    .pt_2()
                    .flex()
                    .justify_center()
                    .child(
                        self.button(
                            "history-load-more",
                            if loading_more {
                                "Loading older sessions…"
                            } else {
                                "Load older sessions"
                            },
                            enabled,
                        )
                        .debug_selector(|| "history-load-more".into())
                        .when(enabled, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.load_older_history(cx)))
                        }),
                    )
                    .into_any_element(),
            );
            items.push(self.feature_message("recent-more").into_any_element());
        }
        items.push(
            div()
                .mt_6()
                .pt_4()
                .border_t_1()
                .border_color(rgb(p.border))
                .child(
                    div()
                        .max_w(px(640.))
                        .text_size(px(chrome::scale::CAPTION))
                        .line_height(gpui::relative(1.5))
                        .text_color(rgb(p.muted))
                        .child(RECENT_FOOTER),
                )
                .into_any_element(),
        );
        items
    }

    /// "142 sessions in 12 projects", or how many a search found, with
    /// fold-all and clear-search actions.
    fn render_recent_summary(&self, page: &RecentPage, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let groups = plural(page.groups.len(), "project", "projects");
        // An empty page says why below; the line only counts what is there.
        let text = match (page.searching, page.matches(), page.total) {
            (true, 0, total) => {
                format!("No matches among {}", plural(total, "session", "sessions"))
            }
            (true, matches, total) => format!(
                "{} of {total} in {groups}",
                plural(matches, "match", "matches")
            ),
            (false, _, 0) => String::new(),
            (false, _, total) => {
                format!("{} in {groups}", plural(total, "session", "sessions"))
            }
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
            .mb_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::META))
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
                            IconName::ChevronsUpDown
                        } else {
                            IconName::ChevronUp
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

    /// Nothing to list: a search that found nothing, an empty Hidden
    /// filter, or a hub with no sessions yet. Says which, and the way out.
    fn render_recent_empty(&self, page: &RecentPage, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let (icon, title, body) = match (page.searching, self.extras.show_archived) {
            (true, _) => (
                IconName::Search,
                "No sessions match",
                "Try a name, project, folder, provider or model. Esc clears the search.",
            ),
            (false, true) => (
                IconName::EyeOff,
                "Nothing hidden",
                "Hide a session to take it off every sidebar. It stays here in history.",
            ),
            (false, false) => (
                IconName::BookOpen,
                "No sessions yet",
                "Sessions you start show up here, ended ones included.",
            ),
        };
        div()
            .debug_selector(|| "history-empty".into())
            .py_10()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .text_center()
            .child(
                div()
                    .size(px(36.))
                    .mb_1()
                    .rounded_full()
                    .bg(chrome::tonal(p, alpha::SUBTLE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(icon)
                            .size(px(icon_size::XL))
                            .text_color(rgb(p.muted)),
                    ),
            )
            .child(
                div()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(420.))
                    .text_size(px(chrome::scale::META))
                    .line_height(gpui::relative(1.45))
                    .text_color(rgb(p.muted))
                    .child(body),
            )
            .when(!page.searching && self.extras.show_archived, |d| {
                d.child(
                    div().pt_2().child(
                        self.button("history-show-all", "Show all sessions", true)
                            .debug_selector(|| "history-show-all".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.extras.show_archived = false;
                                this.extras.recent.cursor = 0;
                                cx.notify();
                            })),
                    ),
                )
            })
    }

    fn render_recent_group(
        &self,
        g: usize,
        group: &Group,
        project: Option<&KnownProject>,
        collapsed: bool,
        searching: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
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
        // Its newest row already says how recent the group is.
        chrome::interactive_control(div().id(("history-group", g)), p, !searching)
            .debug_selector(move || format!("history-group-{g}"))
            .when(g > 0, |d| d.mt_4())
            .mb(px(2.))
            .h(px(36.))
            .px_2()
            .rounded(px(p.control_radius))
            .flex()
            .items_center()
            .gap_2()
            .when(!searching, |d| {
                d.hover(move |s| s.bg(chrome::tonal(p, alpha::FAINT)))
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
                    .min_w(px(22.))
                    .px(px(6.))
                    .rounded_full()
                    .bg(chrome::tonal(p, alpha::REST))
                    .flex()
                    .justify_center()
                    .text_size(px(chrome::scale::CAPTION))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(if searching { p.accent } else { p.muted }))
                    .debug_selector(move || format!("history-count-{g}"))
                    .child(count),
            )
    }

    /// One session: a flat row under its group that opens on click, with
    /// its title under the group's name; a live session also names its
    /// state. The age gives way to the row's quiet actions while the row is
    /// hovered or under the keyboard cursor.
    fn render_recent_row(
        &self,
        ix: usize,
        entry: &Entry,
        group: &Group,
        selected: bool,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let s = &entry.session;
        let open = s.clone();
        let resume = s.clone();
        let id = s.id.clone();
        let connected = self.view.connected;
        // Inside its group the header names the folder; a row says only
        // where it differs: a subfolder, or which worktree.
        let place = recent_place(&s.cwd, group, self.worktree_root());
        let age = history::age(entry.updated_at, now);
        let dot = || div().flex_shrink_0().text_color(rgb(p.disabled)).child("·");
        let hidden = self.history_hidden(&s.id, None);
        let confirming = self.extras.recent.confirm_delete.as_deref() == Some(s.id.as_str());
        // Ended is what most of history is, so it goes unsaid; a live
        // session's state is worth reading.
        let live = connected && !s.stopped();
        let reveal = selected || confirming;
        let mut actions: Vec<Stateful<Div>> = Vec::new();
        // A child cleared from the sidebar on this device comes back here;
        // that is not an archive restore.
        if self.clear_marked(&s.id) {
            let id = s.id.clone();
            actions.push(
                self.row_action("unclear-recent", "Show in sidebar", true, false)
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(CLEARED_TOOLTIP).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.unclear_session(&id, cx)
                    })),
            );
        }
        if s.stopped() && matches!(s.provider.as_str(), "claude" | "codex") {
            actions.push(
                self.row_action("resume-recent", "Resume…", connected, false)
                    .when(connected, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.resume_session(&resume, window, cx)
                        }))
                    }),
            );
        }
        actions.push(
            self.row_action(
                "archive-recent",
                if hidden { "Unhide" } else { "Hide" },
                true,
                false,
            )
            .debug_selector(move || format!("recent-hide-{ix}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_archive(&id, cx)
            })),
        );
        if self.history_available() {
            actions.extend(self.render_forget_controls(ix, s, confirming, cx));
        }
        div()
            .id(("recent-row", ix))
            .group(ROW_GROUP)
            .debug_selector(move || format!("recent-row-{ix}"))
            .relative()
            .ml(px(ROW_TITLE_X - 36.))
            .mb(px(2.))
            .min_h(px(48.))
            // The title lines up with its group's name.
            .pl(px(36.))
            .pr(px(6.))
            .py(px(6.))
            .rounded(px(p.control_radius))
            .flex()
            .items_center()
            .gap_3()
            .when(connected, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_recent(open.clone(), window, cx)
                    }))
            })
            .map(|d| {
                if selected {
                    d.bg(rgb(p.selected))
                } else {
                    d.hover(move |s| s.bg(chrome::tonal(p, alpha::FAINT)))
                }
            })
            // The keyboard cursor: the row's fill plus an accent rule.
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(10.))
                        .bottom(px(10.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(rgb(p.accent)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
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
                                    .text_size(px(chrome::scale::BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(entry.title.clone()),
                            )
                            .when(live, |d| {
                                d.child(div().flex_shrink_0().child(session_badge(s, p, connected)))
                            })
                            // Under the Hidden filter every row is; the badge would only repeat it.
                            .when(hidden && !self.extras.show_archived, |d| {
                                d.child(
                                    div()
                                        .flex_shrink_0()
                                        .px(px(6.))
                                        .rounded_full()
                                        .bg(chrome::tonal(p, alpha::REST))
                                        .text_size(px(chrome::scale::CAPTION))
                                        .text_color(rgb(p.muted))
                                        .debug_selector(move || format!("recent-hidden-{ix}"))
                                        .child("Hidden"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .min_w_0()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(div().flex_shrink_0().child(chrome::model_badge(s, p, 11.)))
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
            .map(|row| {
                let cluster = div()
                    .id("recent-actions")
                    .flex()
                    .items_center()
                    .gap_1()
                    // A press between the actions is not a press on the row.
                    .on_click(|_, _, cx| cx.stop_propagation());
                if reveal {
                    return row.child(cluster.flex_shrink_0().children(actions));
                }
                // At rest the row keeps its width for the title and shows
                // its age; hovered, the actions cover the end of the row.
                // Only the cursor's row is in the Tab order, so Tab never
                // lands on an action nobody can see.
                row.child(
                    div()
                        .flex_shrink_0()
                        .pr_2()
                        .whitespace_nowrap()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .group_hover(ROW_GROUP, |s| s.opacity(0.))
                        .child(age),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .flex()
                        .opacity(0.)
                        .group_hover(ROW_GROUP, |s| s.opacity(1.))
                        .child(
                            // The hovered row's own fill, made opaque so a
                            // long title ends under the actions.
                            div()
                                .flex()
                                .bg(rgb(p.chat))
                                .rounded(px(p.control_radius))
                                .child(
                                    cluster
                                        .h_full()
                                        .pl_3()
                                        .pr(px(6.))
                                        .rounded(px(p.control_radius))
                                        .bg(chrome::tonal(p, alpha::FAINT))
                                        .children(actions.into_iter().map(|a| a.tab_stop(false))),
                                ),
                        ),
                )
            })
    }

    /// A row's text action: no fill until hovered, so a list of rows never
    /// repeats a column of buttons. Muted, or error-toned when destructive.
    fn row_action(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        enabled: bool,
        danger: bool,
    ) -> Stateful<Div> {
        let p = self.palette();
        let (wash, ink) = if danger {
            (gpui::Hsla::from(rgb(p.error)).opacity(0.12), p.error)
        } else {
            (chrome::tonal(p, alpha::REST), p.text)
        };
        chrome::interactive_control(div().id(id), p, enabled)
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .flex_shrink_0()
            .rounded(px(p.control_radius))
            .text_size(px(chrome::scale::META))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(match (enabled, danger) {
                (false, _) => p.disabled,
                (true, true) => p.error,
                (true, false) => p.muted,
            }))
            .when(enabled, |d| {
                d.hover(move |s| s.bg(wash).text_color(rgb(ink)))
                    .active(move |s| s.bg(wash).text_color(rgb(ink)))
            })
            .child(label.into())
    }

    /// "Delete…" on a history row, or its confirmation. A live session cannot
    /// be deleted; the hub refuses it too.
    fn render_forget_controls(
        &self,
        ix: usize,
        s: &Session,
        confirming: bool,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let stopped = s.stopped();
        let forgetting = self
            .view
            .requests
            .get("session-forget")
            .is_some_and(|state| state.loading);
        let enabled = stopped && self.view.connected && !forgetting;
        let id = s.id.clone();
        if confirming {
            let cancel = s.id.clone();
            vec![
                self.row_action("forget-cancel", "Cancel", true, false)
                    .debug_selector(move || format!("recent-delete-cancel-{ix}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.extras.recent.confirm_delete.as_deref() == Some(cancel.as_str()) {
                            this.extras.recent.confirm_delete = None;
                        }
                        cx.notify();
                    })),
                self.danger_button("forget-recent", "Delete for good", enabled)
                    .h(px(28.))
                    .py_0()
                    .flex()
                    .items_center()
                    .debug_selector(move || format!("recent-delete-confirm-{ix}"))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.delete_from_history(&id, cx)
                        }))
                    }),
            ]
        } else {
            vec![
                self.row_action("forget-recent", "Delete…", enabled, true)
                    .debug_selector(move || format!("recent-delete-{ix}"))
                    .when(!stopped, |d| {
                        d.tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new(
                                "Stop the session before deleting it from history",
                            )
                            .build(window, cx)
                        })
                    })
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.delete_from_history(&id, cx)
                        }))
                    }),
            ]
        }
    }
}

/// The rows of a `recent` reply: `sessions.history`'s `{sessions}` page or
/// an older hub's bare `sessions.recent` array.
fn history_rows(value: &Value) -> Option<&Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get("sessions").and_then(Value::as_array))
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
