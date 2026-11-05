//! r3-fleet (R3.7): the Fleet view, a responsive grid of live session tiles
//! for supervising several agents at once. Each tile shows the session's
//! title and project, its status in the R3.1 lexicon, how long it has been
//! at it, its latest events and the ask it waits on, with that ask's own
//! actions. Arrows (or h j k l) move the focus between tiles, Enter opens
//! the focused session, 1 / 2 / 3 answer its ask and Esc goes back.
//!
//! Data: the same session list, groups and reasons the sidebar and palette
//! read (`Workspace::session_bucket`, `session_kind`, `inbox_reason`). The
//! hub sends no per-session transcript for sessions that are not open, so
//! a tile's event lines are what the session row itself reports: its last
//! tool, plan step, subagents and background work. Design v2 only.
use super::attention::{self, Bucket};
use super::composer_v2::{
    StripKey, approval_context, approval_facts, approval_headline, approval_request, plan_text,
    strip_keys,
};
use super::primitives::{
    self, ButtonSize, ButtonVariant, CountTone, EventRow, StatusBadge, StatusKind,
};
use super::*;
use gpui::AnyElement;

actions!(
    native,
    [
        /// Open the Fleet view (Ctrl+Shift+F, the sidebar, the palette).
        ShowFleet,
        FleetLeft,
        FleetRight,
        FleetUp,
        FleetDown,
        /// Open the focused tile's session.
        FleetOpen,
        /// The focused tile's ask: its first, second, … fifth action.
        FleetKey1,
        FleetKey2,
        FleetKey3,
        FleetKey4,
        FleetKey5
    ]
);

/// The key context the shell adds while the Fleet view has the keys.
pub(super) const KEY_CONTEXT: &str = "Fleet";

/// Window-local state of the Fleet view.
#[derive(Default)]
pub(super) struct FleetUi {
    /// The focused tile, by session id, so focus follows a session as the
    /// grid re-sorts. `None` (or a session no longer shown) is the first.
    pub(super) focus: Option<String>,
    /// Which tiles show: Needs you only, Active (the default) or All.
    pub(super) filter: Filter,
    /// The tile whose request Details is open, by session id.
    pub(super) details: Option<String>,
    /// Columns the grid was last drawn with, for Up and Down.
    pub(super) columns: usize,
}

/// A step of the keyboard focus across the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    Left,
    Right,
    Up,
    Down,
}

/// How many columns fit `width`: at most `max`, each at least `min` wide
/// with `gap` between them, and never fewer than one.
pub(crate) fn columns(width: f32, min: f32, gap: f32, max: usize) -> usize {
    let mut n = max.max(1);
    while n > 1 && (width - gap * (n - 1) as f32) / (n as f32) < min {
        n -= 1;
    }
    n
}

/// Where the focus lands from tile `at` of `count`, laid out in `columns`.
/// Left and Right walk the reading order (across rows); Up and Down keep
/// the column, and Down from a short last row's neighbour lands on the
/// last tile. Nothing wraps.
pub(crate) fn step(at: usize, count: usize, columns: usize, dir: Move) -> usize {
    if count == 0 {
        return 0;
    }
    let at = at.min(count - 1);
    let cols = columns.max(1);
    match dir {
        Move::Left => at.saturating_sub(1),
        Move::Right => (at + 1).min(count - 1),
        Move::Up => at.checked_sub(cols).unwrap_or(at),
        Move::Down if at + cols < count => at + cols,
        Move::Down if at / cols < (count - 1) / cols => count - 1,
        Move::Down => at,
    }
}

/// The Fleet's filter (R4.7): Needs you only for a burst of asks, Active
/// (Needs you and Working, the default) or All.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Filter {
    NeedsYou,
    #[default]
    Active,
    All,
}

/// Whether a group shows under `filter`: Needs you only; the default
/// keeps what matters (Needs you and Working); All adds Idle and Ended.
pub(crate) fn shown(bucket: Bucket, filter: Filter) -> bool {
    match filter {
        Filter::NeedsYou => bucket == Bucket::NeedsYou,
        Filter::Active => matches!(bucket, Bucket::NeedsYou | Bucket::Working),
        Filter::All => true,
    }
}

/// The header's one-line count of the fleet: "2 need you · 3 working ·
/// 5 idle · 20 ended" (empty groups left out).
pub(crate) fn summary(counts: [usize; 4]) -> String {
    let parts: Vec<String> = Bucket::ALL
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(bucket, n)| match bucket {
            Bucket::NeedsYou if n == 1 => "1 needs you".to_owned(),
            Bucket::NeedsYou => format!("{n} need you"),
            other => format!("{n} {}", other.label().to_lowercase()),
        })
        .collect();
    if parts.is_empty() {
        "No sessions yet".into()
    } else {
        parts.join(" · ")
    }
}

