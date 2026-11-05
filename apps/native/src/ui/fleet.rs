//! r3-fleet (R3.7), r5-fleet (R5.4): the Fleet view, a grid of live session
//! tiles at their natural height (or a dense table, one row per session)
//! for supervising several agents at once. Each tile shows the session's
//! title and project, its status in the R3.1 lexicon, how long it has been
//! at it, its latest events and the ask it waits on, with that ask's own
//! actions. Arrows (or h j k l) move the focus between tiles, Enter opens
//! the focused session, 1 / 2 / 3 answer its ask and Esc goes back.
//!
//! Data: the same session list, groups, counts and reasons the sidebar and
//! palette read (r4-truth: `Workspace::attention_groups`,
//! `attention_counts`, `session_status`, `inbox_reason`). The
//! hub sends no per-session transcript for sessions that are not open, so
//! a tile's event lines are what the session row itself reports: its last
//! tool, plan step, subagents and background work. Design v2 only.
use super::attention::{self, Bucket};
use super::composer_v2::{
    StripKey, approval_context, approval_facts, approval_headline, approval_request, plan_text,
    strip_keys,
};
use super::primitives::{self, ButtonSize, ButtonVariant, Col, EventRow, StatusKind};
use super::system;
use super::*;
use gpui::AnyElement;
use gpui_component::input::{Input, InputEvent, InputState};
use serde_json::Value;
use std::collections::HashMap;

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
        FleetKey5,
        /// R7.1: Shift+1, the positive answer's variant (Allow all).
        FleetVariant
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
    /// Columns the grid was last drawn with, for Up and Down (1 in the
    /// table).
    pub(super) columns: usize,
    /// r5-fleet: each session's last few tools (tool, summary), oldest
    /// first: a working tile's live tail.
    pub(super) recent: HashMap<String, Vec<(String, String)>>,
    /// r5-fleet: the answer field of each text-question tile, by session.
    pub(super) answers: HashMap<String, (Entity<InputState>, gpui::Subscription)>,
    /// r5-fleet: the "Always allow" grants sent from this window (Session
    /// details lists them; see `grants.rs`).
    pub(super) grants: Vec<super::grants::Granted>,
    /// r6-decisions (R6.7): the order of the tiles.
    pub(super) sort: Sort,
    /// r6-decisions (R6.7): while the Fleet is on screen the sidebar is its
    /// rail; this holds whether it was collapsed before, to restore on the
    /// way out.
    pub(super) rail: Option<bool>,
}

/// r6-decisions (R6.7): the Fleet's order. By status (the sidebar's: Needs
/// you oldest wait first, then Working in the hub's order, …), or Waiting
/// longest: what waits on you first, longest first, then every other
/// session by how long it has been quiet, longest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Sort {
    #[default]
    Status,
    Waiting,
}

/// R6.7 batch approve: the positions (among `tiles`) of every approval that
/// asks for exactly what tile `n` asks (the same command in the same
/// folder), when there is more than one and none of them needs a second
/// press. Empty otherwise.
pub(super) fn batch_of(tiles: &[Tile], n: usize) -> Vec<usize> {
    let Some(Ask::Approval {
        command,
        folder,
        risk,
        ..
    }) = tiles.get(n).and_then(|t| t.ask.as_ref())
    else {
        return Vec::new();
    };
    let command = command.trim();
    if command.is_empty() || risk.is_some_and(super::decision::Risk::confirms) {
        return Vec::new();
    }
    let same: Vec<usize> = tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            matches!(&t.ask, Some(Ask::Approval { command: c, folder: f, .. })
                if c.trim() == command && f == folder)
        })
        .map(|(ix, _)| ix)
        .collect();
    if same.len() > 1 { same } else { Vec::new() }
}

/// R7.1: what number key `n` does on an approval tile whose keyed actions
/// are `keys`: the one key map, never shifted by a batch (the batch is
/// Shift+1, [`super::decision::VARIANT_KEY`]).
pub(super) fn key_at(keys: &[StripKey], n: usize) -> Option<StripKey> {
    keys.iter().find(|k| k.number() == Some(n)).cloned()
}

/// The tiles in `sort` order (stable: ties keep the sidebar's order).
/// Waiting longest keeps what waits on you first (the proposals sit after
/// those tiles in both orders), longest wait first, then the rest by how
/// long each has been quiet, longest first; an unknown time goes last.
pub(super) fn sort_tiles(mut tiles: Vec<Tile>, sort: Sort) -> Vec<Tile> {
    if sort == Sort::Waiting {
        tiles.sort_by_key(|t| (t.bucket != Bucket::NeedsYou, t.since.unwrap_or(i64::MAX)));
    }
    tiles
}

/// The batch's label: "Allow all 3 · cargo test" (a long command cut).
pub(crate) fn batch_label(count: usize, command: &str) -> String {
    format!(
        "Allow all {count} · {}",
        wks_native::format::truncate_end(command.trim(), 28)
            .unwrap_or_else(|| command.trim().to_owned())
    )
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
    // r7-status (R7.3): Ready for review needs you too.
    match filter {
        Filter::NeedsYou => bucket.wants_you(),
        Filter::Active => bucket.wants_you() || bucket == Bucket::Working,
        Filter::All => true,
    }
}

/// The header's one-line count of the fleet: "2 need you · 1 to review ·
/// 3 working · 5 idle · 20 ended" (empty groups left out). Each number is
/// the count of a group of top-level sessions, job proposals in Needs you:
/// the filters' counts are sums of these ([`filter_tip`]).
pub(crate) fn summary(counts: [usize; 5]) -> String {
    let parts: Vec<String> = Bucket::ALL
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(bucket, n)| match bucket {
            Bucket::NeedsYou if n == 1 => "1 needs you".to_owned(),
            Bucket::NeedsYou => format!("{n} need you"),
            Bucket::Review => format!("{n} to review"),
            other => format!("{n} {}", other.label().to_lowercase()),
        })
        .collect();
    if parts.is_empty() {
        "No sessions yet".into()
    } else {
        wks_native::format::meta(parts)
    }
}

/// r7-status (R7.4): what a filter's count counts, in the summary line's
/// own words, for its tooltip.
pub(crate) fn filter_tip(filter: Filter) -> &'static str {
    match filter {
        Filter::NeedsYou => {
            "Needs you: sessions waiting on an answer or a fix, plus those ready for review and job proposals"
        }
        Filter::Active => "Active: the Needs you count plus the sessions working now",
        Filter::All => "All: every session listed, idle and ended included",
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
    /// the provider's label, the exact rule shown, tooltip), the keys in
    /// button order (Allow, each grant, Deny), the full request for Details
    /// and its risk cue.
    Approval {
        headline: Vec<(String, bool)>,
        context: Option<String>,
        request: Option<String>,
        folder: String,
        host: String,
        always: Vec<(String, String, String, String)>,
        keys: Vec<StripKey>,
        facts: Vec<(String, String, bool)>,
        command: String,
        risk: Option<super::decision::Risk>,
        /// R7.7: the session's access mode in words, for Details.
        access: String,
    },
    /// r5-fleet (R5.4): one question, answered on the tile: its options
    /// (label, description) as a list (keys 1…n send one), or with none an
    /// inline field (1 focuses it, Enter sends). `card` binds the answer.
    Question {
        headline: String,
        options: Vec<(String, Option<String>)>,
        card: Option<String>,
    },
    /// A plan, or a question set the tile cannot answer (several
    /// questions, multiple choice, a secret, an MCP form): its headline,
    /// answered in the session (1 opens it).
    Open {
        headline: String,
        verb: &'static str,
    },
    /// r6-decisions (R6.2): a subagent that stopped answering: 1 Nudge,
    /// 2 Stop, and View output (no key, R7.1).
    Stalled(super::children::Stall),
}

