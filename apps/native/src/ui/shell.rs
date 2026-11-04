//! Design v2's shell: the attention-first sidebar (one labelled list of
//! destinations, Needs you as a pure inbox, single-line rows for what is
//! quiet), its account-and-usage footer, the collapsed rail of status-ringed
//! avatar tiles, `g n` (next session that needs you) and the keyboard cheat
//! sheet. Everything here draws only under `Design::V2`; the shipped sidebar
//! lives in `sidebar.rs`.
use super::attention::{self, Bucket, InboxAction, Reason};
use super::primitives::{
    self, ButtonSize, ButtonVariant, Intent, StatusBadge, StatusKind, kbd, status_indicator,
    tooltip_view,
};
use super::sidebar::SidebarRow;
use super::*;
use gpui::{AnyElement, AnyView, HighlightStyle, StyledText, deferred};
use gpui_component::tooltip::Tooltip;
use std::collections::{BTreeMap, BTreeSet};
use wks_native::child_agents::ChildAgent;

/// Window-local state of the v2 shell.
pub(super) struct ShellUi {
    /// Folded groups, by `Bucket::index`. Ended starts folded.
    pub(super) folded: [bool; 4],
    /// The keyboard cheat sheet (`?`, Ctrl+/).
    pub(super) shortcuts: bool,
    /// The model, effort and access popover under the title's model chip.
    pub(super) model_popover: bool,
    /// Where the model chip was last drawn, so the popover hangs from it.
    pub(super) model_anchor: gpui::Bounds<gpui::Pixels>,
    /// "Finish setup" was dismissed for this run.
    pub(super) setup_dismissed: bool,
    // r2-shell
    /// Idle shows every session, not only the most recent few.
    pub(super) idle_all: bool,
    /// Sessions whose subagents are listed one by one instead of folded
    /// into "4 subagents · 1 not responding".
    pub(super) open_children: BTreeSet<String>,
    /// The session list's scroll (rows have two heights, so it is not a
    /// uniform list).
    pub(super) scroll: gpui::ScrollHandle,
    /// The command palette (Ctrl+K).
    pub(super) palette: super::palette::PaletteUi,
    // r3-foundation
    /// Failed background tasks the user acknowledged in the title's red
    /// chip, by task id: their chip is gone for this run (memory only).
    pub(super) acknowledged_failures: BTreeSet<String>,
    // r3-shell
    /// The model popover was opened from the header (toolbar or access
    /// chip), so it hangs from the strip rather than the composer's chip.
    pub(super) model_from_header: bool,
    /// The model row the keyboard is on in the popover (↑/↓, Enter).
    pub(super) model_cursor: usize,
    /// The popover's focus, so its keys reach it.
    pub(super) model_focus: Option<gpui::FocusHandle>,
}

impl Default for ShellUi {
    fn default() -> Self {
        Self {
            folded: [false, false, false, true],
            shortcuts: false,
            model_popover: false,
            model_anchor: gpui::Bounds::default(),
            setup_dismissed: false,
            idle_all: false,
            open_children: BTreeSet::new(),
            scroll: gpui::ScrollHandle::new(),
            palette: Default::default(),
            acknowledged_failures: BTreeSet::new(),
            model_from_header: false,
            model_cursor: 0,
            model_focus: None,
        }
    }
}

/// One row of the v2 session list.
#[derive(Clone)]
pub(super) enum ShellRow {
    Group {
        bucket: Bucket,
        count: usize,
        folded: bool,
    },
    /// A session or subagent, in the group its tree belongs to; `tag` tells
    /// a repeated title apart ("3d", "#2").
    Item {
        row: SidebarRow,
        bucket: Bucket,
        tag: Option<String>,
    },
    /// A session's subagents folded into one line.
    Children {
        parent: String,
        depth: usize,
        summary: String,
        fault: bool,
        open: bool,
    },
    /// Needs you is empty.
    AllClear,
    /// Idle's older sessions, folded.
    ShowMore { hidden: usize },
}

/// A provider-native child's status: a running child that has gone quiet is
/// a fault ("Not responding"), never also Working.
fn child_kind(child: &ChildAgent, connected: bool, now: i64) -> StatusKind {
    if !connected {
        StatusKind::Idle
    } else if child.failed() {
        StatusKind::Failed
    } else if attention::child_stalled(child, now) {
        StatusKind::NotResponding
    } else if child.running() {
        StatusKind::Working
    } else if child.settled() {
        StatusKind::Done
    } else if wks_native::child_agents::provider_active(&child.status) {
        StatusKind::NeedsYou
    } else {
        StatusKind::Idle
    }
}

/// A provider's mark in the neutral identity colour (v2 keeps brand colour
/// for the composer's model chip only).
pub(super) fn neutral_glyph(provider: &str, size: f32, p: Palette) -> AnyElement {
    let color = rgb(p.roles.provider);
    match provider {
        "claude" => gpui::svg()
            .path("brand/claude.svg")
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
        "codex" => gpui::svg()
            .path("brand/openai.svg")
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
        _ => Icon::new(IconName::Bot)
            .size(px(size))
            .flex_shrink_0()
            .text_color(color)
            .into_any_element(),
    }
}

/// The brand mark and the "Workspacer" wordmark beside it.
pub(super) fn wordmark(p: Palette) -> Div {
    let t = p.tokens;
    div()
        .flex()
        .items_center()
        .gap(px(t.space.md))
        .flex_shrink_0()
        .child(brand_mark(t.icon.lg, p))
        .child(
            div()
                .debug_selector(|| "sidebar-wordmark".into())
                .text_size(px(t.text.subheading))
                .font_weight(FontWeight(t.weight.semibold))
                .text_color(rgb(p.text))
                .child("Workspacer"),
        )
}

/// `text` with every match of `query` drawn in the full text colour and
/// semibold (the rest keeps the parent's colour).
pub(super) fn highlighted(text: String, query: &str, p: &Palette) -> StyledText {
    let ranges = attention::match_ranges(&text, query);
    let style = HighlightStyle {
        color: Some(rgb(p.text).into()),
        font_weight: Some(FontWeight(p.tokens.weight.semibold)),
        background_color: Some(primitives::wash(p.warning, p.tokens.wash.tone_rest)),
        ..Default::default()
    };
    StyledText::new(text).with_highlights(ranges.into_iter().map(|r| (r, style)))
}

/// Tabular numerals (`tnum`), so ages and counts line up in a column.
pub(super) fn tabular<E: Styled>(mut element: E) -> E {
    element
        .text_style()
        .get_or_insert_with(Default::default)
        .font_features = Some(gpui::FontFeatures(std::sync::Arc::new(vec![(
        "tnum".into(),
        1,
    )])));
    element
}

/// A small icon action inside a row, shown on hover.
fn row_action(id: SharedString, icon: IconName, tip: &'static str, p: &Palette) -> Stateful<Div> {
    let t = p.tokens;
    primitives::icon_button(id, icon, tip, ButtonVariant::Tertiary, true, p)
        .size(px(t.icon.md + t.space.sm))
        .rounded(px(t.radius.sm))
}

/// A rail tile's hover card: title, status, then the meta line.
fn hover_card(
    title: String,
    status: StatusBadge,
    meta: String,
    p: Palette,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let (title, status, meta) = (title.clone(), status.clone(), meta.clone());
        Tooltip::element(move |_, _| {
            let t = p.tokens;
            div()
                .debug_selector(|| "rail-hover-card".into())
                .max_w(px(t.layout.menu_width))
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(
                    div()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.semibold))
                        .text_color(rgb(p.text))
                        .child(title.clone()),
                )
                .child(status.clone().render(&p))
                .when(!meta.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(meta.clone()),
                    )
                })
        })
        .build(window, cx)
    }
}

/// One subagent or child session under a tree's root, before it is shown
/// on its own row or folded into the summary.
struct Descendant {
    row: SidebarRow,
    kind: StatusKind,
    /// Blocked on the user, failed or stalled: never folded away.
    loud: bool,
    matches: bool,
    active: bool,
}

impl Workspace {
    // r2-shell
    /// Modal (vim) keys for the design on screen: v2 starts with them off.
    pub(super) fn vim(&self) -> bool {
        self.settings.vim(self.design())
    }