/// A tool's line in a tile: its glyph and verb ("Read", "Run", "Edit").
pub(crate) fn tool_line(name: &str) -> (IconName, String) {
    let short = name.rsplit("__").next().unwrap_or(name);
    let lower = short.to_ascii_lowercase();
    match lower.as_str() {
        "read" | "notebookread" | "read_file" => (IconName::File, "Read".into()),
        "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" => {
            (IconName::Replace, "Edit".into())
        }
        "bash" | "shell" | "exec" | "exec_command" | "local_shell" => {
            (IconName::SquareTerminal, "Run".into())
        }
        "grep" | "glob" | "search" | "websearch" | "ls" => (IconName::Search, "Search".into()),
        "webfetch" | "web_fetch" => (IconName::Globe, "Open".into()),
        "task" | "agent" => (IconName::Bot, "Delegate".into()),
        _ if lower.contains("todo") || lower.contains("plan") => {
            (IconName::GalleryVerticalEnd, "Plan".into())
        }
        _ => (IconName::Settings, short.to_owned()),
    }
}

/// The next tile after `position` (wrapping, itself excluded) that asks
/// something, by session id.
pub(super) fn next_asking(tiles: &[Tile], position: usize) -> Option<String> {
    (1..tiles.len())
        .map(|step| &tiles[(position + step) % tiles.len()])
        .find(|t| t.ask.is_some())
        .map(|t| t.id.clone())
}

/// One event line of a tile.
#[derive(Clone)]
pub(super) struct Line {
    pub icon: IconName,
    pub verb: String,
    pub argument: Option<String>,
    pub summary: Option<String>,
    /// A named failure or a stall: red, with its glyph.
    pub fault: Option<StatusKind>,
}

/// What a tile waits on, and so what its keys do.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ask {
    /// A tool waits for permission (R4.3, as in the conversation): the
    /// request as a headline (words and code spans), why, a long command
    /// whole, its folder and host, each "Always allow" grant as (option id,
    /// the provider's label, the exact rule shown, tooltip), the keys in button order (Allow, each grant,
    /// Deny) and the full request for Details.
    Approval {
        headline: Vec<(String, bool)>,
        context: Option<String>,
        request: Option<String>,
        folder: String,
        host: String,
        always: Vec<(String, String, String, String)>,
        keys: Vec<StripKey>,
        facts: Vec<(String, String, bool)>,
    },
    /// A plan or a question: its headline, answered in the session (1).
    Open {
        headline: String,
        verb: &'static str,
    },
}

/// One tile of the grid.
#[derive(Clone)]
pub(super) struct Tile {
    /// Into `view.sessions`.
    pub index: usize,
    pub id: String,
    pub title: String,
    pub project: String,
    pub model: String,
    pub kind: StatusKind,
    pub bucket: Bucket,
    pub time: Option<String>,
    /// The context window's share in use, when the runtime reports it.
    pub context: Option<f64>,
    pub lines: Vec<Line>,
    pub ask: Option<Ask>,
}

impl Workspace {
    /// Every session the fleet knows (the sidebar's visible set), most
    /// urgent first: Needs you oldest wait first, Working in the hub's
    /// order, then Idle and Ended most recent first.
    pub(super) fn fleet_sessions(&self, cx: &App) -> Vec<(usize, Bucket)> {
        let now = timing::now_ms();
        let sessions = &self.view.sessions;
        let mut all: Vec<(usize, Bucket)> = self
            .visible_sessions(cx)
            .into_iter()
            .map(|ix| (ix, self.session_bucket(&sessions[ix], now)))
            .collect();
        let activity = |ix: usize| sessions[ix].telemetry.last_activity_ms;
        all.sort_by_key(|&(ix, bucket)| {
            let order = match bucket {
                Bucket::NeedsYou => activity(ix).unwrap_or(i64::MAX),
                Bucket::Working => 0,
                Bucket::Idle | Bucket::Ended => -activity(ix).unwrap_or(i64::MIN / 2),
            };
            (bucket, order)
        });
        all
    }

    /// The tiles on screen, in grid order.
    pub(super) fn fleet_tiles(&self, cx: &App) -> Vec<Tile> {
        let now = timing::now_ms();
        self.fleet_sessions(cx)
            .into_iter()
            .filter(|&(_, bucket)| shown(bucket, self.fleet.filter))
            .map(|(ix, bucket)| self.fleet_tile(ix, bucket, now))
            .collect()
    }