impl Ask {
    /// The ask in one line, for the table's Needs column.
    pub(super) fn text(&self) -> String {
        match self {
            Self::Approval { headline, .. } => super::decision::headline_text(headline)
                .0
                .replace('\u{2009}', ""),
            Self::Question { headline, .. } | Self::Open { headline, .. } => headline.clone(),
            Self::Stalled(stall) => stall.reason(timing::now_ms()),
        }
    }
}

/// A question's options: (label, description).
pub(super) type Options = Vec<(String, Option<String>)>;

/// The question a tile answers itself: exactly one, single choice, not
/// secret and not an MCP form. (headline, options).
pub(super) fn answerable(
    questions: &Value,
    elicitation: Option<&str>,
) -> Option<(String, Options)> {
    if elicitation.is_some() {
        return None;
    }
    let list = super::questions::question_list(questions);
    let [question] = list.as_slice() else {
        return None;
    };
    if question["multiSelect"].as_bool() == Some(true) || super::questions::secret(question) {
        return None;
    }
    let headline = question["question"]
        .as_str()
        .or(question["header"].as_str())
        .map(str::trim)
        .filter(|q| !q.is_empty())?
        .to_owned();
    let options = question["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let label = o["label"].as_str().or(o.as_str())?.trim();
            let description = o["description"]
                .as_str()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_owned);
            (!label.is_empty()).then(|| (label.to_owned(), description))
        })
        .collect();
    Some((headline, options))
}

/// Most tool lines a working tile keeps as its live tail.
const TAIL: usize = 3;

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
    /// R6.7: since when it has waited on you (Needs you) or been quiet
    /// (the rest), epoch ms: the Waiting-longest order.
    pub since: Option<i64>,
    /// R6.2: a stalled subagent of it, also when another ask leads.
    pub stall: Option<super::children::Stall>,
}

impl Workspace {
    /// Every session the fleet knows (the sidebar's visible set) in the
    /// sidebar's own order and groups (r4-truth): each tree where its most
    /// urgent member puts it, Needs you oldest wait first, Working in the
    /// hub's order, then Idle and Ended most recent first. `(index, depth,
    /// the tree's group)`.
    pub(super) fn fleet_sessions(&self, cx: &App) -> Vec<(usize, usize, Bucket)> {
        let visible = self.visible_sessions(cx);
        self.attention_groups(&visible, timing::now_ms())
            .into_iter()
            .flat_map(|(bucket, members)| {
                members
                    .into_iter()
                    .map(move |(ix, depth)| (ix, depth, bucket))
            })
            .collect()
    }

    /// The tiles on screen, in grid order: one per top-level tree, in the
    /// chosen [`Sort`].
    pub(super) fn fleet_tiles(&self, cx: &App) -> Vec<Tile> {
        let now = timing::now_ms();
        let all = self.fleet_sessions(cx);
        let tiles = all
            .iter()
            .enumerate()
            // r4-integration: one tile per tree, as the sidebar and the
            // filter counts have it; a nested session lives on its root's
            // tile (the subagents line), not on a second tile of its own.
            .filter(|(_, (_, depth, bucket))| *depth == 0 && shown(*bucket, self.fleet.filter))
            .map(|(at, &(ix, depth, _))| {
                // The session and the sessions nested under it.
                let subtree: Vec<(usize, usize)> = std::iter::once((ix, depth))
                    .chain(
                        all[at + 1..]
                            .iter()
                            .take_while(|(_, d, _)| *d > depth)
                            .map(|&(ix, d, _)| (ix, d)),
                    )
                    .collect();
                self.fleet_tile(&subtree, now)
            })
            .collect();
        sort_tiles(tiles, self.fleet.sort)
    }

    /// The job proposals the Fleet shows as cards under its filter (R5.4:
    /// they count in Needs you, so they show there).
    pub(super) fn fleet_jobs(&self) -> Vec<wks_native::jobs::Job> {
        if shown(Bucket::NeedsYou, self.fleet.filter) {
            self.job_proposals()
        } else {
            Vec::new()
        }
    }