    /// The sidebar's search text, trimmed.
    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_owned()
    }

    /// How long `session` (or its quietest running subagent) has been
    /// silent, when that is past the heartbeat threshold.
    pub(super) fn stalled_for(&self, session: &Session, now: i64) -> Option<i64> {
        if !self.view.connected || session.stopped() {
            return None;
        }
        let own = attention::session_stalled(session, now)
            .then(|| session.telemetry.last_activity_ms.map(|at| now - at))
            .flatten();
        let children = self
            .native_children(session)
            .iter()
            .filter(|c| attention::child_stalled(c, now))
            .filter_map(|c| {
                c.telemetry
                    .last_activity_ms
                    .or(c.telemetry.started_at_ms)
                    .map(|at| now - at)
            })
            .max();
        own.max(children)
    }

    /// Failed background work of a live session (a failure to look at).
    fn failed_tasks(&self, session: &Session) -> usize {
        if session.stopped() {
            return 0;
        }
        session.tasks.iter().filter(|t| t.failed()).count()
    }

    /// Why a session sits in Needs you, and its inline action.
    pub(super) fn inbox_reason(&self, session: &Session, now: i64) -> Option<Reason> {
        attention::reason(
            session,
            self.stalled_for(session, now),
            self.failed_tasks(session),
        )
    }

    /// The group one session belongs to on its own: stalls and failures
    /// need you; a subagent at work keeps an idle parent among the working.
    pub(super) fn session_bucket(&self, session: &Session, now: i64) -> Bucket {
        let own = attention::bucket(session, self.paused(session));
        if own == Bucket::NeedsYou {
            return own;
        }
        if self.view.connected
            && !session.stopped()
            && (self.stalled_for(session, now).is_some() || self.failed_tasks(session) > 0)
        {
            return Bucket::NeedsYou;
        }
        if own > Bucket::Working && self.native_children(session).iter().any(|c| c.running()) {
            Bucket::Working
        } else {
            own
        }
    }

    /// What a session row shows as its status (stalled is a fault).
    fn session_kind(&self, session: &Session, now: i64) -> StatusKind {
        if !self.view.connected {
            return StatusKind::Idle;
        }
        if self.paused(session) {
            return StatusKind::Paused;
        }
        if attention::needs_you(session) {
            return StatusKind::NeedsYou;
        }
        if self.stalled_for(session, now).is_some() {
            return StatusKind::NotResponding;
        }
        match attention::bucket(session, false) {
            Bucket::Working => StatusKind::Working,
            _ => StatusKind::Idle,
        }
    }

    fn is_active(&self, id: &str) -> bool {
        self.navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref())
            .is_some_and(|s| s == id)
            && (self.navigation_selected.is_some() || self.view.child.is_none())
    }

    /// The sidebar's rows under v2: each group's header, then its trees.
    /// A tree's subagents fold into one summary line unless one is blocked,
    /// failed, open or matches the search; Idle shows the most recent few.
    pub(super) fn shell_rows(&self, cx: &App) -> Vec<ShellRow> {
        let visible = self.visible_sessions(cx);
        let sessions = &self.view.sessions;
        let tree = wks_native::navigation::session_tree(sessions, &visible);
        let query = self.query(cx);
        let searching = !query.is_empty();
        let lower = query.to_lowercase();
        let now = timing::now_ms();
        let buckets: BTreeMap<usize, Bucket> = tree
            .iter()
            .map(|&(ix, _)| (ix, self.session_bucket(&sessions[ix], now)))
            .collect();
        let bucket_of = |ix: usize| buckets.get(&ix).copied().unwrap_or(Bucket::Idle);
        let groups = attention::group(&tree, bucket_of, |ix| {
            sessions[ix].telemetry.last_activity_ms
        });
        let mut rows = Vec::new();
        let needs_empty = !groups.iter().any(|(b, _)| *b == Bucket::NeedsYou);
        if needs_empty && !searching && !visible.is_empty() {
            rows.push(ShellRow::Group {
                bucket: Bucket::NeedsYou,
                count: 0,
                folded: false,
            });
            rows.push(ShellRow::AllClear);
        }
        for (bucket, members) in groups {
            // Back into trees: a root and the members nested under it.
            let mut trees: Vec<Vec<(usize, usize)>> = Vec::new();
            for (ix, depth) in members {
                match trees.last_mut() {
                    Some(current) if depth > 0 => current.push((ix, depth)),
                    _ => trees.push(vec![(ix, depth)]),
                }
            }
            let folded = self.shell.folded[bucket.index()] && !searching;
            rows.push(ShellRow::Group {
                bucket,
                count: trees.len(),
                folded,
            });
            if folded {
                continue;
            }
            let limit = self.tokens().shell().idle_visible;
            let capped = bucket == Bucket::Idle && !searching && !self.shell.idle_all;
            let hidden = if capped {
                trees.len().saturating_sub(limit)
            } else {
                0
            };
            let shown = trees.len() - hidden;
            let mut items: Vec<ShellRow> = Vec::new();
            for members in trees.into_iter().take(shown) {
                let (root, depth) = members[0];
                items.push(ShellRow::Item {
                    row: SidebarRow::Session { index: root, depth },
                    bucket,
                    tag: None,
                });
                self.push_descendants(&members, bucket, &lower, now, &mut items);
            }
            // Repeated titles among this group's top-level rows.
            let keys: Vec<(String, Option<String>)> = items
                .iter()
                .map(|row| match row {
                    ShellRow::Item {
                        row: SidebarRow::Session { index, depth: 0 },
                        ..
                    } => {
                        let s = &sessions[*index];
                        (
                            self.session_title(s),
                            s.telemetry
                                .last_activity_ms
                                .map(|at| wks_native::format::age(at, now)),
                        )
                    }
                    _ => (String::new(), None),
                })
                .collect();
            let tags = attention::disambiguate(&keys);
            for ((row, tag), (title, _)) in items.iter_mut().zip(tags).zip(keys.iter()) {
                // Nested rows carry an empty key: never tagged.
                if let ShellRow::Item { tag: slot, .. } = row {
                    *slot = tag.filter(|_| !title.is_empty());
                }
            }
            rows.extend(items);
            if hidden > 0 {
                rows.push(ShellRow::ShowMore { hidden });
            }
        }
        rows
    }

    /// A tree's members after its root, and every member's subagents: the
    /// loud, open, active or matching ones on their own rows, the rest in
    /// one summary line.
    fn push_descendants(
        &self,
        members: &[(usize, usize)],
        bucket: Bucket,
        query: &str,
        now: i64,
        out: &mut Vec<ShellRow>,
    ) {
        let sessions = &self.view.sessions;
        let connected = self.view.connected;
        let root = &sessions[members[0].0];
        let mut found: Vec<Descendant> = Vec::new();
        for (n, &(ix, depth)) in members.iter().enumerate() {
            let s = &sessions[ix];
            if n > 0 {
                let kind = self.session_kind(s, now);
                found.push(Descendant {
                    row: SidebarRow::Session { index: ix, depth },
                    kind,
                    loud: matches!(
                        kind,
                        StatusKind::NeedsYou | StatusKind::Failed | StatusKind::NotResponding
                    ),
                    matches: !query.is_empty()
                        && self.session_title(s).to_lowercase().contains(query),
                    active: self.is_active(&s.id),
                });
            }
            for child in self.native_children(s).iter().filter(|child| {
                !self.provider_cleared(&s.id, child)
                    || self
                        .view
                        .child
                        .as_ref()
                        .is_some_and(|t| t.parent == s.id && t.agent == child.id)
            }) {
                let kind = child_kind(child, connected, now);
                let title = [&child.description, &child.label]
                    .into_iter()
                    .find(|t| !t.is_empty())
                    .cloned()
                    .unwrap_or_default();
                found.push(Descendant {
                    row: SidebarRow::Provider {
                        parent: s.id.clone(),
                        child: Box::new(child.clone()),
                        depth: depth + 1,
                    },
                    kind,
                    loud: kind == StatusKind::NeedsYou || kind == StatusKind::NotResponding,
                    matches: !query.is_empty() && title.to_lowercase().contains(query),
                    active: self.view.child.as_ref().is_some_and(|t| {
                        t.parent == s.id
                            && t.agent == child.id
                            && self.navigation_selected.is_none()
                    }),
                });
            }
        }
        if found.is_empty() {
            return;
        }
        let open = self.shell.open_children.contains(&root.id);
        let (mut total, mut stalled, mut failed, mut working, mut listed) = (0, 0, 0, 0, 0);
        for d in &found {
            let own_row = open || d.active || d.matches || (d.loud && bucket == Bucket::NeedsYou);
            if own_row {
                listed += 1;
                out.push(ShellRow::Item {
                    row: d.row.clone(),
                    bucket,
                    tag: None,
                });
            }
            total += 1;
            match (&d.row, d.kind) {
                (_, StatusKind::NotResponding) => stalled += 1,
                (_, StatusKind::Failed) => failed += 1,
                (_, StatusKind::Working) => working += 1,
                _ => {}
            }
        }
        if listed == found.len() && !open {
            return;
        }
        if let Some(summary) = attention::children_summary(total, stalled, failed, working) {
            out.push(ShellRow::Children {
                parent: root.id.clone(),
                depth: members[0].1 + 1,
                summary,
                fault: stalled + failed > 0,
                open,
            });
        }
    }

    /// Sessions in the order the list shows them (what j/k, Enter in the
    /// search and `g n` walk): v2's grouped order, else the tree's.
    pub(super) fn nav_sessions(&self, cx: &App) -> Vec<usize> {
        if !self.design().is_v2() {
            return self.visible_sessions(cx);
        }
        self.shell_rows(cx)
            .into_iter()
            .filter_map(|row| match row {
                ShellRow::Item {
                    row: SidebarRow::Session { index, .. },
                    ..
                } => Some(index),
                _ => None,
            })
            .collect()
    }

    /// Where session `index` (into [`Self::nav_sessions`]) sits in the v2 list.
    pub(super) fn shell_position(&self, index: usize, cx: &App) -> usize {
        let order = self.nav_sessions(cx);
        self.shell_rows(cx)
            .iter()
            .position(|row| {
                matches!(row, ShellRow::Item { row: SidebarRow::Session { index: ix, .. }, .. }
                    if Some(ix) == order.get(index))
            })
            .unwrap_or(index)
    }

    /// Bring session `index` (into the shown order) into view in whichever
    /// list the design draws.
    pub(super) fn reveal_sidebar_row(&self, index: usize, cx: &App) {
        if self.design().is_v2() {
            self.shell
                .scroll
                .scroll_to_item(self.shell_position(index, cx));
        } else {
            self.sidebar_scroll.scroll_to_item(
                self.sidebar_session_position(index, cx),
                gpui::ScrollStrategy::Center,
            );
        }
    }

    /// Sessions waiting on the user (or stalled, or failing), among those
    /// listed.
    pub(super) fn needing_you(&self, cx: &App) -> Vec<usize> {
        let now = timing::now_ms();
        self.visible_sessions(cx)
            .into_iter()
            .filter(|&ix| self.session_bucket(&self.view.sessions[ix], now) == Bucket::NeedsYou)
            .collect()
    }

    /// `g n` / Alt+N: open the next session that needs you. (v1 keeps `g n`
    /// for Scratch.)
    pub(super) fn next_needs_you(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.design().is_v2() {
            self.open_feature(Screen::Scratch, window, cx);
            return;
        }
        // A search could hide the one waiting; the jump looks at everything.
        if !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let needing = self.needing_you(cx);
        let order = self.nav_sessions(cx);
        let current = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref())
            .and_then(|id| self.view.sessions.iter().position(|s| &s.id == id));
        let Some(next) = attention::next_needing(&order, &needing, current) else {
            self.extras.notice = "Nothing needs you right now.".into();
            cx.notify();
            return;
        };
        // Its group may be folded: open it.
        self.shell.folded[Bucket::NeedsYou.index()] = false;
        if let Some(position) = self.nav_sessions(cx).iter().position(|&ix| ix == next) {
            self.reveal_sidebar_row(position, cx);
        }
        self.open_listed_session(next, cx);
    }

    /// Open session `index` in the conversation.
    pub(super) fn open_listed_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let current = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref())
            .and_then(|id| self.view.sessions.iter().position(|s| &s.id == id));
        self.new_session = false;
        self.screen = Screen::Conversation;
        let id = self.view.sessions[index].id.clone();
        if Some(index) != current || self.view.child.is_some() {
            self.command(Command::Select(id), cx);
        }
        cx.notify();
    }

    fn toggle_group(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        let folded = &mut self.shell.folded[bucket.index()];
        *folded = !*folded;
        cx.notify();
    }

    fn single_height(&self) -> f32 {
        let t = self.tokens();
        if self.settings.sidebar_compact {
            t.layout.sidebar_row_compact
        } else {
            t.shell().row_single
        }
    }

    // ── The sidebar ─────────────────────────────────────────────────────

    /// The v2 sidebar: wordmark, search, the destinations, the grouped
    /// sessions and the account footer. Docked, or in the rail's flyout.
    pub(super) fn render_sidebar_v2(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let rows = self.shell_rows(cx);
        let query = self.query(cx);
        let filtered = !query.is_empty() || self.project_filter.is_some();
        let empty = !rows
            .iter()
            .any(|r| matches!(r, ShellRow::Item { .. } | ShellRow::AllClear));
        let header = chrome::drag_region(div())
            .debug_selector(|| "sidebar-drag-region".into())
            .h(px(t.layout.footer_height))
            .px(px(t.space.lg))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .child(wordmark(p))
            .child(div().flex_1())
            .child(div().occlude().child(self.sidebar_toggle_v2(cx)));
        let matches = (!query.is_empty()).then(|| self.match_count(&rows, &query));
        let search = div().px(px(t.space.md)).pb(px(t.space.sm)).child(
            div()
                .key_context("SessionSearch")
                .debug_selector(|| "sidebar-search-field".into())
                .h(px(t.layout.control_height_sm + t.space.xs))
                .px(px(t.space.md))
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .rounded(px(t.radius.control))
                .bg(primitives::tonal(&p, t.wash.rest))
                .child(
                    Icon::new(IconName::Search)
                        .size(px(t.icon.sm))
                        .text_color(rgb(p.roles.text_tertiary)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.search).appearance(false)),
                )
                .when_some(matches, |d, n| {
                    d.child(
                        div()
                            .debug_selector(|| "sidebar-search-count".into())
                            .flex_shrink_0()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(match n {
                                0 => "No matches".to_owned(),
                                1 => "1 match".to_owned(),
                                n => format!("{n} matches"),
                            }),
                    )
                }),
        );
        let list = div()
            .id("sessions")
            .debug_selector(|| "sidebar-sessions".into())
            .track_scroll(&self.shell.scroll)
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .px(px(t.space.sm))
            .pb(px(t.space.lg))
            .children(
                rows.iter()
                    .enumerate()
                    .map(|(ix, row)| self.render_shell_row(row, ix, &query, cx)),
            );
        div()
            .id("sidebar")
            .relative()
            .w(px(wks_native::navigation::sidebar_width(
                self.settings.sidebar_width,
                unzoom(window.viewport_size().width),
            )))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(t.radius.panel))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .debug_selector(|| "session-sidebar".into())
            .child(header)
            .child(search)
            .child(self.render_destinations(cx))
            .children(self.render_cleanup_banner(cx))
            .when(self.view.power_paused, |d| {
                d.child(
                    div()
                        .px(px(t.space.lg))
                        .pb(px(t.space.md))
                        .flex()
                        .flex_col()
                        .gap(px(t.space.sm))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(self.view.notice.clone())
                        .when(self.view.can_resume_power_pause, |d| {
                            d.child(
                                primitives::button(
                                    "resume-power-pause",
                                    "Reconnect",
                                    Some(IconName::Redo),
                                    ButtonVariant::Secondary,
                                    ButtonSize::Small,
                                    true,
                                    &p,
                                )
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.command(Command::Refresh, cx)
                                    }),
                                ),
                            )
                        }),
                )
            })
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(self.render_ui_notice(cx))
            })
            .children(self.render_filter_v2(cx))
            .when(empty, |d| d.child(self.render_list_empty_v2(filtered, cx)))
            .child(list)
            .children(self.render_finish_setup(cx))
            .child(self.render_footer_v2(cx))
            .child(self.sidebar_resize_handle(cx))
            .into_any_element()
    }

    /// Rows whose title matches the search (sessions and subagents).
    fn match_count(&self, rows: &[ShellRow], query: &str) -> usize {
        let query = query.to_lowercase();
        rows.iter()
            .filter(|row| match row {
                ShellRow::Item {
                    row: SidebarRow::Session { index, .. },
                    ..
                } => self
                    .session_title(&self.view.sessions[*index])
                    .to_lowercase()
                    .contains(&query),
                ShellRow::Item {
                    row: SidebarRow::Provider { child, .. },
                    ..
                } => [&child.description, &child.label]
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query)),
                _ => false,
            })
            .count()
    }

    /// The global destinations at the top of the sidebar, still labelled
    /// but in a two-column grid (r3-shell: three rows, not six), shortcuts
    /// in their tooltips.
    fn render_destinations(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let vim = self.vim();
        let pending = self.pending_total();
        let new_session = !self.demo;
        div()
            .debug_selector(|| "sidebar-destinations".into())
            .px(px(t.space.sm))
            .pb(px(t.space.sm))
            .flex()
            .flex_wrap()
            .gap_y(px(t.space.xxs))
            .child(
                self.destination(
                    "new-session",
                    IconName::Plus,
                    "New session",
                    Some(if cfg!(target_os = "macos") {
                        "⌘ N"
                    } else {
                        "Ctrl N"
                    }),
                    false,
                    new_session,
                    None,
                )
                .debug_selector(|| "new-session-button".into())
                .when(new_session, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx)))
                }),
            )
            .child(self.destination_to(
                "nav-projects",
                IconName::Folder,
                "Projects",
                Some(if cfg!(target_os = "macos") {
                    "⌘ P"
                } else {
                    "Ctrl P"
                }),
                Screen::Projects,
                None,
                cx,
            ))
            .child(self.destination_to(
                "nav-jobs",
                IconName::Calendar,
                "Jobs",
                vim.then_some("g j"),
                Screen::Jobs,
                None,
                cx,
            ))
            .child(self.destination_to(
                "nav-hosts",
                IconName::LayoutDashboard,
                "Machines",
                vim.then_some("g o"),
                Screen::Hosts,
                None,
                cx,
            ))
            .child(self.destination_to(
                "nav-scratch",
                IconName::FolderClosed,
                "Scratch",
                vim.then_some("g x"),
                Screen::Scratch,
                (pending > 0).then(|| {
                    div()
                        .debug_selector(|| "scratch-pending-badge".into())
                        .child(primitives::count_badge(
                            primitives::count_text(pending),
                            primitives::CountTone::Neutral,
                            &p,
                        ))
                        .into_any_element()
                }),
                cx,
            ))
            .child(self.destination_to(
                "nav-settings",
                IconName::Settings,
                "Settings",
                Some(if cfg!(target_os = "macos") {
                    "⌘ ,"
                } else {
                    "Ctrl ,"
                }),
                Screen::Settings,
                self.update_available().then(|| {
                    div()
                        .debug_selector(|| "update-pill".into())
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.accent))
                        .child(
                            div()
                                .size(px(t.layout.status_dot))
                                .rounded(px(t.radius.full))
                                .bg(rgb(p.accent)),
                        )
                        .child("Update")
                        .into_any_element()
                }),
                cx,
            ))
    }

    /// One destination cell: icon, label, then a badge; the shortcut in
    /// its tooltip. Half the sidebar's width.
    #[allow(clippy::too_many_arguments)]
    fn destination(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        shortcut: Option<&'static str>,
        open: bool,
        enabled: bool,
        badge: Option<AnyElement>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        chrome::interactive_control(div().id(id), p, enabled)
            .w(gpui::relative(0.5))
            .min_w_0()
            .h(px(t.layout.control_height_sm))
            .px(px(t.space.md))
            .rounded(px(t.radius.control))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .text_size(px(t.text.body))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(if open { p.text } else { p.roles.text_secondary }))
            .when(open, |d| d.bg(rgb(p.selected)))
            .when(!open && enabled, |d| {
                d.hover(move |s| s.bg(rgb(p.roles.hover)).text_color(rgb(p.text)))
            })
            .child(Icon::new(icon).size(px(t.icon.md)).text_color(rgb(if open {
                p.text
            } else {
                p.roles.text_tertiary
            })))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(badge)
            .tooltip(tooltip_view(
                if id == "nav-scratch" && self.pending_total() > 0 {
                    "Scratch files · some are waiting for you to run them"
                } else {
                    label
                },
                shortcut,
                &p,
            ))
    }

    /// A destination that opens a page, highlighted while it is open.
    #[allow(clippy::too_many_arguments)]
    fn destination_to(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        shortcut: Option<&'static str>,
        screen: Screen,
        badge: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = !self.new_session && self.screen == screen;
        self.destination(id, icon, label, shortcut, open, true, badge)
            .debug_selector(move || id.into())
            .on_click(cx.listener(move |this, _, window, cx| {
                if matches!(screen, Screen::Projects | Screen::Settings) {
                    this.show_screen(screen, window, cx)
                } else {
                    this.open_feature(screen, window, cx)
                }
            }))
    }

    /// A rail icon that opens a page, highlighted while it is open.
    fn nav_icon(
        &self,
        id: &'static str,
        icon: IconName,
        tip: &'static str,
        shortcut: Option<&'static str>,
        screen: Screen,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let open = !self.new_session && self.screen == screen;
        primitives::icon_button_with_shortcut(
            id,
            icon,
            tip,
            shortcut,
            ButtonVariant::Tertiary,
            true,
            &p,
        )
        .debug_selector(move || id.into())
        .when(open, |d| d.bg(rgb(p.selected)).text_color(rgb(p.text)))
        .on_click(cx.listener(move |this, _, window, cx| {
            if matches!(screen, Screen::Projects | Screen::Settings) {
                this.show_screen(screen, window, cx)
            } else {
                this.open_feature(screen, window, cx)
            }
        }))
    }

    fn sidebar_toggle_v2(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let (icon, tip) = if self.sidebar_collapsed {
            (IconName::PanelLeftOpen, "Expand sidebar")
        } else {
            (IconName::PanelLeftClose, "Collapse sidebar")
        };
        primitives::icon_button(
            "toggle-sidebar",
            icon,
            tip,
            ButtonVariant::Tertiary,
            true,
            &p,
        )
        .debug_selector(|| "sidebar-toggle".into())
        .on_click(cx.listener(|this, _, _, cx| {
            this.sidebar_collapsed = !this.sidebar_collapsed;
            cx.notify();
        }))
    }

    /// The filtered project, above the list: its mark and name, waiting
    /// scratch, a new session there, and the way back to every project.
    fn render_filter_v2(&self, cx: &mut Context<Self>) -> Option<Div> {
        let filter = self.project_filter.clone()?;
        let p = self.palette();
        let t = p.tokens;
        let group = self.filter_group();
        let folders = self.filter_folders();
        let (known, title) = match &group {
            Some(group) => (Some(group.as_known()), group.name.clone()),
            None => (
                self.known_projects()
                    .into_iter()
                    .find(|k| wks_native::projects::same_dir(&k.path, &filter)),
                self.project_name(&filter),
            ),
        };
        let mark_path = known.as_ref().map_or(filter.clone(), |k| k.path.clone());
        let pending = self.pending_in(&folders);
        let spawn_in = folders.first().cloned();
        let can_spawn = self.view.connected && !self.demo && spawn_in.is_some();
        Some(
            div()
                .debug_selector(|| "sidebar-project-filter".into())
                .mx(px(t.space.md))
                .mb(px(t.space.md))
                .px(px(t.space.md))
                .h(px(t.layout.control_height))
                .rounded(px(t.radius.control))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.border))
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .child(self.project_mark(known.as_ref(), &mark_path, t.icon.lg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(p.text))
                        .child(title),
                )
                .when(pending > 0, |d| {
                    let target = filter.clone();
                    d.child(
                        div()
                            .id("sidebar-filter-pending")
                            .debug_selector(|| "sidebar-filter-pending".into())
                            .cursor_pointer()
                            .tooltip(tooltip_view(
                                "Scratch files waiting for you to run them",
                                None,
                                &p,
                            ))
                            .child(primitives::pill(
                                format!("{pending} waiting"),
                                Intent::Neutral,
                                &p,
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.extras.scratch_preset = Some(target.clone());
                                this.open_feature(Screen::Scratch, window, cx)
                            })),
                    )
                })
                .child(
                    primitives::icon_button(
                        "new-agent-in-filter",
                        IconName::Plus,
                        "New session in this project",
                        ButtonVariant::Tertiary,
                        can_spawn,
                        &p,
                    )
                    .debug_selector(|| "new-agent-in-filter".into())
                    .when_some(spawn_in.filter(|_| can_spawn), |d, path| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            let filter = this.project_filter.clone();
                            this.new_agent_in_project(path.clone(), window, cx);
                            this.project_filter = filter;
                        }))
                    }),
                )
                .child(
                    primitives::icon_button(
                        "all-projects",
                        IconName::Close,
                        "Show every project",
                        ButtonVariant::Tertiary,
                        true,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_filter = None;
                        cx.notify();
                    })),
                ),
        )
    }

    fn render_list_empty_v2(&self, filtered: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let (title, body) = if !self.view.connected {
            (
                self.connection_copy().label,
                self.connection_copy().description,
            )
        } else if self.view.sessions_loading || self.archive_loading() {
            ("Loading sessions…", "Checking your workspace for sessions.")
        } else if filtered {
            (
                "No matching sessions",
                "Try another search or clear the filters.",
            )
        } else {
            ("No sessions yet", "Start a new session to begin.")
        };
        div()
            .px(px(t.space.xl))
            .py(px(t.space.lg))
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(
                div()
                    .text_size(px(t.text.body))
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(p.text))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(body),
            )
            .when(filtered, |d| {
                d.child(
                    primitives::button(
                        "clear-session-filters",
                        "Clear filters",
                        None,
                        ButtonVariant::Secondary,
                        primitives::ButtonSize::Small,
                        true,
                        &p,
                    )
                    .debug_selector(|| "clear-session-filters".into())
                    .mt(px(t.space.md))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.project_filter = None;
                        this.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
                )
            })
    }

    fn render_shell_row(
        &self,
        row: &ShellRow,
        ix: usize,
        query: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match row {
            ShellRow::Group {
                bucket,
                count,
                folded,
            } => self.render_group_header(*bucket, *count, *folded, cx),
            ShellRow::Item {
                row: SidebarRow::Session { index, depth },
                bucket,
                tag,
            } => self.render_session_v2(*index, *depth, *bucket, tag.clone(), ix, query, cx),
            ShellRow::Item {
                row:
                    SidebarRow::Provider {
                        parent,
                        child,
                        depth,
                    },
                ..
            } => self.render_child_v2(parent, child, *depth, ix, query, cx),
            ShellRow::Children {
                parent,
                depth,
                summary,
                fault,
                open,
            } => self.render_children_line(parent, *depth, summary, *fault, *open, ix, cx),
            ShellRow::AllClear => self.render_all_clear(),
            ShellRow::ShowMore { hidden } => self.render_show_more(*hidden, cx),
        }
    }

    fn render_group_header(
        &self,
        bucket: Bucket,
        count: usize,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let needs = bucket == Bucket::NeedsYou && count > 0;
        let slug = bucket.slug();
        div()
            .h(px(t.shell().row_single + t.space.md))
            .px(px(t.space.xs))
            .pb(px(t.space.xxs))
            .flex()
            .items_end()
            .gap(px(t.space.sm))
            .child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!("sidebar-group-{slug}"))),
                    p,
                    true,
                )
                .debug_selector(move || format!("sidebar-group-{slug}"))
                .h(px(t.layout.control_height_sm - t.space.xs))
                .px(px(t.space.xs))
                .rounded(px(t.radius.sm))
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .text_size(px(t.text.meta))
                .font_weight(FontWeight(t.weight.semibold))
                .text_color(rgb(if needs {
                    p.warning
                } else {
                    p.roles.text_secondary
                }))
                .hover(move |s| s.bg(rgb(p.roles.hover)))
                .tooltip(tooltip_view(
                    if folded {
                        "Show this group"
                    } else {
                        "Fold this group"
                    },
                    None,
                    &p,
                ))
                .child(bucket.label())
                .when(!needs && count > 0, |d| {
                    d.child(
                        div()
                            .font_weight(FontWeight(t.weight.regular))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(count.to_string()),
                    )
                })
                .group(SharedString::from(format!("group-{slug}")))
                .child(
                    div()
                        .when(!folded, |d| {
                            d.invisible()
                                .group_hover(SharedString::from(format!("group-{slug}")), |s| {
                                    s.visible()
                                })
                        })
                        .child(
                            Icon::new(if folded {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(t.icon.xs))
                            .text_color(rgb(p.roles.text_tertiary)),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(bucket, cx))),
            )
            .when(needs, |d| {
                d.child(
                    chrome::interactive_control(div().id("sidebar-next-needs-you"), p, true)
                        .debug_selector(|| "sidebar-next-needs-you".into())
                        .mb(px(t.space.xs))
                        .rounded(px(t.radius.full))
                        .tooltip(tooltip_view(
                            "Open the next session that needs you",
                            Some(if self.vim() { "g n" } else { "Alt N" }),
                            &p,
                        ))
                        .child(primitives::count_badge(
                            primitives::count_text(count),
                            primitives::CountTone::NeedsYou,
                            &p,
                        ))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.next_needs_you(window, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    /// Needs you, empty: said once, calmly.
    fn render_all_clear(&self) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        div()
            .debug_selector(|| "sidebar-all-clear".into())
            .h(px(t.shell().row_single))
            .px(px(t.space.md))
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(
                div()
                    .size(px(t.layout.status_box))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::Check).size(px(t.icon.xs))),
            )
            .child("All clear · nothing is waiting on you")
            .into_any_element()
    }

    fn render_show_more(&self, hidden: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        div()
            .h(px(t.shell().row_single))
            .flex()
            .items_center()
            .child(
                chrome::interactive_control(div().id("sidebar-show-more"), p, true)
                    .debug_selector(|| "sidebar-show-more".into())
                    .h(px(t.layout.control_height_sm - t.space.xs))
                    .px(px(t.space.md))
                    .rounded(px(t.radius.control))
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .text_size(px(t.text.meta))
                    .font_weight(FontWeight(t.weight.medium))
                    .text_color(rgb(p.roles.text_secondary))
                    .hover(move |s| s.bg(rgb(p.roles.hover)).text_color(rgb(p.text)))
                    .child(
                        div()
                            .size(px(t.layout.status_box))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::ChevronDown).size(px(t.icon.xs))),
                    )
                    .child(format!("Show {hidden} more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shell.idle_all = true;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The left edge of a row at `depth`, and the 1px tree guide that ties a
    /// nested row to its parent's status mark.
    fn row_frame(&self, depth: usize, height: f32) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let indent = t.space.xl;
        let depth = depth.min(6) as f32;
        div()
            .relative()
            .h(px(height))
            .pl(px(depth * indent))
            .py(px(t.stroke.hairline))
            .when(depth > 0., |d| {
                // Under the parent's status box: its row's left padding,
                // the control's focus border and half the box.
                let x =
                    (depth - 1.) * indent + t.space.md + t.stroke.focus + t.layout.status_box / 2.;
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(x))
                        .w(px(t.stroke.hairline))
                        .bg(rgb(p.border)),
                )
            })
    }

    /// A title in the row's type, matches highlighted, and its repeat tag.
    fn row_title(&self, title: String, tag: Option<String>, strong: bool, query: &str) -> Div {
        let p = self.palette();
        let t = p.tokens;
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.xs))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink()
                    .truncate()
                    .text_size(px(t.text.body))
                    .line_height(px(t.text.body + t.space.xs))
                    .font_weight(FontWeight(if strong {
                        t.weight.semibold
                    } else {
                        t.weight.medium
                    }))
                    .child(highlighted(title, query, &p)),
            )
            .when_some(tag, |d, tag| {
                d.child(
                    div()
                        .debug_selector(|| "sidebar-row-tag".into())
                        .flex_shrink_0()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(tag),
                )
            })
    }

    /// A row's right edge: its age at rest (tabular), its actions on hover
    /// (hidden actions take no clicks).
    fn row_trailing(
        &self,
        group: SharedString,
        age: Option<String>,
        actions: Vec<AnyElement>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let slot = (t.icon.md + t.space.sm) * actions.len() as f32;
        div()
            .relative()
            .flex_shrink_0()
            .min_w(px(slot))
            .h(px(t.icon.md + t.space.sm))
            .flex()
            .items_center()
            .justify_end()
            .when_some(age, |d, age| {
                d.child(
                    div()
                        .text_size(px(t.text.meta))
                        .map(tabular)
                        .text_color(rgb(p.roles.text_tertiary))
                        .when(!actions.is_empty(), |d| {
                            d.group_hover(group.clone(), |s| s.invisible())
                        })
                        .child(age),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .children(actions),
            )
    }

    /// The inline action of a waiting row: Review (a tool waits for
    /// permission: it opens the session at the request, never allowing an
    /// unseen command blind), Answer or Open. Shown on hover or selection.
    fn inbox_button(
        &self,
        index: usize,
        reason: &Reason,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let action = reason.action;
        let tip = match action {
            InboxAction::Allow => format!("{} · review the request in its session", reason.text),
            InboxAction::Answer => "Open the session at its question".to_owned(),
            InboxAction::Open => "Open the session".to_owned(),
        };
        primitives::button(
            SharedString::from(format!("sidebar-inbox-{ix}")),
            action.label(),
            None,
            ButtonVariant::Secondary,
            ButtonSize::Small,
            true,
            &p,
        )
        .debug_selector(move || format!("sidebar-inbox-action-{ix}"))
        .h(px(t.icon.lg + t.space.xxs))
        .px(px(t.space.md))
        .text_size(px(t.text.meta))
        .flex_shrink_0()
        .tooltip(tooltip_view(tip, None, &p))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_listed_session(index, cx);
        }))
    }

    /// How long a working session has been at its turn: the turn clock
    /// when this window keeps one, else since it started.
    fn working_elapsed(&self, session: &Session, now: i64) -> Option<String> {
        self.turn_clocks
            .get(&session.id)
            .and_then(|clock| clock.elapsed_label(now))
            .or_else(|| {
                session
                    .telemetry
                    .started_at_ms
                    .map(|at| wks_native::format::age(at, now))
            })
    }

    /// One session row (R3.6): one template for every group. Line one is
    /// the status glyph (none when Idle), the title (it truncates last) and
    /// the time (elapsed while Working, else the age); line two is why it
    /// waits, or the project and what it is doing. Compact drops line two
    /// everywhere. Needs you rows carry the one attention treatment.
    #[allow(clippy::too_many_arguments)]
    fn render_session_v2(
        &self,
        index: usize,
        depth: usize,
        bucket: Bucket,
        tag: Option<String>,
        ix: usize,
        query: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let compact = self.settings.sidebar_compact;
        let now = timing::now_ms();
        let session = &self.view.sessions[index];
        let id = session.id.clone();
        let active = self.is_active(&id);
        let title = self.session_title(session);
        let kind = self.session_kind(session, now);
        let project = self.project_name(&session.cwd);
        let reason = (bucket == Bucket::NeedsYou)
            .then(|| self.inbox_reason(session, now))
            .flatten();
        let working = kind == StatusKind::Working;
        let two_lines = !compact && depth == 0;
        let job = !session.job_id.is_empty();
        let cold =
            super::cache::cold(session, now).map(|cold| super::cache::marker_tooltip(cold, now));
        let time = if working {
            self.working_elapsed(session, now)
        } else {
            session
                .telemetry
                .last_activity_ms
                .map(|at| wks_native::format::age(at, now))
        };
        let (status_label, _) = if self.view.connected {
            self.status_of(session, p)
        } else {
            ("Offline", p.muted)
        };
        let details = {
            let mut lines = vec![title.clone(), session.cwd.clone()];
            if let Some(reason) = &reason {
                lines.push(reason.text.clone());
            }
            let provider = match session.provider_id() {
                "claude" => "Claude",
                "codex" => "Codex",
                "" => "Agent",
                other => other,
            };
            lines.push(format!(
                "{provider} · {} · {}",
                wks_native::format::model_name(&session.display_model()),
                match kind {
                    StatusKind::NotResponding | StatusKind::NeedsYou | StatusKind::Working => {
                        kind.label()
                    }
                    _ => status_label,
                }
            ));
            if job {
                lines.push("Started by a job · its runs are under Jobs → Runs".into());
            }
            if let Some(cold) = &cold {
                lines.push(cold.clone());
            }
            lines.join("\n") + super::sidebar::child_limit_note(session)
        };
        // Nested, finished children offer Clear (this device's view) in place
        // of Archive; a parent with finished children can clear them all.
        let finished_child = depth > 0 && wks_native::child_agents::session_finished(session);
        let clearable = self.clearable_children(session);
        let group: SharedString = format!("sidebar-row-{ix}").into();
        let mut actions: Vec<AnyElement> = Vec::new();
        if !clearable.is_empty() {
            actions.push(
                row_action(
                    format!("clear-finished-{}", session.id).into(),
                    IconName::EyeOff,
                    super::sidebar::CLEAR_FINISHED,
                    &p,
                )
                .debug_selector(move || format!("sidebar-clear-finished-{ix}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.clear_children(clearable.clone(), cx);
                }))
                .into_any_element(),
            );
        }
        actions.push(if finished_child {
            let clear = super::sidebar::session_clear(session);
            row_action(
                format!("clear-child-{}", session.id).into(),
                IconName::EyeOff,
                super::sidebar::CLEAR_CHILD,
                &p,
            )
            .debug_selector(move || format!("sidebar-clear-{ix}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.clear_children(vec![clear.clone()], cx);
            }))
            .into_any_element()
        } else {
            let target = id.clone();
            row_action(
                format!("archive-sidebar-{}", session.id).into(),
                IconName::Inbox,
                "Archive · hides it in every client, keeps it running · restore from Session history → Archived",
                &p,
            )
            .debug_selector(move || format!("sidebar-archive-{ix}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_archive(&target, cx);
            }))
            .into_any_element()
        });
        // The verb chip takes the archive's place on a waiting row: on
        // hover, or while the row is selected.
        let verb = reason
            .as_ref()
            .map(|reason| self.inbox_button(index, reason, ix, cx).into_any_element());
        let trailing = match verb {
            Some(verb) => self.row_trailing_verb(group.clone(), time, verb, active),
            None => self.row_trailing(group.clone(), time, actions),
        };
        let title_el = self.row_title(title, tag, active, query).text_color(rgb(
            if session.stopped() && !self.paused(session) {
                p.roles.text_secondary
            } else {
                p.text
            },
        ));
        let marks = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(t.space.xs))
            .when(job, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("sidebar-job-{ix}"))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child("Job"),
                )
            })
            .when(cold.is_some(), |d| {
                d.child(
                    super::cache::marker(p, t.icon.xs, false)
                        .debug_selector(move || format!("sidebar-cold-{ix}")),
                )
            });
        let first = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .child(status_indicator(kind, &p))
            .child(title_el)
            .child(marks)
            .child(trailing);
        // Line two, under the title: why it waits, or where and what.
        let second = two_lines.then(|| {
            let line = div()
                .w_full()
                .min_w_0()
                .pl(px(t.layout.status_box + t.space.sm))
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .text_size(px(t.text.meta))
                .text_color(rgb(p.roles.text_tertiary));
            match &reason {
                Some(reason) => line.child(
                    div()
                        .debug_selector(move || format!("sidebar-reason-{ix}"))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(if reason.fault {
                            p.error
                        } else {
                            p.roles.text_secondary
                        }))
                        .child(reason.text.clone()),
                ),
                None => {
                    let activity = working
                        .then(|| {
                            [
                                session.telemetry.last_tool_name.trim(),
                                session.telemetry.last_tool_summary.trim(),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" ")
                        })
                        .filter(|a| !a.is_empty());
                    line.child(
                        div()
                            .debug_selector(move || format!("sidebar-project-{ix}"))
                            .min_w_0()
                            .flex_shrink()
                            .when(activity.is_some(), |d| d.max_w(gpui::relative(0.5)))
                            .truncate()
                            .child(project.clone()),
                    )
                    .when_some(activity, |d, activity| {
                        d.child("·").child(
                            div()
                                .debug_selector(move || format!("sidebar-activity-{ix}"))
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .child(activity),
                        )
                    })
                }
            }
        });
        let height = if two_lines {
            t.shell().row_double
        } else {
            self.single_height()
        };
        let attention = reason.is_some();
        let bar = t.signals().attention_bar;
        let control = chrome::interactive_control(
            div().id(SharedString::from(format!("session-{}", session.id))),
            p,
            true,
        )
        .debug_selector(move || format!("sidebar-session-{ix}"))
        .group(group)
        .relative()
        .size_full()
        .pl(px(if attention {
            t.space.md + bar
        } else {
            t.space.md
        }))
        .pr(px(t.space.md))
        .rounded(px(t.radius.control))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(t.space.xxs))
        .when(active, |d| d.bg(rgb(p.selected)))
        .when(!active, |d| d.hover(move |s| s.bg(rgb(p.roles.hover))))
        .tooltip(tooltip_view(details, None, &p))
        .child(first)
        .children(second)
        .on_click(cx.listener(move |this, _, _, cx| this.open_listed_session(index, cx)));
        // R3.1: one attention treatment, the bar and a neutral edge.
        let control = if attention {
            primitives::attention_frame(t.radius.control, &p)
                .size_full()
                .child(control)
                .into_any_element()
        } else {
            control.into_any_element()
        };
        self.row_frame(depth, height)
            .child(control)
            .into_any_element()
    }

    /// A waiting row's right edge: its time at rest and its verb chip on
    /// hover; while the row is selected, the chip alone.
    fn row_trailing_verb(
        &self,
        group: SharedString,
        time: Option<String>,
        verb: AnyElement,
        active: bool,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let slot = div()
            .flex_shrink_0()
            .h(px(t.icon.lg + t.space.xxs))
            .flex()
            .items_center()
            .justify_end();
        if active {
            return slot.child(verb);
        }
        slot.relative()
            .when_some(time, |d, time| {
                d.child(
                    div()
                        .text_size(px(t.text.meta))
                        .map(tabular)
                        .text_color(rgb(p.roles.text_tertiary))
                        .group_hover(group.clone(), |s| s.invisible())
                        .child(time),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .child(verb),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_child_v2(
        &self,
        parent: &str,
        child: &ChildAgent,
        depth: usize,
        ix: usize,
        query: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let now = timing::now_ms();
        let connected = self.view.connected;
        let viewing = self.view.child.as_ref().is_some_and(|v| {
            v.parent == parent && v.agent == child.id && self.navigation_selected.is_none()
        });
        let title = [&child.description, &child.label]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "Subagent".into());
        let kind = child_kind(child, connected, now);
        let stalled = connected && attention::child_stalled(child, now);
        let model = wks_native::format::model_name(&child.model);
        let group: SharedString = format!("sidebar-row-{ix}").into();
        let (parent, agent) = (parent.to_owned(), child.id.clone());
        let mut actions = Vec::new();
        if child.settled() {
            let (parent, child) = (parent.clone(), child.clone());
            actions.push(
                row_action(
                    format!("clear-native-{parent}-{}", child.id).into(),
                    IconName::EyeOff,
                    super::sidebar::CLEAR_SUBAGENT,
                    &p,
                )
                .debug_selector(move || format!("sidebar-clear-provider-{ix}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.clear_children(vec![super::sidebar::provider_clear(&parent, &child)], cx);
                }))
                .into_any_element(),
            );
        }
        let note = if stalled {
            let quiet = child
                .telemetry
                .last_activity_ms
                .or(child.telemetry.started_at_ms)
                .map(|at| attention::quiet_for(now - at))
                .unwrap_or_default();
            Some((format!("Not responding · {quiet}"), p.error))
        } else {
            None
        };
        let meta = div()
            .debug_selector(move || format!("sidebar-child-model-{ix}"))
            .min_w_0()
            .when(note.is_some(), |d| d.flex_shrink_0())
            .when(note.is_none(), |d| {
                d.flex_shrink().max_w(gpui::relative(0.45))
            })
            .truncate()
            .text_size(px(t.text.meta))
            .map(|d| match &note {
                Some((note, color)) => d.text_color(rgb(*color)).child(note.clone()),
                None => d
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(model.clone()),
            });
        let title_el =
            self.row_title(title, None, viewing, query)
                .text_color(rgb(if child.settled() {
                    p.roles.text_secondary
                } else {
                    p.text
                }));
        let trailing = self.row_trailing(group.clone(), None, actions);
        let control = chrome::interactive_control(
            div().id(SharedString::from(format!(
                "sidebar-native-{parent}-{agent}"
            ))),
            p,
            connected,
        )
        .debug_selector(move || format!("sidebar-provider-{ix}"))
        .group(group)
        .size_full()
        .px(px(t.space.md))
        .rounded(px(t.radius.control))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .when(viewing, |d| d.bg(rgb(p.selected)))
        .when(!viewing && connected, |d| {
            d.hover(move |s| s.bg(rgb(p.roles.hover)))
        })
        .tooltip(tooltip_view(
            format!("Subagent · {model} · open its conversation"),
            None,
            &p,
        ))
        .child(status_indicator(kind, &p))
        .child(title_el)
        .child(meta)
        .child(trailing)
        .when(connected, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                this.open_sidebar_child(&parent, &agent, cx);
            }))
        });
        self.row_frame(depth, self.single_height())
            .child(control)
            .into_any_element()
    }

    /// "4 subagents · 1 not responding": a tree's subagents in one line; a click
    /// lists them one by one (and folds them back).
    #[allow(clippy::too_many_arguments)]
    fn render_children_line(
        &self,
        parent: &str,
        depth: usize,
        summary: &str,
        fault: bool,
        open: bool,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let parent = parent.to_owned();
        let (lead, rest) = summary
            .split_once(" · ")
            .map_or((summary.to_owned(), None), |(a, b)| {
                (a.to_owned(), Some(b.to_owned()))
            });
        let control = chrome::interactive_control(
            div().id(SharedString::from(format!("sidebar-children-{parent}"))),
            p,
            true,
        )
        .debug_selector(move || format!("sidebar-children-{ix}"))
        .size_full()
        .px(px(t.space.md))
        .rounded(px(t.radius.control))
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .text_size(px(t.text.meta))
        .text_color(rgb(p.roles.text_tertiary))
        .hover(move |s| {
            s.bg(rgb(p.roles.hover))
                .text_color(rgb(p.roles.text_secondary))
        })
        .tooltip(tooltip_view(
            if open {
                "Fold the subagents into one line"
            } else {
                "List the subagents"
            },
            None,
            &p,
        ))
        .child(
            div()
                .size(px(t.layout.status_box))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(t.icon.xs)),
                ),
        )
        .child(div().flex_shrink_0().child(lead))
        .when_some(rest, |d, rest| {
            d.child("·").child(
                div()
                    .min_w_0()
                    .truncate()
                    .when(fault, |d| d.text_color(rgb(p.error)))
                    .child(rest),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.shell.open_children.remove(&parent) {
                this.shell.open_children.insert(parent.clone());
            }
            cx.notify();
        }));
        self.row_frame(depth, self.single_height())
            .child(control)
            .into_any_element()
    }

    /// Install and sign-in prompts folded into one dismissible line, only
    /// while something is truly pending.
    fn render_finish_setup(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.shell.setup_dismissed {
            return None;
        }
        // R2.3: the one setup source the welcome and Setup read too.
        let notes: Vec<String> = [("claude", "Claude Code"), ("codex", "Codex")]
            .into_iter()
            .filter_map(|(id, name)| match self.agent_setup(id) {
                super::states::AgentSetup::Missing => Some(format!("Install {name}")),
                super::states::AgentSetup::NeedsSignIn => Some(format!("Sign in to {name}")),
                _ => None,
            })
            .collect();
        if notes.is_empty() {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let tip = notes.join(" · ");
        Some(
            div()
                .px(px(t.space.sm))
                .pb(px(t.space.xs))
                .flex()
                .items_center()
                .gap(px(t.space.xxs))
                .child(
                    primitives::list_row("sidebar-finish-setup", false, true, &p)
                        .debug_selector(|| "sidebar-finish-setup".into())
                        .flex_1()
                        .min_h(px(t.shell().nav_row))
                        .text_size(px(t.text.body))
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(p.roles.text_secondary))
                        .tooltip(tooltip_view(tip, None, &p))
                        .child(
                            Icon::new(IconName::CircleUser)
                                .size(px(t.icon.md))
                                .text_color(rgb(p.roles.text_tertiary)),
                        )
                        .child(div().flex_1().child("Finish setup"))
                        .child(primitives::count_badge(
                            primitives::count_text(notes.len()),
                            primitives::CountTone::Neutral,
                            &p,
                        ))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_feature(Screen::Setup, window, cx)
                        })),
                )
                .child(
                    primitives::icon_button(
                        "sidebar-finish-setup-dismiss",
                        IconName::Close,
                        "Hide until next launch",
                        ButtonVariant::Tertiary,
                        true,
                        &p,
                    )
                    .debug_selector(|| "sidebar-finish-setup-dismiss".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shell.setup_dismissed = true;
                        cx.notify();
                    })),
                ),
        )
    }

    /// The footer: account and usage only ("Claude 42%" with a meter) and
    /// the connection when it is not live. r3-shell: no modal-key label.
    fn render_footer_v2(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        div()
            .debug_selector(|| "sidebar-footer".into())
            .h(px(t.layout.footer_height))
            .flex_shrink_0()
            .px(px(t.space.sm))
            .border_t(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .map(|d| match self.footer_connection_label() {
                Some(label) => d.child(
                    div()
                        .debug_selector(|| "sidebar-connection-status".into())
                        .px(px(t.space.sm))
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .when(!self.view.connected, |d| {
                            d.child(status_indicator(StatusKind::Failed, &p))
                        })
                        .child(label),
                ),
                None => d.children(self.render_usage_compact(cx)),
            })
            .child(div().flex_1().min_w_0())
    }

    fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        div()
            .id("sidebar-resize")
            .debug_selector(|| "sidebar-resize".into())
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(t.space.sm))
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .hover(move |s| s.bg(rgb(p.border)))
            .tooltip(tooltip_view(
                "Drag to resize · double-click to reset",
                None,
                &p,
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if event.click_count == 2 {
                        this.settings.sidebar_width = this.tokens().layout.sidebar_width;
                        this.sidebar_drag = None;
                        this.save_settings(cx);
                    } else {
                        let width = wks_native::navigation::sidebar_width(
                            this.settings.sidebar_width,
                            unzoom(window.viewport_size().width),
                        );
                        this.sidebar_drag = Some((event.position.x, width));
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .child(self.sidebar_drag_watch(cx))
    }

    // ── The rail ────────────────────────────────────────────────────────

    /// The collapsed sidebar: only what needs you or is working, as
    /// status-ringed avatar tiles (attention first), "+n" for the rest, and
    /// the destinations as icons.
    pub(super) fn render_rail_v2(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let now = timing::now_ms();
        let order = self.nav_sessions(cx);
        let live: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&ix| {
                let s = &self.view.sessions[ix];
                s.parent_session_id.is_empty()
                    || !self
                        .view
                        .sessions
                        .iter()
                        .any(|parent| parent.id == s.parent_session_id)
            })
            .filter(|&ix| {
                matches!(
                    self.session_bucket(&self.view.sessions[ix], now),
                    Bucket::NeedsYou | Bucket::Working
                )
            })
            .collect();
        let limit = t.shell().rail_visible;
        let shown: Vec<usize> = live.iter().copied().take(limit).collect();
        let rest = self.visible_sessions(cx).len().saturating_sub(shown.len());
        let vim = self.vim();
        let mac = cfg!(target_os = "macos");
        div()
            .id("sidebar")
            .debug_selector(|| "session-sidebar".into())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.rail_hover(*hovered, cx)))
            .w(px(t.layout.rail_width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.base))
            .rounded(px(t.radius.panel))
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(t.space.xs))
            .py(px(t.space.md))
            .child(
                chrome::drag_region(div())
                    .debug_selector(|| "sidebar-drag-region".into())
                    .h(px(t.layout.control_height_sm))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(brand_mark(t.icon.lg, p)),
            )
            .child(self.sidebar_toggle_v2(cx))
            .child(
                primitives::icon_button_with_shortcut(
                    "new-session-rail",
                    IconName::Plus,
                    "New session",
                    Some(if mac { "⌘ N" } else { "Ctrl N" }),
                    ButtonVariant::Tertiary,
                    !self.demo,
                    &p,
                )
                .debug_selector(|| "new-session-button".into())
                .when(!self.demo, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.show_new_session(window, cx)))
                }),
            )
            .child(
                primitives::icon_button_with_shortcut(
                    "search-rail",
                    IconName::Search,
                    "Search everything",
                    Some(if mac { "⌘ K" } else { "Ctrl K" }),
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .debug_selector(|| "sidebar-search".into())
                .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
            )
            .when_some(self.rail_filter(), |d, mark| {
                d.child(mark.on_click(cx.listener(|this, _, _, cx| {
                    this.project_filter = None;
                    cx.notify();
                })))
            })
            .when(!self.ui_bus.notice.is_empty(), |d| {
                d.child(
                    primitives::icon_button(
                        "notice-rail",
                        IconName::Info,
                        "Show the workspace notice",
                        ButtonVariant::Tertiary,
                        true,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_peek(super::portrait::Peek::Pinned, cx)
                    })),
                )
            })
            .child(
                div()
                    .w(px(t.layout.rail_width - 2. * t.space.md))
                    .my(px(t.space.xs))
                    .h(px(t.stroke.hairline))
                    .bg(rgb(p.border)),
            )
            .child(
                div()
                    .id("session-rail")
                    .debug_selector(|| "session-rail".into())
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .children(
                        shown
                            .iter()
                            .enumerate()
                            .map(|(n, &index)| self.render_rail_tile(index, n, cx)),
                    )
                    .when(shown.is_empty(), |d| {
                        d.child(
                            div()
                                .id("rail-all-clear")
                                .debug_selector(|| "rail-all-clear".into())
                                .h(px(t.layout.rail_row))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(p.roles.text_tertiary))
                                .tooltip(tooltip_view(
                                    "All clear · nothing is waiting on you",
                                    None,
                                    &p,
                                ))
                                .child(Icon::new(IconName::Check).size(px(t.icon.sm))),
                        )
                    })
                    .when(rest > 0, |d| {
                        d.child(
                            chrome::interactive_control(div().id("rail-more"), p, true)
                                .debug_selector(|| "rail-more".into())
                                .mt(px(t.space.xs))
                                .size(px(t.layout.rail_session))
                                .rounded(px(t.radius.control))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(t.text.overline))
                                .font_weight(FontWeight(t.weight.semibold))
                                .map(tabular)
                                .text_color(rgb(p.roles.text_tertiary))
                                .border(px(t.stroke.hairline))
                                .border_color(rgb(p.border))
                                .hover(move |s| s.bg(rgb(p.roles.hover)))
                                .tooltip(tooltip_view(
                                    format!(
                                        "{rest} more {} · show the full list",
                                        if rest == 1 { "session" } else { "sessions" }
                                    ),
                                    None,
                                    &p,
                                ))
                                .child(format!("+{}", rest.min(99)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar_collapsed = false;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(self.view.power_paused, |d| {
                d.child(
                    primitives::icon_button(
                        "resume-rail",
                        IconName::Redo,
                        "Reconnect",
                        ButtonVariant::Tertiary,
                        self.view.can_resume_power_pause,
                        &p,
                    )
                    .when(self.view.can_resume_power_pause, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.command(Command::Refresh, cx)))
                    }),
                )
            })
            .child(self.nav_icon(
                "projects-rail",
                IconName::Folder,
                "Projects",
                Some(if mac { "⌘ P" } else { "Ctrl P" }),
                Screen::Projects,
                cx,
            ))
            .child(self.nav_icon(
                "jobs-rail",
                IconName::Calendar,
                "Jobs",
                vim.then_some("g j"),
                Screen::Jobs,
                cx,
            ))
            .child(self.nav_icon(
                "hosts-rail",
                IconName::LayoutDashboard,
                "Machines",
                vim.then_some("g o"),
                Screen::Hosts,
                cx,
            ))
            .child(
                self.nav_icon(
                    "scratch-rail",
                    IconName::FolderClosed,
                    "Scratch files",
                    vim.then_some("g x"),
                    Screen::Scratch,
                    cx,
                )
                .relative()
                .when(self.pending_total() > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(t.space.xxs))
                            .right(px(t.space.xxs))
                            .size(px(t.layout.status_dot))
                            .rounded(px(t.radius.full))
                            .bg(rgb(p.roles.text_secondary)),
                    )
                }),
            )
            .child(self.nav_icon(
                "settings-rail",
                IconName::Settings,
                "Settings",
                Some(if mac { "⌘ ," } else { "Ctrl ," }),
                Screen::Settings,
                cx,
            ))
            .when(!self.view.connected, |d| {
                d.child(
                    div()
                        .id("rail-offline")
                        .tooltip(tooltip_view(self.connection_copy().label, None, &p))
                        .child(status_indicator(StatusKind::Offline, &p)),
                )
            })
            .into_any_element()
    }

    /// One rail tile: the session's avatar with its status glyph as a
    /// corner badge (amber dot needs you, clock not responding, arc at work).
    fn render_rail_tile(&self, index: usize, n: usize, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let now = timing::now_ms();
        let s = &self.view.sessions[index];
        let active = self.is_active(&s.id);
        let title = self.session_title(s);
        let kind = self.session_kind(s, now);
        let kind = if kind == StatusKind::Idle && self.session_bucket(s, now) == Bucket::NeedsYou {
            StatusKind::Failed
        } else {
            kind
        };
        let reason = self.inbox_reason(s, now);
        let age = s
            .telemetry
            .last_activity_ms
            .map(|at| format!(" · {}", wks_native::format::age(at, now)))
            .unwrap_or_default();
        let meta = match &reason {
            Some(reason) => format!("{}\n{}{age}", reason.text, self.project_name(&s.cwd)),
            None => format!("{}{age}", self.project_name(&s.cwd)),
        };
        let size = t.layout.rail_session;
        // r3-foundation: no second ring colour; the status is the R3.1
        // glyph as a corner badge.
        let outer = size + 4. * t.stroke.focus;
        let card = hover_card(title.clone(), StatusBadge::new(kind), meta, p);
        let mark = div()
            .size(px(outer))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(primitives::avatar_with_status(
                &title,
                size,
                Some(kind),
                p.base,
                &p,
            ));
        div()
            .relative()
            .h(px(t.layout.rail_row))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px((t.layout.rail_row - t.icon.lg) / 2.))
                        .w(px(t.stroke.focus + t.stroke.hairline))
                        .h(px(t.icon.lg))
                        .rounded_r(px(t.radius.xs))
                        .bg(rgb(p.accent)),
                )
            })
            .child(
                chrome::interactive_control(
                    div().id(SharedString::from(format!("session-{}", s.id))),
                    p,
                    true,
                )
                .debug_selector(move || format!("sidebar-session-{n}"))
                .rounded(px(t.radius.panel))
                .tooltip(card)
                .child(mark)
                .on_click(cx.listener(move |this, _, _, cx| this.open_listed_session(index, cx))),
            )
            .into_any_element()
    }

    // ── The keyboard cheat sheet ───────────────────────────────────────

    /// `?` opens the cheat sheet whenever no field has the keys, modal keys
    /// on or off (with them on, the `VimNormal` binding already does).
    pub(super) fn shell_keys(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.design().is_v2() || self.vim() || !self.focus.is_focused(window) {
            return;
        }
        if event.keystroke.key_char.as_deref() == Some("?") {
            self.toggle_shortcuts(cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn toggle_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.shell.shortcuts = !self.shell.shortcuts;
        cx.notify();
    }

    /// Every key worth knowing, grouped; the modal keys only when they are on.
    pub(super) fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shell.shortcuts || !self.design().is_v2() {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let mac = cfg!(target_os = "macos");
        let m = |key: &'static str, mac_key: &'static str| if mac { mac_key } else { key };
        let mut groups: Vec<(&'static str, Vec<(&'static str, &'static str)>)> = vec![
            (
                "Sessions",
                vec![
                    (
                        "Command palette: sessions, actions, settings",
                        m("Ctrl K", "⌘ K"),
                    ),
                    ("Keyboard shortcuts", m("Ctrl /", "⌘ /")),
                    ("Next session that needs you", "Alt N"),
                    ("Next / previous session", "Alt ↓, Alt ↑"),
                    ("New session", m("Ctrl N", "⌘ N")),
                    ("Projects", m("Ctrl P", "⌘ P")),
                    ("Settings", m("Ctrl ,", "⌘ ,")),
                ],
            ),
            (
                "Conversation",
                vec![
                    ("Type a message", m("Ctrl L", "⌘ L")),
                    ("Send", m("Ctrl Enter", "⌘ Enter")),
                    ("Files and editor", m("Ctrl Shift E", "⌘ Shift E")),
                    ("Terminal", m("Ctrl `", "⌘ `")),
                    ("Model popover: choose, apply, close", "↑ ↓, Enter, Esc"),
                    ("Refresh", m("Ctrl R", "⌘ R")),
                    ("Zoom in / out", m("Ctrl +, Ctrl -", "⌘ +, ⌘ -")),
                ],
            ),
        ];
        if self.vim() {
            groups.push((
                "Modal keys (press Esc first)",
                vec![
                    ("Next / previous session", "j, k"),
                    ("Start typing", "i"),
                    ("Search", "/"),
                    ("Next session that needs you", "g n"),
                    ("Projects · Settings · History", "g p, g s, g h"),
                    ("Changes · Details · Model", "g d, g e, g m"),
                    ("Jobs · Machines · Scratch", "g j, g o, g x"),
                    ("First / last", "g g, G"),
                    ("This sheet", "?"),
                ],
            ));
        }
        let row = |label: &'static str, keys: &'static str| {
            div()
                .h(px(t.layout.control_height_sm))
                .flex()
                .items_center()
                .gap(px(t.space.lg))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(label),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.sm))
                        // ", " separates alternatives; spaces separate the
                        // keys pressed together or in sequence.
                        .children(keys.split(", ").enumerate().map(|(ix, chord)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(t.space.xxs))
                                .when(ix > 0, |d| {
                                    d.child(
                                        div()
                                            .pr(px(t.space.xs))
                                            .text_size(px(t.text.meta))
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child("/"),
                                    )
                                })
                                .children(chord.split(' ').map(|k| kbd(k, &p)))
                        })),
                )
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(t.space.lg))
            .children(groups.into_iter().map(|(title, keys)| {
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .pb(px(t.space.xs))
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.semibold))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(title),
                    )
                    .children(keys.into_iter().map(|(l, k)| row(l, k)))
            }));
        let dialog = primitives::dialog(&p)
            .id("shortcuts-dialog")
            .debug_selector(|| "shortcuts-dialog".into())
            .occlude()
            .max_h(gpui::relative(0.85))
            .overflow_y_scroll()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(primitives::dialog_header(
                        "Keyboard shortcuts",
                        Some(if self.vim() {
                            "Modal keys are on: Esc leaves the message box, i returns to it.".into()
                        } else {
                            "Most actions are also in the command palette. Single-key moves are in Settings → Keyboard."
                                .into()
                        }),
                        &p,
                    )))
                    .child(
                        primitives::icon_button(
                            "shortcuts-close",
                            IconName::Close,
                            "Close",
                            ButtonVariant::Tertiary,
                            true,
                            &p,
                        )
                        .debug_selector(|| "shortcuts-close".into())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx))),
                    ),
            )
            .child(body);
        Some(
            deferred(
                primitives::scrim(&p)
                    .id("shortcuts-scrim")
                    .debug_selector(|| "shortcuts-scrim".into())
                    .occlude()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx)),
                    )
                    .child(dialog),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