    fn fleet_tile(&self, index: usize, bucket: Bucket, now: i64) -> Tile {
        let session = &self.view.sessions[index];
        let kind = self.session_kind(session, now);
        let time = if kind == StatusKind::Working {
            self.working_elapsed(session, now)
        } else {
            session
                .telemetry
                .last_activity_ms
                .map(|at| wks_native::format::age(at, now))
        };
        let reason = self.inbox_reason(session, now);
        let host = self.decision_host();
        let ask = session
            .approval
            .as_ref()
            .filter(|_| self.view.connected)
            .map(|approval| {
                if plan_text(approval).is_some() {
                    return Ask::Open {
                        headline: "Plan ready for review".into(),
                        verb: "Review plan",
                    };
                }
                // Every grant, with the same exact rule the conversation
                // shows (R4.3), never shortened to "Always allow".
                let always: Vec<(String, String, String, String)> =
                    wks_native::session_controls::always_options(approval)
                        .into_iter()
                        .map(|option| {
                            let (label, tip) =
                                super::composer_v2::always_copy(&option.label, approval);
                            (option.id, option.label, label, tip)
                        })
                        .collect();
                Ask::Approval {
                    headline: approval_headline(approval, &session.cwd),
                    context: approval_context(approval),
                    request: approval_request(approval),
                    folder: session.cwd.clone(),
                    host: host.clone(),
                    keys: strip_keys(always.len(), false),
                    always,
                    facts: approval_facts(approval, &session.cwd, &host),
                }
            })
            .or_else(|| {
                reason
                    .as_ref()
                    .filter(|r| r.action == attention::InboxAction::Answer)
                    .map(|r| Ask::Open {
                        headline: r.text.clone(),
                        verb: "Answer",
                    })
            });
        let mut lines = self.fleet_lines(session);
        // A stall or failure that put the session in Needs you, when no
        // line already names it (its own silence, say), leads.
        if let Some(reason) = reason.as_ref().filter(|r| r.fault)
            && !lines.iter().any(|l| l.fault.is_some())
        {
            lines.insert(
                0,
                Line {
                    icon: IconName::TriangleAlert,
                    verb: reason.text.clone(),
                    argument: None,
                    summary: None,
                    fault: Some(
                        if reason.text.starts_with(wks_native::format::NOT_RESPONDING) {
                            StatusKind::NotResponding
                        } else {
                            StatusKind::Failed
                        },
                    ),
                },
            );
        }
        lines.truncate(self.tokens().fleet().event_lines);
        Tile {
            index,
            id: session.id.clone(),
            title: self.session_title(session),
            project: self.project_name(&session.cwd),
            // R4.7: the catalog's casing ("GPT-5.6 Sol"), as the composer
            // chip shows it; never a second pass over a display name.
            model: {
                let name = session.display_model();
                if name.is_empty() {
                    wks_native::format::model_name("")
                } else {
                    name
                }
            },
            kind,
            bucket,
            time,
            context: session.context.reading().map(|r| r.pct.clamp(0., 100.)),
            lines,
            ask,
        }
    }

    /// What the session row reports it is doing, failures first and each
    /// one named: a subagent that stopped answering or failed, a failed
    /// background command; then the last tool, the plan step in progress,
    /// the subagents at work and the background work still running.
    fn fleet_lines(&self, session: &Session) -> Vec<Line> {
        let now = timing::now_ms();
        let mut faults = Vec::new();
        let mut lines = Vec::new();
        let children = self.native_children(session);
        for child in children.iter() {
            let name = [&child.description, &child.label]
                .into_iter()
                .find(|t| !t.trim().is_empty())
                .map_or_else(|| "A subagent".to_owned(), |t| t.trim().to_owned());
            if attention::child_stalled(child, now) {
                faults.push(Line {
                    icon: IconName::Bot,
                    verb: name,
                    argument: None,
                    summary: Some(wks_native::format::not_responding(
                        child
                            .telemetry
                            .last_activity_ms
                            .or(child.telemetry.started_at_ms),
                        now,
                    )),
                    fault: Some(StatusKind::NotResponding),
                });
            } else if child.failed() {
                faults.push(Line {
                    icon: IconName::Bot,
                    verb: name,
                    argument: None,
                    summary: Some("Failed".into()),
                    fault: Some(StatusKind::Failed),
                });
            }
        }
        let shells = || {
            session
                .tasks
                .iter()
                .filter(|t| t.kind != wks_native::background_tasks::Kind::Agent)
        };
        if !session.stopped()
            && let Some(failed) = shells().find(|t| t.failed())
        {
            faults.push(Line {
                icon: IconName::SquareTerminal,
                verb: wks_native::format::failed_label(&failed.title(), 48),
                argument: None,
                summary: None,
                fault: Some(StatusKind::Failed),
            });
        }
        let tool = session.telemetry.last_tool_name.trim();
        if !tool.is_empty() {
            let (icon, verb) = tool_line(tool);
            let summary = session.telemetry.last_tool_summary.trim();
            lines.push(Line {
                icon,
                verb,
                argument: (!summary.is_empty()).then(|| summary.to_owned()),
                summary: None,
                fault: None,
            });
        }
        if let Some(plan) = &session.plan {
            let total = plan.steps.len();
            let done = plan
                .steps
                .iter()
                .filter(|s| s.status == wks_native::plan::Status::Completed)
                .count();
            if let Some(step) = plan
                .steps
                .iter()
                .find(|s| s.status == wks_native::plan::Status::InProgress)
            {
                lines.push(Line {
                    icon: IconName::GalleryVerticalEnd,
                    verb: format!("Step {} of {total}", (done + 1).min(total)),
                    argument: None,
                    summary: Some(step.label().to_owned()),
                    fault: None,
                });
            }
        }
        let working = children
            .iter()
            .filter(|c| c.running() && !attention::child_stalled(c, now))
            .count();
        if working > 0 {
            lines.push(Line {
                icon: IconName::Bot,
                verb: if working == 1 {
                    "1 subagent working".into()
                } else {
                    format!("{working} subagents working")
                },
                argument: None,
                summary: None,
                fault: None,
            });
        }
        let running = if session.stopped() {
            0
        } else {
            shells().filter(|t| t.running()).count()
        };
        if running > 0 {
            lines.push(Line {
                icon: IconName::LoaderCircle,
                verb: if running == 1 {
                    "1 background task".into()
                } else {
                    format!("{running} background tasks")
                },
                argument: None,
                summary: None,
                fault: None,
            });
        }
        faults.extend(lines);
        faults
    }