    fn fleet_tile(&self, subtree: &[(usize, usize)], now: i64) -> Tile {
        let index = subtree[0].0;
        let session = &self.view.sessions[index];
        // r4-truth: the one status, and the group it counts in on its own.
        let status = self.session_status(session, now);
        let (kind, bucket) = (status.kind, status.bucket);
        let since = self.last_heard(session);
        let time = if kind == StatusKind::Working {
            self.working_elapsed(session, now)
        } else if bucket == Bucket::NeedsYou {
            // R6.7: how long it has waited on you.
            since.map(|at| format!("Waiting {}", wks_native::format::age(at, now)))
        } else {
            since.map(|at| wks_native::format::age(at, now))
        };
        let reason = self.inbox_reason(session, now);
        let stall = self.stalled_child(session, now);
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
                // shows (R4.3).
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
                    // R7.1: Deny with note (4) wherever the provider
                    // reads a note; it opens the note in the session.
                    keys: strip_keys(
                        always.len(),
                        matches!(session.provider_id(), "claude" | "codex"),
                    ),
                    always,
                    facts: approval_facts(approval, &session.cwd, &host),
                    command: super::composer_v2::approval_summary(approval).1,
                    risk: super::decision::risk(approval, &session.cwd),
                    access: super::decision::access_line(session),
                }
            })
            .or_else(|| {
                let questions = session.questions.as_ref().filter(|_| self.view.connected)?;
                Some(
                    match answerable(questions, session.elicitation_server.as_deref()) {
                        Some((headline, options)) => Ask::Question {
                            headline,
                            options,
                            card: session.card(),
                        },
                        None => Ask::Open {
                            headline: reason
                                .as_ref()
                                .map(|r| r.text.clone())
                                .unwrap_or_else(|| "Questions wait for your answers".into()),
                            verb: "Open to answer",
                        },
                    },
                )
            })
            .or_else(|| {
                reason
                    .as_ref()
                    .filter(|r| r.action == attention::InboxAction::Answer)
                    .map(|r| Ask::Open {
                        headline: r.text.clone(),
                        verb: "Open to answer",
                    })
            })
            // R6.2: a stalled subagent, when nothing else asks.
            .or_else(|| stall.clone().map(Ask::Stalled));
        let mut lines = self.fleet_lines(session, subtree, kind == StatusKind::Working);
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
            since,
            stall,
        }
    }

    /// Remember each session's latest tool as it changes, so a working
    /// tile shows its last few tools live (R5.4), not only the latest.
    fn record_fleet_tools(&mut self) {
        let sessions = self.view.sessions.clone();
        let recent = &mut self.fleet.recent;
        recent.retain(|id, _| sessions.iter().any(|s| &s.id == id));
        for session in sessions.iter() {
            let tool = session.telemetry.last_tool_name.trim();
            if tool.is_empty() {
                continue;
            }
            let entry = (
                tool.to_owned(),
                session.telemetry.last_tool_summary.trim().to_owned(),
            );
            let tail = recent.entry(session.id.clone()).or_default();
            if tail.last() != Some(&entry) {
                tail.push(entry);
                if tail.len() > TAIL {
                    tail.remove(0);
                }
            }
        }
    }

    /// What the session row reports it is doing, failures first and each
    /// one named: a subagent that stopped answering or failed, a failed
    /// background command; then its tools (a working session's last few,
    /// newest last), the plan step in progress, the subagents at work and
    /// the background work still running.
    fn fleet_lines(
        &self,
        session: &Session,
        subtree: &[(usize, usize)],
        working: bool,
    ) -> Vec<Line> {
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
        // r4-truth: only a failure still open (the header chip's own).
        if let Some(failed) = self
            .open_failures(session)
            .into_iter()
            .find(|t| t.kind != wks_native::background_tasks::Kind::Agent)
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
        let summary = session.telemetry.last_tool_summary.trim();
        let mut tools: Vec<(String, String)> = match self.fleet.recent.get(&session.id) {
            Some(tail) if working => tail.clone(),
            _ => Vec::new(),
        };
        let latest = (tool.to_owned(), summary.to_owned());
        if !tool.is_empty() && tools.last() != Some(&latest) {
            tools.push(latest);
        }
        let keep = if working { TAIL } else { 1 };
        for (tool, summary) in tools.iter().skip(tools.len().saturating_sub(keep)) {
            let (icon, verb) = tool_line(tool);
            lines.push(Line {
                icon,
                verb,
                argument: (!summary.is_empty()).then(|| summary.clone()),
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
        // r4-truth: the sidebar's own summary of this tree's subagents
        // ("4 subagents · 3 working"), so the two never count differently.
        if let Some((line, _)) = self
            .subagents_line(subtree, now)
            .filter(|(line, fault)| *fault || line.ends_with("working"))
        {
            lines.push(Line {
                icon: IconName::Bot,
                verb: line,
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

    /// The focused tile's position among `tiles` (the first card by
    /// default); `usize::MAX` (no tile) while a job proposal has the focus.
    pub(super) fn fleet_focus(&self, tiles: &[Tile]) -> usize {
        match self.fleet_cursor(tiles, &self.fleet_jobs()) {
            Cursor::Tile(n) => n,
            Cursor::Job(_) | Cursor::None => usize::MAX,
        }
    }

    /// r5-integration: which card has the focus, tiles and job proposals
    /// alike (proposals take the focus as tiles do). A stale or unset focus
    /// falls to the first card on screen.
    fn fleet_cursor(&self, tiles: &[Tile], jobs: &[wks_native::jobs::Job]) -> Cursor {
        let slots = fleet_slots(tiles, jobs);
        let at = self
            .fleet
            .focus
            .as_ref()
            .and_then(|id| slots.iter().position(|s| s == id))
            .unwrap_or(0);
        match slots.get(at) {
            None => Cursor::None,
            Some(id) => match id.strip_prefix(JOB_FOCUS) {
                Some(job) => jobs
                    .iter()
                    .position(|j| j.id == job)
                    .map_or(Cursor::None, Cursor::Job),
                None => tiles
                    .iter()
                    .position(|t| &t.id == id)
                    .map_or(Cursor::None, Cursor::Tile),
            },
        }
    }

    /// R6.7: the sidebar is its rail while the Fleet is on screen (the grid
    /// already shows what the sidebar's groups list), and comes back as it
    /// was on the way out. Opening it again on the Fleet is respected until
    /// the Fleet is left. Called every frame.
    pub(super) fn track_fleet_rail(&mut self) {
        let on = self.screen == Screen::Fleet && !self.new_session && self.design().is_v2();
        match (on, self.fleet.rail) {
            (true, None) => {
                self.fleet.rail = Some(self.sidebar_collapsed);
                self.sidebar_collapsed = true;
            }
            (false, Some(was)) => {
                self.fleet.rail = None;
                self.sidebar_collapsed = was;
            }
            _ => {}
        }
    }

    pub(super) fn open_fleet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.design().is_v2() {
            self.show_screen(Screen::Fleet, window, cx);
        }
    }

    fn fleet_move(&mut self, dir: Move, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let slots = fleet_slots(&tiles, &self.fleet_jobs());
        if slots.is_empty() {
            return;
        }
        let from = self
            .fleet
            .focus
            .as_ref()
            .and_then(|id| slots.iter().position(|s| s == id))
            .unwrap_or(0);
        let at = step(from, slots.len(), self.fleet.columns, dir);
        self.fleet.focus = Some(slots[at].clone());
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
    /// Deny and Always allow in reading order; a question's option `n`
    /// (sent at once), or 1 into its answer field; a plan's 1 opens the
    /// session at it.
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
        let ready = self.view.connected && !self.view.busy;
        match &tile.ask {
            Some(Ask::Approval { keys, risk, .. }) => {
                // R7.1: the one key map (1 Allow, 2 Deny, 3 Always allow,
                // 4 Deny with note); the batch is Shift+1.
                let Some(key) = key_at(keys, n) else {
                    return;
                };
                if !ready {
                    return;
                }
                if key == StripKey::DenyWithNote {
                    self.fleet_deny_with_note(tile.index, window, cx);
                    return;
                }
                if let StripKey::Always(ix) = key {
                    self.fleet_grant(position, ix, cx);
                    return;
                }
                let card = self.view.sessions[tile.index].card();
                if key == StripKey::Allow
                    && !self.confirm_allow(
                        &super::decision::confirm_key(&tile.id, card.as_deref()),
                        *risk,
                        cx,
                    )
                {
                    // R6.3: armed; the second press allows it.
                    return;
                }
                let action = match key {
                    StripKey::Allow => Action::Approve(true, card),
                    StripKey::Deny => Action::Approve(false, card),
                    StripKey::Always(_) | StripKey::DenyWithNote => return,
                };
                self.fleet_act(tile.index, action, cx);
                // Decided: the focus moves on to the next tile that asks
                // something, so 1, 1, 1 clears a run of approvals.
                if let Some(next) = next_asking(&tiles, position) {
                    self.fleet.focus = Some(next);
                }
            }
            Some(Ask::Question { options, card, .. }) if !options.is_empty() => {
                let Some((label, _)) = n.checked_sub(1).and_then(|ix| options.get(ix)) else {
                    return;
                };
                if !ready {
                    return;
                }
                self.fleet_act(
                    tile.index,
                    Action::Answers(vec![label.clone()], card.clone()),
                    cx,
                );
                if let Some(next) = next_asking(&tiles, position) {
                    self.fleet.focus = Some(next);
                }
            }
            Some(Ask::Question { .. }) if n == 1 => {
                if let Some((input, _)) = self.fleet.answers.get(&tile.id) {
                    let input = input.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
            }
            Some(Ask::Open { .. }) if n == 1 => self.open_listed_session(tile.index, cx),
            Some(Ask::Stalled(stall)) => {
                if let Some(key) = super::children::StallKey::nth(n) {
                    self.stall_act(stall, key, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// What a stall decision's buttons run on the Fleet: the action, on
    /// the stall as it stands when pressed.
    fn fleet_stall_key(&self, stall: super::children::Stall) -> super::children::OnStallKey {
        std::rc::Rc::new(move |this: &mut Self, key, cx: &mut Context<Self>| {
            this.fleet.focus = Some(stall.parent.clone());
            this.stall_act(&stall, key, cx);
        })
    }

    /// Grant `ix` ("Always allow") of tile `position`'s approval; the focus
    /// moves on to the next ask.
    fn fleet_grant(&mut self, position: usize, ix: usize, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let Some(tile) = tiles.get(position) else {
            return;
        };
        let Some(Ask::Approval { always, .. }) = &tile.ask else {
            return;
        };
        if !self.view.connected || self.view.busy {
            return;
        }
        let card = self.view.sessions[tile.index].card();
        if let (Some((option, label, _, _)), Some(card)) = (always.get(ix), card) {
            let action = Action::AlwaysAllow {
                option: option.clone(),
                label: label.clone(),
                card,
            };
            self.fleet.focus = Some(tile.id.clone());
            self.fleet_act(tile.index, action, cx);
            if let Some(next) = next_asking(&tiles, position) {
                self.fleet.focus = Some(next);
            }
            cx.notify();
        }
    }

    /// R7.1: Deny with note from the Fleet: the session opens with its
    /// note field ready (the note needs room the tile does not have).
    fn fleet_deny_with_note(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let card = self.view.sessions[index].card();
        self.open_listed_session(index, cx);
        self.open_deny_note(card, window, cx);
    }

    /// R7.1: Shift+1 on the focused tile: the positive answer's variant,
    /// "Allow all" for a batch of identical requests.
    fn fleet_variant(&mut self, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let jobs = self.fleet_jobs();
        let Cursor::Tile(position) = self.fleet_cursor(&tiles, &jobs) else {
            return;
        };
        let batch = batch_of(&tiles, position);
        if !batch.is_empty() && self.view.connected && !self.view.busy {
            self.fleet_batch(&tiles, &batch, cx);
        }
    }

    /// R6.7: allow every approval in `batch` (positions among `tiles`, the
    /// same command in the same folder), each on its own card; the focus
    /// moves on to the first ask left.
    fn fleet_batch(&mut self, tiles: &[Tile], batch: &[usize], cx: &mut Context<Self>) {
        for at in batch {
            let tile = &tiles[*at];
            let card = self.view.sessions[tile.index].card();
            self.fleet_act(tile.index, Action::Approve(true, card), cx);
        }
        self.fleet.focus = tiles
            .iter()
            .enumerate()
            .find(|(ix, t)| !batch.contains(ix) && t.ask.is_some())
            .map(|(_, t)| t.id.clone());
        cx.notify();
    }

    /// Send the typed answer of question tile `id` (Enter, or its Send
    /// answer): the session's card, then the focus back on the grid and on
    /// to the next ask.
    fn submit_fleet_answer(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let Some(position) = tiles.iter().position(|t| t.id == id) else {
            return;
        };
        let tile = &tiles[position];
        let Some(Ask::Question { card, .. }) = &tile.ask else {
            return;
        };
        let Some((input, _)) = self.fleet.answers.get(id) else {
            return;
        };
        let input = input.clone();
        let text = input.read(cx).value().trim().to_owned();
        if text.is_empty() || !self.view.connected || self.view.busy {
            return;
        }
        self.fleet_act(tile.index, Action::Answers(vec![text], card.clone()), cx);
        input.update(cx, |input, cx| input.set_value("", window, cx));
        window.focus(&self.focus);
        self.fleet.focus = next_asking(&tiles, position).or(Some(id.to_owned()));
        cx.notify();
    }

    /// One answer field per text-question tile on screen, made as tiles
    /// need them and dropped with their question.
    fn sync_fleet_answers(&mut self, tiles: &[Tile], window: &mut Window, cx: &mut Context<Self>) {
        let wanted: Vec<&str> = tiles
            .iter()
            .filter(|t| matches!(&t.ask, Some(Ask::Question { options, .. }) if options.is_empty()))
            .map(|t| t.id.as_str())
            .collect();
        self.fleet
            .answers
            .retain(|id, _| wanted.contains(&id.as_str()));
        for id in wanted {
            if self.fleet.answers.contains_key(id) {
                continue;
            }
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type your answer"));
            let owner = id.to_owned();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { secondary: false } => {
                        this.submit_fleet_answer(&owner, window, cx)
                    }
                    _ => {}
                },
            );
            self.fleet
                .answers
                .insert(id.to_owned(), (input, subscription));
        }
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
                let jobs = this.fleet_jobs();
                match this.fleet_cursor(&tiles, &jobs) {
                    Cursor::Tile(at) => this.fleet_open(at, cx),
                    // A proposal opens (or closes) its Details in place.
                    Cursor::Job(ix) => this.toggle_job(&jobs[ix].id, cx),
                    Cursor::None => {}
                }
            }))
            .on_action(cx.listener(|this, _: &FleetKey1, window, cx| this.fleet_key(1, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey2, window, cx| this.fleet_key(2, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey3, window, cx| this.fleet_key(3, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey4, window, cx| this.fleet_key(4, window, cx)))
            .on_action(cx.listener(|this, _: &FleetKey5, window, cx| this.fleet_key(5, window, cx)))
            .on_action(cx.listener(|this, _: &FleetVariant, _, cx| this.fleet_variant(cx)))
    }

    fn fleet_key(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tiles = self.fleet_tiles(cx);
        let jobs = self.fleet_jobs();
        match self.fleet_cursor(&tiles, &jobs) {
            Cursor::Tile(at) => self.fleet_answer(at, n, window, cx),
            Cursor::Job(ix) => {
                let all = self.job_rows().unwrap_or_default();
                self.job_proposal_key(&jobs[ix], &all, n, cx);
            }
            Cursor::None => {}
        }
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
        let table = self.settings.fleet_table;
        // The page's room: the window less the sidebar and the page gutters.
        let width = unzoom(window.viewport_size().width);
        let room = (width - self.sidebar_footprint(width) - 2. * t.space.xxl).min(f.grid_max);
        let cols = if table {
            1
        } else {
            columns(room, f.tile_min, t.space.lg, f.columns_max)
        };
        self.fleet.columns = cols;
        self.record_fleet_tools();
        // r4-truth: the sidebar's counts (top-level trees per group, job
        // proposals in Needs you), never a tally of its own. R5.4: every
        // count is what its filter shows, proposals included (as cards).
        let groups = self.fleet_sessions(cx);
        let counts = self.attention_counts(cx);
        let all_count: usize = counts.iter().sum();
        let needs = counts[Bucket::NeedsYou.index()] + counts[Bucket::Review.index()];
        let active = needs + counts[Bucket::Working.index()];
        let quiet = all_count - active;
        let tiles = self.fleet_tiles(cx);
        let jobs = self.fleet_jobs();
        self.sync_fleet_answers(&tiles, window, cx);
        let focus = self.fleet_focus(&tiles);
        let focused_job = match self.fleet_cursor(&tiles, &jobs) {
            Cursor::Job(ix) => Some(ix),
            _ => None,
        };
        let current = self.fleet.filter;
        let all = current == Filter::All;
        let back = self.page_back_v2(cx);
        // R6.7: the system's SegmentedControl, so the chosen filter, view
        // and order read as chosen.
        let filter = |id: &'static str, label: &'static str, on: Filter, n: usize| {
            system::Segment::new(id, label)
                .count(n)
                .selected(current == on)
                .render(&p)
                .debug_selector(move || id.into())
                .tooltip(primitives::tooltip_view(filter_tip(on), None, &p))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.fleet.filter = on;
                    window.focus(&this.focus);
                    cx.notify();
                }))
        };
        let sort = self.fleet.sort;
        let order = |id: &'static str, label: &'static str, on: Sort| {
            system::Segment::new(id, label)
                .selected(sort == on)
                .render(&p)
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.fleet.sort = on;
                    window.focus(&this.focus);
                    cx.notify();
                }))
        };
        let view = |id: &'static str, label: &'static str, on: bool| {
            system::Segment::new(id, label)
                .selected(table == on)
                .render(&p)
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.settings.fleet_table != on {
                        this.settings.fleet_table = on;
                        this.save_settings(cx);
                    }
                    window.focus(&this.focus);
                    cx.notify();
                }))
        };
        let header = primitives::page_header(
            back,
            "Fleet",
            Some(summary(counts).into()),
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .child(
                        system::segmented_control(&p)
                            .child(view("fleet-view-grid", "Grid", false))
                            .child(view("fleet-view-table", "Table", true)),
                    )
                    .child(
                        system::segmented_control(&p)
                            .child(order("fleet-sort-status", "By status", Sort::Status))
                            .child(order(
                                "fleet-sort-waiting",
                                "Waiting longest",
                                Sort::Waiting,
                            )),
                    )
                    .child(
                        system::segmented_control(&p)
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
                            .child(filter("fleet-filter-all", "All", Filter::All, all_count)),
                    )
                    .into_any_element(),
            ],
            &p,
        )
        .w_auto();
        let body: AnyElement = if tiles.is_empty() && jobs.is_empty() {
            let (title, body) = if groups.is_empty() && all_count == 0 {
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
                (Filter::All, format!("Show all {all_count}"))
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
            let all_jobs = self.job_rows().unwrap_or_default();
            let content: AnyElement = if table {
                self.render_fleet_table(&tiles, &jobs, &all_jobs, focus, focused_job, cx)
                    .into_any_element()
            } else {
                // R5.4: tiles at their natural height, top-aligned: a short
                // tile is never a tall empty box. Job proposals are cards
                // too, after the sessions that wait on you.
                let waiting = tiles
                    .iter()
                    .take_while(|t| t.bucket == Bucket::NeedsYou)
                    .count();
                let mut cards: Vec<AnyElement> = Vec::new();
                for (n, tile) in tiles.iter().enumerate().take(waiting) {
                    cards.push(self.render_fleet_tile(
                        n,
                        tile,
                        n == focus,
                        batch_of(&tiles, n).len(),
                        cx,
                    ));
                }
                for (ix, job) in jobs.iter().enumerate() {
                    cards.push(self.render_fleet_job(
                        ix,
                        job,
                        &all_jobs,
                        focused_job == Some(ix),
                        cx,
                    ));
                }
                for (n, tile) in tiles.iter().enumerate().skip(waiting) {
                    cards.push(self.render_fleet_tile(
                        n,
                        tile,
                        n == focus,
                        batch_of(&tiles, n).len(),
                        cx,
                    ));
                }
                // R7.6: each card is as tall as its content; a row without
                // a decision does not stretch to match one with it.
                system::tile_rows(cols, cards, &p)
                    .debug_selector(|| "fleet-grid".into())
                    .into_any_element()
            };
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
                .child(content)
                .into_any_element()
        };
        // R7.1: the hint lists exactly the keys the focused card shows.
        let caps = match self.fleet_cursor(&tiles, &jobs) {
            Cursor::Tile(at) => focused_caps(&tiles, at),
            Cursor::Job(_) => vec!["1".to_owned(), "2".to_owned()],
            Cursor::None => Vec::new(),
        };
        // r5-shell (R5.4): the shared PageShell, at the grid's width.
        let content = system::page_shell(
            header,
            None,
            div()
                .flex()
                .flex_col()
                .gap(px(t.space.xxl))
                .child(body)
                .child(self.fleet_hints(&caps, table)),
            system::PageWidth::Grid,
            &p,
        )
        .debug_selector(|| "fleet-page".into());
        self.page_view("fleet-scroll", f.grid_max, false, content)
    }

    /// The keys, once, under the grid: faint keycaps and plain words. The
    /// answer keys are exactly the focused card's (R7.1), and show only
    /// while it asks.
    fn fleet_hints(&self, caps: &[String], table: bool) -> Div {
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
            .child(if table {
                hint(&["↑", "↓"], "Move")
            } else {
                hint(&["←", "↑", "↓", "→"], "Move")
            })
            .child(hint(&["↵"], "Open"))
            .when(!caps.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "fleet-hint-answer".into())
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .children(
                            caps.iter()
                                .map(|k| primitives::keycap_hint(k.clone(), false, &p)),
                        )
                        .child(div().pl(px(t.space.xxs)).child("Answer the focused ask")),
                )
            })
            .child(hint(&["Esc"], "Back"))
    }

    /// A tile's first lines: the title (it truncates) and the time, then
    /// the status (a StatusCell), project, model and context.
    fn fleet_tile_head(&self, n: usize, tile: &Tile) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let kind = tile.kind;
        let ended = tile.bucket == Bucket::Ended;
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
                    .child(
                        system::StatusCell::new(kind)
                            .render(&p)
                            .debug_selector(move || format!("fleet-status-{n}")),
                    ),
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
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .child(head)
            .child(meta)
    }

    fn render_fleet_tile(
        &self,
        n: usize,
        tile: &Tile,
        focused: bool,
        batch: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let f = t.fleet();
        let index = tile.index;
        let waiting = tile.kind == StatusKind::NeedsYou;
        let lines: Vec<AnyElement> = if tile.lines.is_empty() && tile.ask.is_none() {
            vec![
                div()
                    .h(px(f.event_row))
                    .flex()
                    .items_center()
                    .text_size(px(t.text.body))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(match tile.bucket {
                        Bucket::Ended => "Ended".to_owned(),
                        Bucket::Working => "Starting…".to_owned(),
                        Bucket::NeedsYou => "Waiting for you".to_owned(),
                        // r7-status (R7.3): what waits for review.
                        Bucket::Review => self.view.sessions[index].uncommitted.map_or_else(
                            || "Changes to review".to_owned(),
                            super::shell::review_reason,
                        ),
                        Bucket::Idle => "Ready for your next message".to_owned(),
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
                    // r6-qa: a fault's glyph leads, beside the words it
                    // qualifies, not at the tile's far edge.
                    if let Some(fault) = line.fault {
                        row = row.lead_status(fault);
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
            .map(|ask| self.render_fleet_ask(n, tile, ask, focused, batch, cx));
        // R6.2: a stalled subagent behind another ask still offers its
        // Nudge / Stop / View output (unkeyed: the keys answer the ask).
        let stall = tile
            .stall
            .as_ref()
            .filter(|_| !matches!(tile.ask, Some(Ask::Stalled(_)) | None))
            .map(|stall| {
                let mut decision = self.stall_decision(
                    stall,
                    &format!("fleet-stall-{n}"),
                    super::decision::Frame::Row,
                    self.fleet_stall_key(stall.clone()),
                    cx,
                );
                decision.keys = false;
                // One Primary per card: the ask above owns it.
                decision.allow_primary = false;
                // Its line above says what stalled; this row acts on it.
                div()
                    .debug_selector(move || format!("fleet-stall-row-{n}"))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .child(decision.render(&p))
            });
        // R5.4: the ask sits near the top, under the title and status; the
        // event lines (a working tile's live tail) follow. R6.3: one accent
        // per card: focused, the focus ring carries it and the attention
        // bar steps aside.
        let surface = match (waiting, focused) {
            (true, false) => system::waiting_tile(&p),
            (true, true) => system::waiting_tile_focused(&p),
            _ => system::tile(&p),
        };
        let card = surface
            .flex_1()
            .child(self.fleet_tile_head(n, tile))
            // R7.3: the session's latest test result, under its status.
            .children(
                self.session_verification(&self.view.sessions[index])
                    .map(|v| {
                        system::verification_line(&v, timing::now_ms(), &p)
                            .debug_selector(move || format!("fleet-verification-{n}"))
                    }),
            )
            .children(ask)
            .when(!lines.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("fleet-lines-{n}"))
                        .flex()
                        .flex_col()
                        .children(lines),
                )
            })
            .children(stall);
        system::tile_frame(
            SharedString::from(format!("fleet-tile-{}", tile.id)),
            focused,
            &p,
        )
        .debug_selector(move || format!("fleet-tile-{n}"))
        .flex()
        .flex_col()
        .when(focused, |d| {
            d.child(div().debug_selector(move || format!("fleet-focused-{n}")))
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

    /// A job proposal as a Fleet card (R5.4): the job, then the one
    /// decision component inline (Approve, Reject, Details). It takes the
    /// keyboard focus as a tile does (r5-integration): the focus ring, its
    /// keycaps (1 Approve, 2 Reject), Enter for Details.
    fn render_fleet_job(
        &self,
        ix: usize,
        job: &wks_native::jobs::Job,
        all: &[wks_native::jobs::Job],
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let head = div()
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child(job.name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(system::StatusCell::new(StatusKind::NeedsYou).render(&p)),
                    )
                    .child("·")
                    .child(div().flex_shrink_0().child("Job proposal")),
            );
        let mut decision = self.job_proposal_decision(
            ix,
            job,
            all,
            "fleet-job",
            super::decision::Frame::Inline,
            cx,
        );
        // R7.1/R7.2: keys and the Filled Allow only on the focused card.
        decision.keys = focused;
        decision.filled = focused;
        let focus = format!("{JOB_FOCUS}{}", job.id);
        let surface = if focused {
            system::waiting_tile_focused(&p)
        } else {
            system::waiting_tile(&p)
        };
        system::tile_frame(("fleet-job-card", ix), focused, &p)
            .debug_selector(move || format!("fleet-job-{ix}"))
            .flex()
            .flex_col()
            .when(focused, |d| {
                d.child(div().debug_selector(move || format!("fleet-job-focused-{ix}")))
            })
            .child(surface.flex_1().child(head).child(decision.render(&p)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.fleet.focus = Some(focus.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    /// A tile's ask, drawn by the one decision component (R5.4): the
    /// request as a headline, why, a long command whole, where it runs and
    /// its risk, then Allow (Primary) and Deny (Destructive) side by side,
    /// each "Always allow" with the rule it saves, and Details below its
    /// toggle. A question is answered here: an option list or a field.
    /// Keycaps show only on the focused tile, where the number keys go.
    fn render_fleet_ask(
        &self,
        n: usize,
        tile: &Tile,
        ask: &Ask,
        focused: bool,
        batch: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use super::decision::{Choice, Decision, Details, Frame, Grant};
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
                command,
                risk,
                access,
            } => {
                // R7.1: the one key map; a key the card does not offer
                // stays empty.
                let number = |key: &StripKey| keys.contains(key).then(|| key.number()).flatten();
                let allow = choice(
                    format!("fleet-allow-{n}"),
                    StripKey::Allow.number().unwrap_or(1),
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
                decision.risk = *risk;
                decision.access = Some(access.clone());
                decision.grants = always
                    .iter()
                    .enumerate()
                    .map(|(ix, (_, words, _, tip))| {
                        // The first grant keeps its selector (and slot 3);
                        // a second has no key.
                        let id = if ix == 0 {
                            format!("fleet-always-{n}")
                        } else {
                            format!("fleet-always-{n}-{ix}")
                        };
                        let choice = Choice::new(id, "Always allow")
                            .key(number(&StripKey::Always(ix)))
                            .enabled(enabled)
                            .tooltip(tip.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.fleet_grant(n, ix, cx);
                            }));
                        Grant::new(choice, words, command)
                    })
                    .collect();
                decision.deny_note = number(&StripKey::DenyWithNote).map(|key| {
                    choice(format!("fleet-note-{n}"), key, "Deny with note".into())
                        .tooltip("Deny, and tell the agent what to do instead (opens the session)")
                });
                decision.deny = number(&StripKey::Deny).map(|key| {
                    choice(format!("fleet-deny-{n}"), key, "Deny".into())
                        .tooltip("Deny this request")
                });
                let open = self.fleet.details.as_deref() == Some(tile.id.as_str());
                let id = tile.id.clone();
                decision.details = Some(Details::facts(
                    Choice::new(format!("fleet-details-{n}"), "Details").on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.fleet.focus = Some(id.clone());
                            this.fleet.details = if this.fleet.details.as_ref() == Some(&id) {
                                None
                            } else {
                                Some(id.clone())
                            };
                            cx.notify();
                        },
                    )),
                    open,
                    "fleet-details",
                    "fleet-fact",
                    facts.clone(),
                ));
                // R6.3: a request that deletes files takes a second press.
                let card = self.view.sessions[tile.index].card();
                decision.armed =
                    self.armed(&super::decision::confirm_key(&tile.id, card.as_deref()));
                // R6.7: the same command in the same folder elsewhere: one
                // press allows them all. R7.1: on Shift+1, never a number;
                // nothing that deletes is ever in a batch (`batch_of`).
                if batch > 1 {
                    let id: SharedString = format!("fleet-batch-{n}").into();
                    decision.alternates.push(
                        Choice::new(id, batch_label(batch, command))
                            .variant_key()
                            .enabled(enabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let tiles = this.fleet_tiles(cx);
                                if let Some(tile) = tiles.get(n) {
                                    this.fleet.focus = Some(tile.id.clone());
                                }
                                this.fleet_variant(cx);
                            }))
                            .tooltip(format!(
                                "Allow this command in {folder} for all {batch} sessions that ask for it"
                            )),
                    );
                }
                decision
            }
            Ask::Question {
                headline, options, ..
            } => {
                let send_enabled = enabled
                    && self
                        .fleet
                        .answers
                        .get(&tile.id)
                        .is_some_and(|(input, _)| !input.read(cx).value().trim().is_empty());
                let id = tile.id.clone();
                let send = Choice::new(
                    format!("fleet-answer-send-{n}"),
                    super::decision::SUBMIT_ANSWER,
                )
                .enabled(send_enabled)
                .tooltip("Send the answer to the agent")
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.submit_fleet_answer(&id, window, cx);
                }));
                let mut decision =
                    Decision::new(format!("fleet-decision-{n}"), send, Frame::Inline);
                decision.headline = vec![(headline.clone(), false)];
                decision.headline_selector = Some(format!("fleet-headline-{n}").into());
                if options.is_empty() {
                    decision.body = self.fleet_answer_field(n, &tile.id, cx);
                } else {
                    decision.show_allow = false;
                    decision.body = Some(self.fleet_options(n, options, focused, enabled, cx));
                }
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
                decision
            }
            Ask::Stalled(stall) => self.stall_decision(
                stall,
                &format!("fleet-stall-{n}"),
                Frame::Inline,
                self.fleet_stall_key(stall.clone()),
                cx,
            ),
        };
        // R7.1: keycaps only on the focused tile (where the number keys
        // go); unfocused tiles show none. R7.2: only the focused tile's
        // positive answer is Filled; the rest are Bordered.
        let mut decision = decision;
        decision.keys = focused;
        decision.filled = focused;
        div()
            .debug_selector(move || format!("fleet-ask-{n}"))
            .w_full()
            .min_w_0()
            .pt(px(t.space.xs))
            .child(decision.render(&p))
            .into_any_element()
    }

    /// A question tile's answer field: Enter sends, Esc gives the keys back
    /// to the grid. Clicks in it stay in it (the tile opens on a click).
    fn fleet_answer_field(&self, n: usize, id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette();
        let t = p.tokens;
        let (input, _) = self.fleet.answers.get(id)?;
        Some(
            div()
                .id(("fleet-answer", n))
                .debug_selector(move || format!("fleet-answer-{n}"))
                .w_full()
                .min_w_0()
                .text_size(px(t.text.body))
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        window.focus(&this.focus);
                        cx.notify();
                    }
                }))
                .child(Input::new(input).disabled(!self.view.connected))
                .into_any_element(),
        )
    }

    /// A question's options as a list: label, description, and the number
    /// key on the focused tile. Choosing one sends it.
    fn fleet_options(
        &self,
        n: usize,
        options: &[(String, Option<String>)],
        focused: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        div()
            .debug_selector(move || format!("fleet-options-{n}"))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .children(
                options
                    .iter()
                    .enumerate()
                    .map(|(ix, (label, description))| {
                        let key = ix + 1;
                        primitives::list_row(("fleet-option", n * 64 + ix), false, enabled, &p)
                            .debug_selector(move || format!("fleet-option-{n}-{ix}"))
                            .h_auto()
                            .py(px(t.space.xs))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().min_w_0().child(label.clone()))
                                    .children(description.clone().map(|d| {
                                        div()
                                            .min_w_0()
                                            .text_size(px(t.text.meta))
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child(d)
                                    })),
                            )
                            // R7.1: keycaps only on the focused tile.
                            .when(key <= 5 && focused, |d| {
                                d.child(primitives::keycap_hint(key.to_string(), false, &p))
                            })
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.fleet_answer(n, key, window, cx);
                                }))
                            })
                    }),
            )
            .into_any_element()
    }

    /// The dense view (R5.4): one row per session (and per job proposal),
    /// with its status, title, project, how long it has waited, what it is
    /// doing, what it needs and its actions. Up and Down move the focus
    /// row; Enter opens it; the number keys answer it as on a tile. R6.3:
    /// a row's approve / deny is the DecisionCard's own answer row
    /// (`Frame::Row`): Allow the filled Primary, Deny Destructive, the
    /// keys on every keyed row (full strength on the focused one).
    fn render_fleet_table(
        &self,
        tiles: &[Tile],
        jobs: &[wks_native::jobs::Job],
        all_jobs: &[wks_native::jobs::Job],
        focus: usize,
        focused_job: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Div {
        use super::decision::{Choice, Decision, Frame};
        let p = self.palette();
        let t = p.tokens;
        let pg = t.pages();
        let now = timing::now_ms();
        let slots = primitives::action_slots_width(4, &p);
        let cols = [
            ("Status", Col::Keep(pg.status)),
            ("Session", Col::Flex),
            ("Project", Col::Fixed(pg.machine)),
            ("Waiting", Col::Fixed(pg.age)),
            ("Doing", Col::Flex),
            ("Needs", Col::Flex),
            ("", Col::Keep(slots)),
        ];
        let enabled = self.view.connected && !self.view.busy;
        let text = |s: String, tone: u32| div().min_w_0().truncate().text_color(rgb(tone)).child(s);
        let waiting = tiles
            .iter()
            .take_while(|t| t.bucket == Bucket::NeedsYou)
            .count();
        let job_row = |ix: usize, job: &wks_native::jobs::Job, cx: &mut Context<Self>| {
            let focused = focused_job == Some(ix);
            let focus = format!("{JOB_FOCUS}{}", job.id);
            // R6.3: the proposal's own decision, as its answer row: Allow
            // and Deny on 1 and 2. R7.1/R7.2: keys and the Filled Allow on
            // the focused row only.
            let mut decision =
                self.job_proposal_decision(ix, job, all_jobs, "fleet-job-row", Frame::Row, cx);
            decision.keys = focused;
            decision.filled = focused;
            // R7.1: the Needs cell says what it would run, and its risk
            // ("Deletes branches"), never a bare "Approve this job?".
            let (need, risk) = (job_need(job), decision.risk);
            primitives::page_table_row(&p)
                .id(SharedString::from(format!("fleet-job-row-{}", job.id)))
                .debug_selector(move || format!("fleet-job-row-{ix}"))
                .cursor_pointer()
                .when(focused, |d| d.bg(rgb(p.selected)))
                .when(!focused, |d| d.hover(|s| s.bg(rgb(p.roles.hover))))
                .child(
                    primitives::page_cell(cols[0].1)
                        .child(system::StatusCell::new(StatusKind::NeedsYou).render(&p))
                        .when(focused, |d| {
                            d.child(div().debug_selector(move || format!("fleet-job-focused-{ix}")))
                        }),
                )
                .child(
                    primitives::page_cell(cols[1].1)
                        .font_weight(FontWeight(t.weight.medium))
                        .child(text(job.name.clone(), p.text)),
                )
                .child(
                    primitives::page_cell(cols[2].1)
                        .child(text("Jobs".into(), p.roles.text_secondary)),
                )
                .child(primitives::page_cell(cols[3].1))
                .child(
                    // R7.1: what it would run, whole in the tooltip.
                    primitives::page_cell(cols[4].1)
                        .id(SharedString::from(format!("fleet-job-doing-{ix}")))
                        .debug_selector(move || format!("fleet-job-doing-{ix}"))
                        .child(text(need.clone(), p.roles.text_secondary))
                        .tooltip(primitives::tooltip_view(need, None, &p)),
                )
                .child(
                    // Its risk ("Deletes branches") and the ask.
                    primitives::page_cell(cols[5].1)
                        .debug_selector(move || format!("fleet-job-need-{ix}"))
                        .flex()
                        .items_center()
                        .gap(px(t.space.sm))
                        .children(risk.map(|r| super::decision::risk_chip(r, &p)))
                        .child(text(
                            Some(job.proposed_by.trim())
                                .filter(|b| !b.is_empty())
                                .map(|b| format!("Allow this job? {b} proposed it"))
                                .unwrap_or_else(|| "Allow this job?".into()),
                            p.text,
                        )),
                )
                .child(primitives::page_cell(cols[6].1).child(decision.render(&p)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.fleet.focus = Some(focus.clone());
                    cx.notify();
                }))
                .into_any_element()
        };
        // One builder per row kind (`Ok` a session's position, `Err` a
        // proposal's), each handed `cx`.
        let row = |which: Result<usize, usize>, cx: &mut Context<Self>| -> AnyElement {
            let n = match which {
                Ok(n) => n,
                Err(ix) => return job_row(ix, &jobs[ix], cx),
            };
            let tile = &tiles[n];
            let index = tile.index;
            let focused = n == focus;
            let choice = |id: String, key: usize, label: SharedString| {
                Choice::new(id, label)
                    .key(Some(key))
                    .enabled(enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.fleet_answer(n, key, window, cx);
                    }))
            };
            let actions: AnyElement = match &tile.ask {
                Some(Ask::Approval {
                    keys,
                    risk,
                    command,
                    folder,
                    ..
                }) => {
                    let key = |k: StripKey| keys.contains(&k).then(|| k.number()).flatten();
                    let allow = choice(
                        format!("fleet-row-allow-{n}"),
                        StripKey::Allow.number().unwrap_or(1),
                        "Allow".into(),
                    )
                    .tooltip("Allow this once");
                    let mut decision =
                        Decision::new(format!("fleet-row-decision-{n}"), allow, Frame::Row);
                    decision.deny = key(StripKey::Deny).map(|k| {
                        choice(format!("fleet-row-deny-{n}"), k, "Deny".into())
                            .tooltip("Deny this request")
                    });
                    decision.risk = *risk;
                    let card = self.view.sessions[index].card();
                    decision.armed =
                        self.armed(&super::decision::confirm_key(&tile.id, card.as_deref()));
                    let batch = batch_of(tiles, n).len();
                    if batch > 1 {
                        let id: SharedString = format!("fleet-row-batch-{n}").into();
                        decision.alternates.push(
                            Choice::new(id, format!("Allow all {batch}"))
                                .variant_key()
                                .enabled(enabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let tiles = this.fleet_tiles(cx);
                                    if let Some(tile) = tiles.get(n) {
                                        this.fleet.focus = Some(tile.id.clone());
                                    }
                                    this.fleet_variant(cx);
                                }))
                                .tooltip(format!(
                                    "Allow {} in {folder} for all {batch} sessions that ask for it",
                                    command.trim()
                                )),
                        );
                    }
                    // R7.1: Deny with note on the focused row (it opens
                    // the session with the note).
                    decision.deny_note = key(StripKey::DenyWithNote).filter(|_| focused).map(|k| {
                        choice(format!("fleet-row-note-{n}"), k, "Deny with note".into()).tooltip(
                            "Deny, and tell the agent what to do instead (opens the session)",
                        )
                    });
                    decision.keys = focused;
                    decision.filled = focused;
                    decision.render(&p).into_any_element()
                }
                Some(Ask::Stalled(stall)) => {
                    let mut decision = self.stall_decision(
                        stall,
                        &format!("fleet-row-stall-{n}"),
                        Frame::Row,
                        self.fleet_stall_key(stall.clone()),
                        cx,
                    );
                    decision.keys = focused;
                    decision.filled = focused;
                    decision.render(&p).into_any_element()
                }
                _ => {
                    let sel: SharedString = format!("fleet-row-open-{n}").into();
                    div()
                        .w_full()
                        .flex()
                        .justify_end()
                        .child(
                            primitives::slot_button(
                                primitives::button(
                                    sel.clone(),
                                    "Open",
                                    None,
                                    ButtonVariant::Tertiary,
                                    ButtonSize::Small,
                                    enabled,
                                    &p,
                                ),
                                &p,
                            )
                            .debug_selector(move || sel.to_string())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_listed_session(index, cx)
                                }))
                            }),
                        )
                        .into_any_element()
                }
            };
            let doing = tile
                .lines
                .iter()
                // R6.2: a stall is what it needs, not what it is doing.
                .find(|l| l.fault != Some(StatusKind::NotResponding) || tile.stall.is_none())
                .map(|l| {
                    [Some(l.verb.clone()), l.argument.clone(), l.summary.clone()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let need = match (&tile.ask, &tile.stall) {
                (Some(ask), _) => ask.text(),
                (None, Some(stall)) => stall.reason(now),
                _ => String::new(),
            };
            // R7.1: a request's risk shows in the table too.
            let need_risk = match &tile.ask {
                Some(Ask::Approval { risk, .. }) => *risk,
                _ => None,
            };
            // r6-integration (R6.2/R6.3): a row answers one ask. When a
            // stalled subagent waits behind the approval, the Needs cell
            // says "+1 more" and names it, so the second ask is not lost;
            // the row (or the tile view) acts on it once the first is done.
            let behind = match (&tile.ask, &tile.stall) {
                (Some(Ask::Stalled(_)) | None, _) | (_, None) => None,
                (Some(_), Some(stall)) => Some(stall.reason(now)),
            };
            let wait = (tile.bucket == Bucket::NeedsYou)
                .then_some(tile.since)
                .flatten()
                .map(|at| wks_native::format::age(at, now))
                .unwrap_or_default();
            let kind = tile.kind;
            primitives::page_table_row(&p)
                .id(SharedString::from(format!("fleet-row-{}", tile.id)))
                .debug_selector(move || format!("fleet-row-{n}"))
                .cursor_pointer()
                .when(focused, |d| d.bg(rgb(p.selected)))
                .when(!focused, |d| d.hover(|s| s.bg(rgb(p.roles.hover))))
                .child(
                    primitives::page_cell(cols[0].1)
                        .debug_selector(move || format!("fleet-status-{n}-{kind:?}"))
                        .child(system::StatusCell::new(kind).render(&p))
                        // The focus marker, inside a cell so it takes no gap.
                        .when(focused, |d| {
                            d.child(div().debug_selector(move || format!("fleet-focused-{n}")))
                        }),
                )
                .child(
                    primitives::page_cell(cols[1].1)
                        .font_weight(FontWeight(t.weight.medium))
                        .child(text(tile.title.clone(), p.text)),
                )
                .child(
                    primitives::page_cell(cols[2].1)
                        .child(text(tile.project.clone(), p.roles.text_secondary)),
                )
                .child(primitives::tabular(
                    primitives::page_cell(cols[3].1)
                        .debug_selector(move || format!("fleet-row-waiting-{n}"))
                        .child(text(wait, p.roles.text_secondary)),
                ))
                .child(primitives::page_cell(cols[4].1).child(text(doing, p.roles.text_secondary)))
                .child(
                    primitives::page_cell(cols[5].1)
                        .flex()
                        .items_center()
                        .gap(px(t.space.xs))
                        .children(need_risk.map(|r| super::decision::risk_chip(r, &p)))
                        .child(text(need, p.text))
                        .when_some(behind, |d, behind| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!("fleet-row-more-{n}")))
                                    .debug_selector(move || format!("fleet-row-more-{n}"))
                                    .flex_shrink_0()
                                    .text_size(px(t.text.meta))
                                    .font_weight(FontWeight(t.weight.medium))
                                    .text_color(rgb(p.roles.status.attention))
                                    .tooltip(primitives::tooltip_view(
                                        format!("Also needs you: {behind}. Open the session to Nudge or Stop it."),
                                        None,
                                        &p,
                                    ))
                                    .child("+1 more"),
                            )
                        }),
                )
                .child(primitives::page_cell(cols[6].1).child(actions))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let tiles = this.fleet_tiles(cx);
                    if let Some(tile) = tiles.iter().find(|t| t.index == index) {
                        this.fleet.focus = Some(tile.id.clone());
                    }
                    this.open_listed_session(index, cx);
                }))
                .into_any_element()
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        for which in (0..waiting)
            .map(Ok)
            .chain((0..jobs.len()).map(Err))
            .chain((waiting..tiles.len()).map(Ok))
        {
            rows.push(row(which, cx));
        }
        primitives::page_table(&p)
            .debug_selector(|| "fleet-table".into())
            .child(primitives::page_table_head(&cols, &p))
            .children(rows)
    }
}