    /// The focused tile's position among `tiles` (the first by default).
    fn fleet_focus(&self, tiles: &[Tile]) -> usize {
        self.fleet
            .focus
            .as_ref()
            .and_then(|id| tiles.iter().position(|t| &t.id == id))
            .unwrap_or(0)
    }

    pub(super) fn open_fleet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.design().is_v2() {
            self.show_screen(Screen::Fleet, window, cx);
        }
    }

    fn fleet_move(&mut self, dir: Move, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        if tiles.is_empty() {
            return;
        }
        let at = step(
            self.fleet_focus(&tiles),
            tiles.len(),
            self.fleet.columns,
            dir,
        );
        self.fleet.focus = Some(tiles[at].id.clone());
        cx.notify();
    }

    fn fleet_open(&mut self, position: usize, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        if let Some(tile) = tiles.get(position) {
            self.fleet.focus = Some(tile.id.clone());
            self.open_listed_session(tile.index, cx);
        }
    }

    /// Key `n` (1-based) on tile `position`'s ask: an approval's Allow,
    /// Always allow and Deny in button order; a plan's or question's 1
    /// opens the session at it.
    fn fleet_answer(
        &mut self,
        position: usize,
        n: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tiles = self.fleet_tiles(cx);
        let Some(tile) = tiles.get(position) else {
            return;
        };
        self.fleet.focus = Some(tile.id.clone());
        window.focus(&self.focus);
        match &tile.ask {
            Some(Ask::Approval { keys, always, .. }) => {
                let Some(key) = n.checked_sub(1).and_then(|ix| keys.get(ix)) else {
                    return;
                };
                if !self.view.connected || self.view.busy {
                    return;
                }
                let card = self.view.sessions[tile.index].card();
                let action = match key {
                    StripKey::Allow => Action::Approve(true, card),
                    StripKey::Deny => Action::Approve(false, card),
                    StripKey::Always(ix) => match (always.get(*ix), card) {
                        (Some((option, label, _, _)), Some(card)) => Action::AlwaysAllow {
                            option: option.clone(),
                            label: label.clone(),
                            card,
                        },
                        _ => return,
                    },
                    StripKey::DenyWithNote => return,
                };
                self.fleet_act(tile.index, action, cx);
                // Decided: the focus moves on to the next tile that asks
                // something, so 1, 1, 1 clears a run of approvals.
                if let Some(next) = next_asking(&tiles, position) {
                    self.fleet.focus = Some(next);
                }
            }
            Some(Ask::Open { .. }) if n == 1 => self.open_listed_session(tile.index, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The hub acts on the selected session only, so a tile's decision
    /// selects its session first (the Fleet stays on screen; Esc then lands
    /// on it), then acts on the card the tile showed.
    fn fleet_act(&mut self, index: usize, action: Action, cx: &mut Context<Self>) {
        let id = self.view.sessions[index].id.clone();
        if self.view.selected.as_ref() != Some(&id) || self.view.child.is_some() {
            self.command(Command::Select(id.clone()), cx);
        }
        self.command(
            Command::Act {
                session: id,
                action,
            },
            cx,
        );
    }

    /// The Fleet's keys on the shell root (bound under [`KEY_CONTEXT`]).
    pub(super) fn fleet_actions(&self, shell: Div, cx: &mut Context<Self>) -> Div {
        shell
            .on_action(cx.listener(|this, _: &ShowFleet, window, cx| this.open_fleet(window, cx)))
            .on_action(cx.listener(|this, _: &FleetLeft, _, cx| this.fleet_move(Move::Left, cx)))
            .on_action(cx.listener(|this, _: &FleetRight, _, cx| this.fleet_move(Move::Right, cx)))
            .on_action(cx.listener(|this, _: &FleetUp, _, cx| this.fleet_move(Move::Up, cx)))
            .on_action(cx.listener(|this, _: &FleetDown, _, cx| this.fleet_move(Move::Down, cx)))
            .on_action(cx.listener(|this, _: &FleetOpen, _, cx| {
                let tiles = this.fleet_tiles(cx);
                let at = this.fleet_focus(&tiles);
                this.fleet_open(at, cx)
            }))
            .on_action(cx.listener(|this, _: &FleetKey1, window, cx| this.fleet_key(1, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey2, window, cx| this.fleet_key(2, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey3, window, cx| this.fleet_key(3, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey4, window, cx| this.fleet_key(4, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey5, window, cx| this.fleet_key(5, window, cx)))
    }

    fn fleet_key(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let at = self.fleet_focus(&tiles);
        self.fleet_answer(at, n, window, cx);
    }

    /// Whether the Fleet has the keys: on screen, under v2, with nothing
    /// else focused.
    pub(super) fn fleet_keys(&self, window: &Window) -> bool {
        self.screen == Screen::Fleet
            && !self.new_session
            && self.design().is_v2()
            && self.focus.is_focused(window)
    }

    // ── Drawing ─────────────────────────────────────────────────────────

    pub(super) fn render_fleet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let f = t.fleet();
        // The page's room: the window less the sidebar and the page gutters.
        let width = unzoom(window.viewport_size().width);
        let room = (width - self.sidebar_footprint(width) - 2. * t.space.xxl).min(f.grid_max);
        let cols = columns(room, f.tile_min, t.space.lg, f.columns_max);
        self.fleet.columns = cols;
        let groups = self.fleet_sessions(cx);
        let mut counts = [0usize; 4];
        for (_, bucket) in &groups {
            counts[bucket.index()] += 1;
        }
        let active = counts[Bucket::NeedsYou.index()] + counts[Bucket::Working.index()];
        let quiet = groups.len() - active;
        let tiles = self.fleet_tiles(cx);
        let focus = self.fleet_focus(&tiles);
        let current = self.fleet.filter;
        let all = current == Filter::All;
        let needs = counts[Bucket::NeedsYou.index()];
        let filter = |id: &'static str, label: &'static str, on: Filter, n: usize| {
            primitives::segment(id, label, current == on, &p)
                .debug_selector(move || id.into())
                .gap(px(t.space.sm))
                .child(primitives::count_badge(
                    primitives::count_text(n),
                    CountTone::Neutral,
                    &p,
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.fleet.filter = on;
                    window.focus(&this.focus);
                    cx.notify();
                }))
        };
        let header = primitives::page_header(
            None,
            "Fleet",
            Some(summary(counts).into()),
            vec![
                primitives::segmented(&p)
                    .child(filter(
                        "fleet-filter-needs",
                        "Needs you",
                        Filter::NeedsYou,
                        needs,
                    ))
                    .child(filter(
                        "fleet-filter-active",
                        "Active",
                        Filter::Active,
                        active,
                    ))
                    .child(filter("fleet-filter-all", "All", Filter::All, groups.len()))
                    .into_any_element(),
            ],
            &p,
        );
        let body: AnyElement = if tiles.is_empty() {
            let (title, body) = if groups.is_empty() {
                (
                    "No sessions yet",
                    "Start a session and it shows here, live, beside every other.".to_owned(),
                )
            } else if current == Filter::NeedsYou {
                (
                    "All clear",
                    format!("Nothing is waiting on you. {}.", summary(counts)),
                )
            } else {
                (
                    "All clear",
                    format!(
                        "No agent is working or waiting on you. {}.",
                        summary(counts)
                    ),
                )
            };
            // The way out of an empty filter: the next wider one.
            let (wider, label) = if current == Filter::NeedsYou && active > 0 {
                (Filter::Active, format!("Show active {active}"))
            } else {
                (Filter::All, format!("Show all {}", groups.len()))
            };
            let offer = !all && (quiet > 0 || wider == Filter::Active);
            primitives::empty_state(IconName::CircleCheck, title, Some(body.into()), &p)
                .debug_selector(|| "fleet-empty".into())
                .when(offer, |d| {
                    d.child(
                        primitives::button(
                            "fleet-show-all",
                            label,
                            None,
                            ButtonVariant::Secondary,
                            ButtonSize::Small,
                            true,
                            &p,
                        )
                        .debug_selector(|| "fleet-show-all".into())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.fleet.filter = wider;
                                window.focus(&this.focus);
                                cx.notify();
                            },
                        )),
                    )
                })
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.lg))
                .when(needs == 0, |d| {
                    d.child(
                        div()
                            .debug_selector(|| "fleet-all-clear".into())
                            .flex()
                            .items_center()
                            .gap(px(t.space.sm))
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(
                                Icon::new(IconName::Check)
                                    .size(px(t.icon.sm))
                                    .text_color(rgb(p.roles.text_tertiary)),
                            )
                            .child("All clear · nothing is waiting on you"),
                    )
                })
                .child(
                    div()
                        .debug_selector(|| "fleet-grid".into())
                        .w_full()
                        .grid()
                        .grid_cols(cols as u16)
                        // R4.7: tiles in one row share its height (the
                        // tallest, at least `tile_height`); a tile's ask
                        // sits at its foot, so asks line up across a row.
                        .gap(px(t.space.lg))
                        .children(
                            tiles
                                .iter()
                                .enumerate()
                                .map(|(n, tile)| self.render_fleet_tile(n, tile, n == focus, cx)),
                        ),
                )
                .into_any_element()
        };
        let content = div()
            .debug_selector(|| "fleet-page".into())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(t.space.xxl))
            .child(header)
            .child(body)
            .child(self.fleet_hints());
        self.page_view("fleet-scroll", f.grid_max, false, content)
    }

    /// The keys, once, under the grid: faint keycaps and plain words.
    fn fleet_hints(&self) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let hint = |keys: &[&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .children(keys.iter().map(|k| primitives::keycap_hint(*k, false, &p)))
                .child(div().pl(px(t.space.xxs)).child(label))
        };
        div()
            .debug_selector(|| "fleet-hints".into())
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(t.space.xl))
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(hint(&["←", "↑", "↓", "→"], "Move"))
            .child(hint(&["↵"], "Open"))
            .child(hint(&["1", "2", "3"], "Answer the focused ask"))
            .child(hint(&["Esc"], "Back"))
    }

    fn render_fleet_tile(
        &self,
        n: usize,
        tile: &Tile,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let f = t.fleet();
        let index = tile.index;
        let kind = tile.kind;
        let waiting = kind == StatusKind::NeedsYou;
        let ended = tile.bucket == Bucket::Ended;
        // Line one: the title (it truncates) and the time.
        let head = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .child(
                div()
                    .debug_selector(move || format!("fleet-title-{n}"))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(if ended {
                        p.roles.text_secondary
                    } else {
                        p.text
                    }))
                    .child(tile.title.clone()),
            )
            .children(tile.time.clone().map(|time| {
                primitives::tabular(
                    div()
                        .flex_shrink_0()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(time),
                )
            }));
        // Line two: the status in the lexicon, then where and which model.
        let meta = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(t.space.sm))
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(
                div()
                    .debug_selector(move || format!("fleet-status-{n}-{kind:?}"))
                    .flex_shrink_0()
                    .child(if kind.has_glyph() {
                        StatusBadge::new(kind)
                            .render(&p)
                            .debug_selector(move || format!("fleet-status-{n}"))
                    } else {
                        // Idle draws no glyph (R3.1); its empty glyph box
                        // would read as an indent here, so the word leads.
                        div()
                            .debug_selector(move || format!("fleet-status-{n}"))
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(kind.text_color(&p)))
                            .child(kind.label())
                    }),
            )
            .child("·")
            .child(div().flex_shrink_0().child(tile.project.clone()))
            .child("·")
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(tile.model.clone()),
            )
            .children(tile.context.map(|pct| {
                let color = primitives::level_color(wks_native::format::level(pct), &p)
                    .unwrap_or(p.roles.text_tertiary);
                primitives::tabular(
                    div()
                        .debug_selector(move || format!("fleet-context-{n}"))
                        .flex_shrink_0()
                        .text_color(rgb(color))
                        .child(wks_native::format::context_label(Some(pct))),
                )
            }));
        let lines = if tile.lines.is_empty() {
            vec![
                div()
                    .h(px(f.event_row))
                    .flex()
                    .items_center()
                    .text_size(px(t.text.body))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(match (tile.bucket, &tile.ask) {
                        (Bucket::Ended, _) => "Ended",
                        (Bucket::Working, _) => "Starting…",
                        (Bucket::NeedsYou, Some(Ask::Open { verb: "Answer", .. })) => {
                            "Waiting for your answer"
                        }
                        (Bucket::NeedsYou, _) => "Waiting for your decision",
                        (Bucket::Idle, _) => "Ready for your next message",
                    })
                    .into_any_element(),
            ]
        } else {
            tile.lines
                .iter()
                .enumerate()
                .map(|(ix, line)| {
                    let mut row = EventRow::new(line.icon.clone(), line.verb.clone());
                    if let Some(argument) = &line.argument {
                        row = row.argument(argument.clone());
                    }
                    if let Some(summary) = &line.summary {
                        row = row.summary(summary.clone());
                    }
                    if let Some(fault) = line.fault {
                        row = row.status(fault);
                    }
                    row.render(&p)
                        .debug_selector(move || format!("fleet-line-{n}-{ix}"))
                        .h(px(f.event_row))
                        .into_any_element()
                })
                .collect()
        };
        let ask = tile
            .ask
            .as_ref()
            .map(|ask| self.render_fleet_ask(n, tile, ask, focused, cx));
        let group: SharedString = format!("fleet-tile-{n}").into();
        let surface = if waiting {
            primitives::attention_frame(t.radius.panel, &p)
                .pl(px(t.space.xl + t.signals().attention_bar))
        } else {
            div()
                .relative()
                .rounded(px(t.radius.panel))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.roles.card_border))
                .group_hover(group.clone(), |s| {
                    s.border_color(rgb(p.roles.border_strong))
                })
                .pl(px(t.space.xl))
        };
        let card = surface
            .size_full()
            .min_h(px(f.tile_height))
            .pr(px(t.space.xl))
            .py(px(t.space.lg))
            .flex()
            .flex_col()
            .gap(px(t.space.md))
            .bg(rgb(p.surface))
            .shadow(primitives::elevation(&p, t.elevation.raised))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .child(head)
                    .child(meta),
            )
            .child(
                div()
                    .debug_selector(move || format!("fleet-lines-{n}"))
                    .flex()
                    .flex_col()
                    .children(lines),
            )
            .child(div().flex_1())
            .children(ask);
        chrome::interactive_control(
            div().id(SharedString::from(format!("fleet-tile-{}", tile.id))),
            p,
            true,
        )
        .debug_selector(move || format!("fleet-tile-{n}"))
        .group(group)
        .rounded(px(t.radius.panel + t.stroke.focus))
        .when(focused, |d| {
            d.border_color(rgb(p.roles.focus))
                .child(div().debug_selector(move || format!("fleet-focused-{n}")))
        })
        .tooltip(primitives::tooltip_view(
            format!("Open {}", tile.title),
            Some("Enter"),
            &p,
        ))
        .child(card)
        .on_click(cx.listener(move |this, _, _, cx| {
            let tiles = this.fleet_tiles(cx);
            if let Some(tile) = tiles.iter().find(|t| t.index == index) {
                this.fleet.focus = Some(tile.id.clone());
            }
            this.open_listed_session(index, cx);
        }))
        .into_any_element()
    }

    /// A tile's ask, under a hairline, drawn by the one decision component
    /// (R4.3): the request as a headline, why, a long command whole, its
    /// folder and host, then Allow (always the Primary), each "Always allow"
    /// with its exact rule and Deny (Secondary). Details expands the full
    /// request in place. Keycaps show only on the focused tile, where the
    /// number keys go.
    fn render_fleet_ask(
        &self,
        n: usize,
        tile: &Tile,
        ask: &Ask,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::decision::{Choice, Decision, Details, Frame};
        let p = self.palette();
        let t = p.tokens;
        let enabled = self.view.connected && !self.view.busy;
        let choice = |id: String, key: usize, label: SharedString| {
            Choice::new(id, label)
                .key(Some(key))
                .enabled(enabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.fleet_answer(n, key, window, cx);
                }))
        };
        let decision = match ask {
            Ask::Approval {
                headline,
                context,
                request,
                folder,
                host,
                always,
                keys,
                facts,
            } => {
                let number = |key: &StripKey| keys.iter().position(|k| k == key).map(|ix| ix + 1);
                let allow = choice(
                    format!("fleet-allow-{n}"),
                    number(&StripKey::Allow).unwrap_or(1),
                    "Allow".into(),
                )
                .tooltip("Allow this once");
                let mut decision =
                    Decision::new(format!("fleet-decision-{n}"), allow, Frame::Inline);
                decision.headline = headline.clone();
                decision.headline_selector = Some(format!("fleet-headline-{n}").into());
                decision.reason = context.clone();
                decision.request = request.clone();
                decision.request_selector = format!("fleet-request-{n}").into();
                decision.folder = Some(folder.clone());
                decision.host = Some(host.clone());
                decision.alternates = always
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, (_, _, rule, tip))| {
                        let key = number(&StripKey::Always(ix))?;
                        // The first grant keeps its selector.
                        let id = if ix == 0 {
                            format!("fleet-always-{n}")
                        } else {
                            format!("fleet-always-{n}-{ix}")
                        };
                        Some(choice(id, key, rule.clone().into()).tooltip(tip.clone()))
                    })
                    .collect();
                decision.deny = number(&StripKey::Deny).map(|key| {
                    choice(format!("fleet-deny-{n}"), key, "Deny".into())
                        .tooltip("Deny this request")
                });
                let open = self.fleet.details.as_deref() == Some(tile.id.as_str());
                let id = tile.id.clone();
                decision.details = Some(Details {
                    toggle: Choice::new(format!("fleet-details-{n}"), "Details").on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.fleet.focus = Some(id.clone());
                            this.fleet.details = if this.fleet.details.as_ref() == Some(&id) {
                                None
                            } else {
                                Some(id.clone())
                            };
                            cx.notify();
                        }),
                    ),
                    open,
                    content: open.then(|| {
                        super::decision::facts("fleet-details", "fleet-fact", facts.clone(), &p)
                            .into_any_element()
                    }),
                });
                decision.keys = focused;
                decision
            }
            Ask::Open { headline, verb } => {
                let mut decision = Decision::new(
                    format!("fleet-decision-{n}"),
                    choice(format!("fleet-open-{n}"), 1, (*verb).into()),
                    Frame::Inline,
                );
                decision.headline = vec![(headline.clone(), false)];
                decision.headline_selector = Some(format!("fleet-headline-{n}").into());
                decision.keys = focused;
                decision
            }
        };
        div()
            .debug_selector(move || format!("fleet-ask-{n}"))
            .w_full()
            .flex()
            .flex_col()
            .pt(px(t.space.lg))
            .border_t(px(t.stroke.hairline))
            .border_color(rgb(p.roles.card_border))
            .child(decision.render(&p))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_drop_one_at_a_time_and_never_reach_zero() {
        assert_eq!(columns(1240., 320., 12., 3), 3);
        assert_eq!(columns(900., 320., 12., 3), 2);
        assert_eq!(columns(500., 320., 12., 3), 1);
        assert_eq!(columns(10., 320., 12., 3), 1);
        assert_eq!(columns(5000., 320., 12., 3), 3);
    }

    #[test]
    fn focus_moves_across_the_grid_without_wrapping() {
        // 5 tiles in 3 columns: 0 1 2 / 3 4.
        assert_eq!(step(0, 5, 3, Move::Right), 1);
        assert_eq!(step(2, 5, 3, Move::Right), 3);
        assert_eq!(step(4, 5, 3, Move::Right), 4);
        assert_eq!(step(0, 5, 3, Move::Left), 0);
        assert_eq!(step(1, 5, 3, Move::Down), 4);
        // Down from above the short row's gap lands on the last tile.
        assert_eq!(step(2, 5, 3, Move::Down), 4);
        assert_eq!(step(4, 5, 3, Move::Down), 4);
        assert_eq!(step(4, 5, 3, Move::Up), 1);
        assert_eq!(step(1, 5, 3, Move::Up), 1);
        assert_eq!(step(0, 0, 3, Move::Down), 0);
    }

    #[test]
    fn the_default_view_keeps_what_matters() {
        assert!(shown(Bucket::NeedsYou, Filter::Active));
        assert!(shown(Bucket::Working, Filter::Active));
        assert!(!shown(Bucket::Idle, Filter::Active));
        assert!(!shown(Bucket::Ended, Filter::Active));
        assert!(shown(Bucket::Ended, Filter::All));
        // R4.7: Needs you only, for a burst of asks.
        assert!(shown(Bucket::NeedsYou, Filter::NeedsYou));
        assert!(!shown(Bucket::Working, Filter::NeedsYou));
    }

    #[test]
    fn the_summary_counts_in_the_lexicon() {
        assert_eq!(
            summary([2, 3, 5, 20]),
            "2 need you · 3 working · 5 idle · 20 ended"
        );
        assert_eq!(summary([1, 0, 0, 0]), "1 needs you");
        assert_eq!(summary([0, 0, 0, 0]), "No sessions yet");
    }

    #[test]
    fn tools_read_as_verbs() {
        assert_eq!(tool_line("Read").1, "Read");
        assert_eq!(tool_line("Bash").1, "Run");
        assert_eq!(tool_line("mcp__fs__read_file").1, "Read");
        assert_eq!(tool_line("Custom").1, "Custom");
    }
}