/// The `fleet.focus` prefix of a job proposal's card (a tile's focus is
/// its session id).
const JOB_FOCUS: &str = "job:";

/// The card that has the Fleet's focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cursor {
    Tile(usize),
    Job(usize),
    None,
}

/// R7.1: the keycaps the focused tile `at` shows, in the order of the
/// one key map (a batch's Shift+1 last).
fn focused_caps(tiles: &[Tile], at: usize) -> Vec<String> {
    let Some(tile) = tiles.get(at) else {
        return Vec::new();
    };
    let mut caps: Vec<String> = match &tile.ask {
        Some(Ask::Approval { keys, .. }) => keys
            .iter()
            .filter_map(StripKey::number)
            .map(|n| n.to_string())
            .collect(),
        Some(Ask::Question { options, .. }) if !options.is_empty() => {
            (1..=options.len().min(5)).map(|n| n.to_string()).collect()
        }
        Some(Ask::Question { .. } | Ask::Open { .. }) => vec!["1".to_owned()],
        Some(Ask::Stalled(_)) => vec!["1".to_owned(), "2".to_owned()],
        None => Vec::new(),
    };
    if batch_of(tiles, at).len() > 1 {
        caps.push(super::decision::VARIANT_KEY.to_owned());
    }
    caps
}

/// R7.1: what a job proposal would run, for the table's Needs cell ("Run
/// git branch --merged … | xargs git branch -d"), or its sentence.
fn job_need(job: &wks_native::jobs::Job) -> String {
    match job.describe() {
        (_, Some(command)) => format!("Run {}", command.trim()),
        (sentence, None) => sentence,
    }
}

/// Every focusable Fleet card's focus id in on-screen order: the tiles
/// that wait on you, the job proposals, then the rest (the grid's and the
/// table's order), so the arrows walk what the eye reads.
fn fleet_slots(tiles: &[Tile], jobs: &[wks_native::jobs::Job]) -> Vec<String> {
    let waiting = tiles
        .iter()
        .take_while(|t| t.bucket == Bucket::NeedsYou)
        .count();
    tiles[..waiting]
        .iter()
        .map(|t| t.id.clone())
        .chain(jobs.iter().map(|j| format!("{JOB_FOCUS}{}", j.id)))
        .chain(tiles[waiting..].iter().map(|t| t.id.clone()))
        .collect()
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
            summary([2, 1, 3, 5, 20]),
            "2 need you · 1 to review · 3 working · 5 idle · 20 ended"
        );
        assert_eq!(summary([1, 0, 0, 0, 0]), "1 needs you");
        assert_eq!(summary([0, 0, 0, 0, 0]), "No sessions yet");
        assert!(shown(Bucket::Review, Filter::NeedsYou));
        assert!(shown(Bucket::Review, Filter::Active));
    }

    #[test]
    fn tools_read_as_verbs() {
        assert_eq!(tool_line("Read").1, "Read");
        assert_eq!(tool_line("Bash").1, "Run");
        assert_eq!(tool_line("mcp__fs__read_file").1, "Read");
        assert_eq!(tool_line("Custom").1, "Custom");
    }
}
