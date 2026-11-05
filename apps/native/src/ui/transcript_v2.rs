//! Design v2's transcript (area-transcript): the reading column, CLI-dense
//! tool rows, one card anatomy for subagents and fleet wakes, hugging user
//! bubbles, quiet metadata and "Jump to latest" with its new-message count.
//!
//! Everything here runs only under `Design::V2`; the v1 renderers in
//! `transcript.rs`, `work.rs`, `tools.rs`, `children.rs` and `fleet_card.rs`
//! branch into it once and otherwise stay as they were. Every style reads
//! tokens (`docs/design-v2.md`).
use super::primitives::{self, ButtonSize, ButtonVariant, FadeEdge, StatusBadge, StatusKind};
use super::*;
use gpui::{AnyElement, Pixels, relative};
use gpui_component::{Icon, IconName};
use std::ops::Range;
use wks_native::child_agents::ChildAgent;
use wks_native::format;
use wks_native::model::Row;
use wks_native::tool_preview;
use wks_native::transcript::{Fleet, FleetKind};

/// What a finished call produced, in a few words: `"42 lines"` for a read,
/// `"3 matches"` for a search. `None` when the row already says enough.
pub(super) fn tool_result_meta(category: &str, name: &str, output: &str) -> Option<String> {
    let lines = output.lines().filter(|l| !l.trim().is_empty()).count();
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match category {
        "Read" if lines > 0 => Some(plural(lines, "line", "lines")),
        "Search" if lines == 0 => Some("No matches".into()),
        "Search" if name.eq_ignore_ascii_case("glob") => Some(plural(lines, "file", "files")),
        "Search" => Some(plural(lines, "match", "matches")),
        "Command" if lines > 0 => Some(plural(lines, "line of output", "lines of output")),
        _ => None,
    }
}

/// r3-transcript: tool output this short shows inline under its row.
pub(super) const INLINE_LINES: usize = 5;

/// A line that only reports the exit status ("Exit code 101").
fn exit_line(line: &str) -> Option<i64> {
    let lower = line.trim().to_ascii_lowercase();
    let rest = [
        "exit code ",
        "exit status ",
        "exited with code ",
        "process exited with code ",
    ]
    .iter()
    .find_map(|p| lower.strip_prefix(p))?;
    rest.trim_end_matches(['.', ')']).trim().parse().ok()
}

/// A failed command's exit code, from the output's own report.
pub(super) fn exit_code(output: &str) -> Option<i64> {
    output.lines().find_map(exit_line)
}

/// What a finished call shows under its row (r3-transcript): output of at
/// most [`INLINE_LINES`] lines, the exit-status line left to the row. A
/// failure shows its last lines with how many came before. Reads and edits
/// never inline (their output is the file, or "updated"). `None`: nothing.
pub(super) fn inline_output(
    category: &str,
    output: &str,
    failed: bool,
) -> Option<(Vec<String>, usize)> {
    if matches!(
        category,
        "Read" | "Edit" | "Skill" | "Subagent" | "Workflow"
    ) {
        return None;
    }
    let kept: Vec<String> = output
        .lines()
        .filter(|l| !l.trim().is_empty() && exit_line(l).is_none())
        .map(|l| l.trim_end().replace('\t', "    "))
        .collect();
    // Shared indentation goes (cargo indents its status lines).
    let indent = kept
        .iter()
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    let lines: Vec<String> = kept
        .into_iter()
        .map(|l| {
            let l = l[indent..].to_owned();
            format::truncate_end(&l, 240).unwrap_or(l)
        })
        .collect();
    match lines.len() {
        0 => None,
        n if n <= INLINE_LINES => Some((lines, 0)),
        n if failed => Some((lines[n - INLINE_LINES..].to_vec(), n - INLINE_LINES)),
        _ => None,
    }
}

/// A work block's one line (r3-transcript): "Worked 5m 40s · 6 tools"
/// (`format::worked_summary`), "Working · 3 tools" while it runs.
pub(super) fn work_line(running: bool, duration_ms: Option<i64>, tools: usize) -> String {
    match duration_ms {
        _ if running => format!("Working · {}", plural(tools, "tool", "tools")),
        Some(ms) if ms >= 1000 => format::worked_summary(ms, tools),
        _ => format!("Worked · {}", plural(tools, "tool", "tools")),
    }
}

/// Whether a run of calls ended well although some failed: its last
/// finished call succeeded (a failed `cargo test`, fixed, then passing).
/// The run is then coloured by that final outcome, never red (R5.4).
pub(super) fn recovered<'a>(rows: impl IntoIterator<Item = &'a Row>) -> bool {
    let calls: Vec<_> = rows.into_iter().filter_map(|r| r.tool.as_ref()).collect();
    // Every failed call was run again later (same tool, same target) and
    // that run passed: `cargo test` failed, then passed. Reading a file
    // after a failed build fixes nothing.
    let mut any = false;
    for (at, failed) in calls.iter().enumerate().filter(|(_, t)| t.is_error) {
        any = true;
        let target = failed.target();
        let fixed = calls[at + 1..].iter().any(|later| {
            later.complete
                && !later.is_error
                && later.name == failed.name
                && later.target() == target
        });
        if !fixed {
            return false;
        }
    }
    any
}

/// A response card action in plain words (r2-transcript: no "Prefill").
pub(super) fn card_action_label(action: &wks_native::transcript::CardAction) -> String {
    use wks_native::transcript::CardAction;
    match action {
        CardAction::OpenWorker { label, .. } => format!("Open {label}"),
        CardAction::ViewDiff { label, .. } => format!("View diff · {label}"),
        CardAction::FillComposer { label, .. } => format!("Reply “{label}”"),
    }
}

/// A subagent's status in the shared vocabulary (r3-transcript: only the
/// lexicon's words), with the word to show.
pub(super) fn child_status(status: &str, connected: bool) -> (StatusKind, &'static str) {
    if !connected {
        return (StatusKind::Offline, StatusKind::Offline.label());
    }
    let kind = match status {
        "running" | "working" | "thinking" | "responding" | "streaming" | "background"
        | "starting" | "initializing" => StatusKind::Working,
        "complete" | "completed" | "done" => StatusKind::Done,
        "failed" | "error" | "lost" => StatusKind::Failed,
        "approval" | "waiting_approval" | "question" | "waiting_input" => StatusKind::NeedsYou,
        _ => StatusKind::Idle,
    };
    (kind, kind.label())
}

/// A fleet entry's v1 status word (`fleet_card::entry_status`) as a v2 kind.
pub(super) fn fleet_entry_kind(label: &str) -> StatusKind {
    match label {
        "Needs approval" | "Has a question" | "Needs a decision" | "Escalated" | "Blocked"
        | "Threshold crossed" => StatusKind::NeedsYou,
        "Failed" => StatusKind::Failed,
        "Still running" => StatusKind::Working,
        "Resolved" | "Answered" | "Finished" => StatusKind::Done,
        _ => StatusKind::Idle,
    }
}

/// The card's eyebrow, in sentence case, saying where things stand now:
/// a supervisor wake reads "Decision needed" only while one still is, and
/// "Decision made" after (never "needed" beside "Resolved").
pub(super) fn fleet_eyebrow(kind: FleetKind, pending: bool, entries: usize) -> String {
    let agents = |verb: &str| {
        if entries == 1 {
            format!("Agent {verb}")
        } else {
            format!("{entries} agents {verb}")
        }
    };
    match kind {
        FleetKind::Blocked if pending => "Decision needed".into(),
        FleetKind::Blocked => "Decision made".into(),
        FleetKind::Finished => agents("finished"),
        FleetKind::Failed => agents("failed"),
        FleetKind::Escalated if pending => agents("escalated"),
        FleetKind::Escalated => "Escalation handled".into(),
        FleetKind::CatchUp => "Catch-up".into(),
        FleetKind::Threshold if pending => "Threshold crossed".into(),
        FleetKind::Threshold => "Threshold handled".into(),
        FleetKind::Progress => "Progress update".into(),
    }
}

/// A fleet entry's state (`fleet_card::entry_status`) as the lower-case
/// word the one-line wake uses ("Release checklist answered").
pub(super) fn fleet_word(label: &str) -> String {
    match label {
        "Needs approval" | "Has a question" | "Needs a decision" => "needs you".into(),
        "Finished" => "done".into(),
        "Still running" => "is still running".into(),
        "Threshold crossed" => "crossed a threshold".into(),
        other => other.to_lowercase(),
    }
}

/// Rows that arrived since the reader stopped following, for "n new".
pub(super) fn unseen(seen: Option<&(String, usize)>, session: &str, rows: usize) -> usize {
    match seen {
        Some((owner, at)) if owner == session => rows.saturating_sub(*at),
        _ => 0,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ----- r2-transcript: the event row ----------------------------------------
//
// One line per agent or tool event: a bare 14px glyph, the verb, the primary
// argument (mono), the result in a few words, then status or duration on the
// right. Tool calls, skills, delegations, subagents, turn changes and settled
// fleet wakes all use it, so the transcript reads as one log. `systems` builds
// a shared event-row primitive in parallel; this one is token-only and meant
// to fold into it.

/// What an event row leads with: a bare glyph, never a tile.
#[derive(Clone)]
pub(super) enum Glyph {
    Named(IconName),
    Asset(&'static str),
    /// A status indicator in the glyph's place (subagent rows).
    Status(StatusKind),
}

/// A call's glyph by what it does.
pub(super) fn tool_glyph(name: &str, category: &str) -> Glyph {
    let short = short_tool_name(name);
    match category {
        "Read" => Glyph::Asset("lucide/file-text.svg"),
        "Edit" => Glyph::Asset("lucide/pencil.svg"),
        "Command" => Glyph::Named(IconName::SquareTerminal),
        "Search" => Glyph::Named(IconName::Search),
        "Skill" => Glyph::Asset("lucide/sparkles.svg"),
        "Subagent" => Glyph::Named(IconName::Bot),
        "Workflow" => Glyph::Named(IconName::GalleryVerticalEnd),
        _ if short.starts_with("web") || short.contains("fetch") => Glyph::Named(IconName::Globe),
        _ if short.contains("todo") || short.contains("plan") => {
            Glyph::Named(IconName::GalleryVerticalEnd)
        }
        _ => Glyph::Asset("lucide/wrench.svg"),
    }
}

/// `mcp__server__read_file` / `functions.exec` → `read_file` / `exec`,
/// lower-cased.
fn short_tool_name(name: &str) -> String {
    let short = name.rsplit("__").next().unwrap_or(name);
    short
        .rsplit('.')
        .next()
        .unwrap_or(short)
        .to_ascii_lowercase()
}

/// The event's verb, past tense once it has finished ("Read", "Edited",
/// "Ran"), progressive while it runs. `None` for tools with no stock verb
/// (the row then uses the tool's own title).
pub(super) fn tool_verb(name: &str, category: &str, running: bool) -> Option<&'static str> {
    let short = short_tool_name(name);
    let pick = |done: &'static str, live: &'static str| Some(if running { live } else { done });
    match short.as_str() {
        "write" | "write_file" | "create_file" => pick("Wrote", "Writing"),
        "glob" | "list" | "list_directory" | "ls" => pick("Listed", "Listing"),
        "webfetch" | "web_fetch" | "fetch" => pick("Fetched", "Fetching"),
        "websearch" | "web_search" => pick("Searched web", "Searching web"),
        "todowrite" | "update_plan" => pick("Updated plan", "Updating plan"),
        _ => match category {
            "Read" => pick("Read", "Reading"),
            "Edit" => pick("Edited", "Editing"),
            "Command" => pick("Ran", "Running"),
            "Search" => pick("Searched", "Searching"),
            "Skill" => pick("Used skill", "Using skill"),
            "Subagent" => pick("Delegated", "Delegating"),
            "Workflow" => pick("Ran workflow", "Running workflow"),
            _ => None,
        },
    }
}

/// What ends an event row.
#[derive(Clone, Debug)]
pub(super) enum EventEnd {
    Nothing,
    /// A quiet duration or age, tertiary, inline after the result
    /// (r3-transcript: "· 2 matches · 6s", never on a far edge).
    Quiet(String),
    /// A status that needs reading (Working, Not responding, Failed) on the
    /// column's right edge.
    Badge(StatusBadge),
}

/// One event row's content; [`event_row`] draws it.
pub(super) struct EventRow {
    pub id: SharedString,
    pub glyph: Glyph,
    pub verb: SharedString,
    /// The primary argument, drawn in the mono face when `mono` is set.
    pub target: Option<SharedString>,
    pub mono: Option<SharedString>,
    /// r3-transcript: what a count covers ("This turn"), before the diff.
    pub scope: Option<SharedString>,
    pub diff: (usize, usize),
    /// The result in a few words ("42 lines", "3 matches").
    pub summary: Option<SharedString>,
    /// r3-transcript: a failure's reason, inline in red ("exit 101").
    pub fault: Option<SharedString>,
    pub end: EventEnd,
    /// `Some(open)` draws an inline disclosure chevron (on hover while closed).
    pub disclosure: Option<bool>,
    /// The chevron always shows (a work block's one line), not on hover.
    pub reveal: bool,
    /// The call failed: glyph and verb turn the fault red.
    pub failed: bool,
    /// Regular-weight verb (child rows, whose title is not a verb).
    pub quiet: bool,
    /// Controls after the status (View output / Nudge / Stop, Review).
    pub trailing: Vec<AnyElement>,
}

impl EventRow {
    pub(super) fn new(
        id: impl Into<SharedString>,
        glyph: Glyph,
        verb: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            glyph,
            verb: verb.into(),
            target: None,
            mono: None,
            scope: None,
            diff: (0, 0),
            summary: None,
            fault: None,
            end: EventEnd::Nothing,
            disclosure: None,
            reveal: false,
            failed: false,
            quiet: false,
            trailing: Vec::new(),
        }
    }
}

/// A glyph in the event row's fixed icon column.
pub(super) fn glyph_box(glyph: Glyph, color: u32, p: &Palette) -> Div {
    let t = p.tokens;
    let tt = t.transcript();
    let size = px(t.icon.sm);
    div()
        .size(px(tt.event_icon))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(color))
        .child(match glyph {
            Glyph::Named(name) => Icon::new(name).size(size).into_any_element(),
            Glyph::Asset(path) => gpui::svg()
                .path(path)
                .size(size)
                .text_color(rgb(color))
                .into_any_element(),
            Glyph::Status(kind) => primitives::status_indicator(kind, p).into_any_element(),
        })
}

/// Draws one [`EventRow`]: CLI-dense, one line. r3-transcript: its hover
/// fill hangs into the gutter on both sides, so the glyph lines up with the
/// prose's left edge and a status or action with the column's right edge;
/// durations and the disclosure chevron sit inline with the result.
pub(super) fn event_row(row: EventRow, p: &Palette) -> Stateful<Div> {
    let p = *p;
    let t = p.tokens;
    let tt = t.transcript();
    // r4-calm (R4.4): one red signal per failure. A red status already on
    // the row (its glyph or its badge) carries it; otherwise the glyph
    // does. The verb stays neutral and a fault's reason is named in
    // secondary text unless nothing else on the row is red.
    let red = |kind: StatusKind| matches!(kind, StatusKind::Failed | StatusKind::NotResponding);
    let signalled = matches!(&row.end, EventEnd::Badge(badge) if red(badge.kind))
        || matches!(row.glyph, Glyph::Status(kind) if red(kind));
    let glyph_red = row.failed && !signalled;
    let fault_color = if signalled || glyph_red {
        p.roles.text_secondary
    } else {
        p.error
    };
    let glyph_color = if glyph_red {
        p.error
    } else {
        p.roles.text_tertiary
    };
    let mono = row.mono.clone();
    let meta = |text: SharedString, color: u32| {
        div()
            .flex_shrink_0()
            .text_size(px(t.text.meta))
            .text_color(rgb(color))
            .child(text)
    };
    div()
        .id(row.id)
        .group("event-row")
        .h(px(tt.tool_row))
        .mx(px(-t.space.sm))
        .px(px(t.space.sm))
        .rounded(px(t.radius.control))
        .flex()
        .items_center()
        .gap(px(t.space.md))
        .min_w_0()
        .cursor_pointer()
        .hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
        .active(move |s| s.bg(primitives::tonal(&p, t.wash.pressed)))
        .child(glyph_box(row.glyph, glyph_color, &p))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(t.text.body))
                .font_weight(FontWeight(if row.quiet {
                    t.weight.regular
                } else {
                    t.weight.medium
                }))
                // R5.4: activity is the middle level, under the prose.
                .text_color(rgb(p.roles.text_secondary))
                .child(row.verb),
        )
        .when_some(row.target, |d, target| {
            d.child(
                div()
                    .min_w_0()
                    .flex_shrink()
                    .truncate()
                    .when_some(mono, |d, mono| d.font_family(mono))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(target),
            )
        })
        .when_some(row.scope, |d, scope| {
            d.child(meta(scope, p.roles.text_secondary))
        })
        .when(row.diff.0 > 0, |d| {
            d.child(meta(format!("+{}", row.diff.0).into(), p.success))
        })
        .when(row.diff.1 > 0, |d| {
            d.child(meta(format!("−{}", row.diff.1).into(), p.error))
        })
        .when_some(row.summary, |d, summary| {
            d.child(meta(format!("· {summary}").into(), p.roles.text_tertiary))
        })
        .map(|d| match &row.end {
            EventEnd::Quiet(label) => d.child(
                primitives::tabular(meta(format!("· {label}").into(), p.roles.text_tertiary))
                    .debug_selector(|| "event-duration".into()),
            ),
            _ => d,
        })
        .when_some(row.fault, |d, fault| {
            d.child(
                meta(format!("· {fault}").into(), fault_color)
                    .debug_selector(|| "event-fault".into()),
            )
        })
        .when_some(row.disclosure, |d, open| {
            d.child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(p.roles.text_tertiary))
                    .when(!open && !row.reveal, |d| {
                        d.opacity(0.).group_hover("event-row", |s| s.opacity(1.))
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(t.icon.xs)),
                    ),
            )
        })
        .child(div().flex_1())
        .map(|d| match row.end {
            EventEnd::Badge(badge) => d.child(badge.render(&p)),
            _ => d,
        })
        .children(row.trailing)
}

/// The hairline guide that holds a group's rows under its header, from the
/// header glyph's centre down.
pub(super) fn event_guide(p: &Palette) -> Div {
    let t = p.tokens;
    let tt = t.transcript();
    system::edge(
        div()
            .flex()
            .flex_col()
            .ml(px(tt.event_icon / 2.))
            .pl(px(tt.event_icon / 2. + t.space.md - t.stroke.hairline)),
        system::Side::Left,
        system::EdgeTone::Divider,
        p,
    )
}

impl Workspace {
    // ----- The reading column -------------------------------------------

    /// The transcript's reading measure in px, gutters included: the
    /// reading-width setting, never so wide that the floating tool rail
    /// would cover it.
    ///
    /// r2-transcript: the column is fluid. It fills the pane (less room for
    /// the floating rail) between `reading_min` and the setting's cap, so it
    /// grows with the window and gives way gracefully when Changes docks.
    pub(super) fn reading_max(&self) -> Pixels {
        // r2-shell: the tool rail docks to it
        let t = self.tokens();
        let tt = t.transcript();
        let gutter = t.layout.chat_gutter;
        let cap = px(self.settings.reading_width.measure(&t) + 2. * gutter);
        if self.chat_width <= Pixels::ZERO {
            return cap;
        }
        let rail = if self.tools_float_v2(false) {
            2. * (t.layout.rail_width + t.space.md)
        } else {
            0.
        };
        let room = self.chat_width - px(rail);
        cap.min(room.max(px(tt.reading_min + 2. * gutter)))
    }

    /// The column's outer measure without its gutters (v1: `chat_frame`).
    pub(super) fn reading_frame(&self) -> Div {
        if !self.design().is_v2() {
            return chrome::chat_frame();
        }
        div().w_full().max_w(self.reading_max()).mx_auto()
    }

    /// The centred reading column with its gutters (v1: `chat_column`).
    pub(super) fn reading_column(&self) -> Div {
        if !self.design().is_v2() {
            return chrome::chat_column();
        }
        self.reading_frame()
            .px(px(self.tokens().layout.chat_gutter))
    }

    /// Whether the session's tools float in a rail beside the column (v2).
    /// r3-shell: never; they live on the header strip's toolbar, so the
    /// column keeps all of its room.
    pub(super) fn tools_float_v2(&self, _narrow: bool) -> bool {
        false
    }

    // ----- Quiet metadata -------------------------------------------------

    /// A message's time, without seconds, in the quietest tier.
    pub(super) fn timestamp_v2(&self, duration: Option<SharedString>, at: Option<i64>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        div()
            .debug_selector(|| "message-timestamp-footer".into())
            .flex()
            .justify_end()
            .items_center()
            .gap(px(t.space.md))
            .text_size(px(t.text.overline))
            .text_color(rgb(p.roles.text_tertiary))
            .children(duration)
            .when_some(at, |d, at| {
                d.child(format::timestamp(
                    Some(at),
                    timing::now_ms(),
                    self.settings.twelve_hour_clock,
                ))
            })
    }

    /// A hover-revealed copy button for one message.
    pub(super) fn copy_v2(&self, id: String, copy: String) -> Stateful<Div> {
        let p = self.palette();
        primitives::icon_button(
            SharedString::from(id),
            IconName::Copy,
            "Copy message",
            ButtonVariant::Tertiary,
            true,
            &p,
        )
        .opacity(0.)
        .group_hover("message", |s| s.opacity(1.))
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))
    }

    // ----- Tool calls -------------------------------------------------------

    /// One tool call as a CLI-dense line: `● Read src/main.rs · 3 lines`.
    /// The dot is the call's state (Working pulses, Failed is red, a finished
    /// call stays neutral); clicking opens its input and output beneath.
    pub(super) fn render_step_v2(
        &mut self,
        row: &Arc<Row>,
        recovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let tool = row.tool.as_ref().expect("tool row");
        let session = self.chat_owner();
        let identity = tools::identity(row);
        let element_key = format!("{session}-{identity}");
        // r3-transcript: a failure shows inline under its row (red, exit
        // code); its full input and output stay one click away.
        let expanded = self.tool_expansion.get(&identity).copied().unwrap_or(false);
        let preview = self.step_previews.get(&element_key, [row], || {
            tool_preview::parse(
                &tool.name,
                &tool.input,
                tool.complete.then_some(tool.output.as_str()),
            )
        });
        let cwd = self
            .selected_session()
            .map(|s| s.cwd.trim_end_matches('/').to_owned())
            .unwrap_or_default();
        let target = if preview.target == preview.title {
            String::new()
        } else if !cwd.is_empty()
            && let Some(relative) = preview.target.strip_prefix(&format!("{cwd}/"))
        {
            relative.to_owned()
        } else {
            preview.target.clone()
        };
        // r2-transcript: one event row: glyph, verb, argument, result, and
        // the status or duration on the right.
        let running = !tool.complete && !tool.is_error;
        let verb = tool_verb(&tool.name, tool.category(), running)
            .map(str::to_owned)
            .unwrap_or_else(|| preview.title.clone());
        let duration = match (row.timestamp_ms, tool.completed_at_ms) {
            (Some(start), Some(end)) if end - start >= 1000 => Some(format::duration(end - start)),
            _ => None,
        };
        let key = row.key;
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let mut event = EventRow::new(
            format!("{element_key}-step"),
            tool_glyph(&tool.name, tool.category()),
            verb,
        );
        event.target = (!target.is_empty()).then(|| target.into());
        event.mono = Some(mono);
        event.diff = (preview.added, preview.removed);
        let inline = (tool.complete || tool.is_error)
            .then(|| inline_output(tool.category(), &tool.output, tool.is_error))
            .flatten();
        // Shown output says how much there was; the count would repeat it.
        event.summary =
            (tool.complete && !tool.is_error && (inline.is_none() || tool.category() == "Search"))
                .then(|| tool_result_meta(tool.category(), &tool.name, &tool.output))
                .flatten()
                .map(Into::into);
        let code = tool
            .is_error
            .then(|| exit_code(&tool.output))
            .flatten()
            .map(|code| format!("exit code {code}"));
        // R5.4: a failure the run later recovered from keeps a muted mark
        // (tertiary "Failed · exit code 1"), never red.
        let muted = tool.is_error && recovered;
        if muted {
            event.summary = Some(
                code.map_or_else(|| "Failed".to_owned(), |c| format!("Failed · {c}"))
                    .into(),
            );
        } else {
            event.fault = code.map(Into::into);
        }
        event.failed = tool.is_error && !muted;
        event.disclosure = Some(expanded);
        event.end = if muted {
            duration.map_or(EventEnd::Nothing, EventEnd::Quiet)
        } else if tool.is_error {
            EventEnd::Badge(StatusBadge::new(StatusKind::Failed))
        } else if running {
            EventEnd::Badge(StatusBadge::new(StatusKind::Working))
        } else {
            duration.map_or(EventEnd::Nothing, EventEnd::Quiet)
        };
        let header = event_row(event, &p)
            .debug_selector(move || format!("tool-toggle-{key}"))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_tool(identity.clone(), expanded, cx)),
            );
        let mut step = div().flex().flex_col().child(header);
        if let Some((lines, before)) = inline.filter(|_| !expanded) {
            step = step.child(self.inline_output_v2(
                &element_key,
                lines,
                before,
                tool.is_error && !muted,
                cx,
            ));
        }
        if expanded {
            let mut details = self
                .tool_details_v2(&preview, tool, &element_key, window, cx)
                .when(row.truncated, |d| {
                    d.child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.warning))
                            .child("Tool content was clipped to the retained-history limit."),
                    )
                });
            let input = tool.value();
            if let Some(path) = input["file_path"]
                .as_str()
                .or_else(|| input["path"].as_str())
            {
                let cwd = self
                    .selected_session()
                    .map(|s| s.cwd.as_str())
                    .unwrap_or("");
                let link = super::file_viewer::tool_link(cwd, &input, path);
                details = details.child(self.file_button(
                    &format!("live:{session}:{key}-file"),
                    link,
                    path,
                    cx,
                ));
            }
            step = step.child(
                div()
                    // Under the verb, past the status column.
                    .pl(px(t.transcript().event_icon + t.space.md))
                    .pt(px(t.space.xxs))
                    .pb(px(t.space.sm))
                    .max_h(px(tt.details_max + t.space.xxl))
                    .child(details),
            );
        }
        step
    }

    /// Short output under its row (r3-transcript): dimmed mono, aligned with
    /// the verb; a failure's lines in red, with how many came before.
    fn inline_output_v2(
        &self,
        element_key: &str,
        lines: Vec<String>,
        before: usize,
        failed: bool,
        cx: &App,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let line_height = t.text.meta * t.text.line_height;
        let key = element_key.to_owned();
        div()
            .debug_selector(move || format!("tool-inline-output-{key}"))
            .pl(px(t.transcript().event_icon + t.space.md))
            .pb(px(t.space.xs))
            .flex()
            .flex_col()
            .text_size(px(t.text.meta))
            .line_height(px(line_height))
            .when(before > 0, |d| {
                d.child(div().text_color(rgb(p.roles.text_tertiary)).child(format!(
                    "… {}",
                    plural(before, "earlier line", "earlier lines")
                )))
            })
            // r4-calm: a failure's output is a faintly tinted block in
            // secondary text; the row's status is its one red signal.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .font_family(mono)
                    .when(failed, |d| {
                        d.mr(px(t.space.sm))
                            .px(px(t.space.sm))
                            .py(px(t.space.xxs))
                            .rounded(px(t.radius.xs))
                            .bg(primitives::wash(p.roles.status.fault, t.wash.rest))
                    })
                    .text_color(rgb(if failed {
                        p.roles.text_secondary
                    } else {
                        p.roles.text_tertiary
                    }))
                    .children(
                        lines
                            .into_iter()
                            .map(|line| div().min_w_0().truncate().whitespace_nowrap().child(line)),
                    ),
            )
    }

    /// A tool's input and output, opened under its line: on the card
    /// surface with its hairline edge, as code blocks are.
    fn tool_details_v2(
        &self,
        preview: &tool_preview::ToolPreview,
        tool: &wks_native::transcript::Tool,
        element_key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let cwd = preview.working_directory.to_string();
        let workspace = cx.entity().downgrade();
        let (on_hover, on_menu) =
            super::link_menu::link_events(workspace.clone(), Some(cwd.clone()));
        let on_link = super::file_viewer::output_link_handler(workspace, cwd);
        let label = |text: String| {
            div()
                .text_size(px(t.text.overline))
                .font_weight(FontWeight(t.weight.medium))
                .text_color(rgb(p.roles.text_tertiary))
                .child(text)
        };
        let mut body = div()
            .id(SharedString::from(format!("{element_key}-details")))
            .debug_selector(|| "tool-details".into())
            .max_h(px(tt.details_max))
            .overflow_y_scroll()
            .p(px(t.space.lg))
            .rounded(px(t.radius.control))
            .bg(rgb(p.surface))
            .map(|d| system::framed(d, &p))
            .flex()
            .flex_col()
            .gap(px(t.space.lg))
            .when(!preview.working_directory.is_empty(), |d| {
                d.child(label(preview.working_directory.clone()))
            })
            .when(!preview.description.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.prose))
                        .child(preview.description.clone()),
                )
            });
        for (index, block) in preview.blocks.iter().take(16).enumerate() {
            let text = block.text.lines().take(160).collect::<Vec<_>>().join("\n");
            let text: String = text.chars().take(12_000).collect();
            let clipped = text.len() < block.text.trim_end_matches('\n').len();
            body = body.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xs))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.md))
                            .child(label(block.label.clone()).flex_1().min_w_0().truncate())
                            .when(block.language != "text", |d| {
                                d.child(label(block.language.to_owned()))
                            })
                            .when(
                                block.label == "Output" && tool.completed_at_ms.is_some(),
                                |d| {
                                    d.child(label(format::timestamp(
                                        tool.completed_at_ms,
                                        timing::now_ms(),
                                        self.settings.twelve_hour_clock,
                                    )))
                                },
                            ),
                    )
                    .child(
                        div().debug_selector(|| "tool-block-text".into()).child(
                            TextView::markdown(
                                SharedString::from(format!("{element_key}-{index}")),
                                tool_preview::fenced(block.language, &text),
                                window,
                                cx,
                            )
                            .style(gpui_component::text::TextViewStyle {
                                on_link_click: Some(on_link.clone()),
                                linkify: Some(super::markdown::linkify()),
                                on_link_hover: Some(on_hover.clone()),
                                on_link_menu: Some(on_menu.clone()),
                                highlight_theme: gpui_component::Theme::global(cx)
                                    .highlight_theme
                                    .clone(),
                                is_dark: gpui_component::Theme::global(cx).mode.is_dark(),
                                ..Default::default()
                            })
                            .selectable(true),
                        ),
                    )
                    .when(clipped, |d| d.child(label("Preview shortened".into()))),
            );
        }
        if preview.blocks.len() > 16 {
            body = body.child(label(format!(
                "{} additional sections omitted from preview",
                preview.blocks.len() - 16
            )));
        }
        div().child(super::smooth_scroll::scroll_zone(body))
    }

    /// A run of adjacent calls: one summary line when there is more than one
    /// ("4 steps · Read 1 file, edited 1 file"), then the call lines on a
    /// thin guide, older ones folded behind "n earlier steps".
    pub(super) fn render_work_card_v2(
        &mut self,
        span: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let view = self.view.clone();
        let rows = &view.transcript.rows;
        let session = self.chat_owner();
        let card_key = format!("work:{session}:{}", tools::identity(&rows[span.start]));
        let summary = self
            .work_summaries
            .get(&card_key, rows.range(span.clone()), || {
                tool_preview::summarize_work(rows.range(span.clone()).map(AsRef::as_ref))
            });
        let steps = rows
            .range(span.clone())
            .filter(|r| r.tool.is_some())
            .count();
        // R5.4: coloured by the final outcome: a failure later recovered
        // from is not red at the run's level.
        let recovered = recovered(rows.range(span.clone()).map(AsRef::as_ref));
        let failed = if recovered { 0 } else { summary.failed };
        // r3-transcript: finished work folds to one line ("Worked 5m 40s ·
        // 6 tools ›") so the reply and its changes lead the turn; the block
        // still in progress (the transcript's last, while the session is
        // busy) stays open, as does one with a failure. Ctrl+O opens all.
        let in_progress = summary.running > 0
            || (span.end == rows.len() && self.selected_session().is_some_and(Session::busy));
        let expand_all = self.chat.expand_all;
        let open = expand_all
            || steps == 1
            || self
                .chat
                .open
                .get(&card_key)
                .copied()
                .unwrap_or(in_progress || failed > 0);
        let earlier_key = format!("{card_key}:earlier");
        let show_earlier = expand_all || self.chat.open.get(&earlier_key).copied().unwrap_or(false);
        let hidden = if show_earlier {
            0
        } else {
            span.len().saturating_sub(tt.visible_steps)
        };
        let hidden_steps = rows
            .range(span.start..span.start + hidden)
            .filter(|r| r.tool.is_some())
            .count();
        let mut card = div()
            .debug_selector(|| "tool-activity-group".into())
            .w_full()
            .flex()
            .flex_col();
        if steps > 1 {
            // r3-transcript: "Worked 5m 40s · 6 tools ›", nothing more: the
            // diff counts belong to the turn's Changed line, the breakdown to
            // the rows inside.
            let toggle_key = card_key.clone();
            let default_open = in_progress || failed > 0;
            let mut event = EventRow::new(
                format!("{card_key}-header"),
                Glyph::Status(if failed > 0 {
                    StatusKind::Failed
                } else if in_progress {
                    StatusKind::Working
                } else {
                    StatusKind::Done
                }),
                work_line(in_progress, summary.duration_ms, steps),
            );
            event.quiet = true;
            event.disclosure = Some(open);
            event.reveal = true;
            // The glyph carries the state; a failure is also named inline.
            event.fault = (failed > 0).then(|| format!("{failed} failed").into());
            card = card.child(
                event_row(event, &p)
                    .debug_selector(|| "work-card-toggle".into())
                    .tooltip(primitives::tooltip_view(
                        if open {
                            "Hide tools · every block:"
                        } else {
                            "Show tools · every block:"
                        },
                        Some("Ctrl O"),
                        &p,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.chat.expand_all {
                            // Leaving "show everything" from one block: it
                            // closes, the rest return to their defaults.
                            this.chat.expand_all = false;
                            this.chat.open.insert(toggle_key.clone(), true);
                        }
                        this.toggle_open(toggle_key.clone(), default_open, cx)
                    })),
            );
        }
        if open {
            let mut list = if steps > 1 {
                // A hairline guide from the header's glyph down its calls.
                event_guide(&p)
            } else {
                div().flex().flex_col()
            };
            if hidden > 0 {
                list = list.child(
                    div()
                        .id(SharedString::from(format!("{card_key}-earlier")))
                        .debug_selector(|| "work-earlier-steps".into())
                        .h(px(tt.tool_row))
                        .mx(px(-t.space.sm))
                        .px(px(t.space.sm))
                        .rounded(px(t.radius.control))
                        .flex()
                        .items_center()
                        .gap(px(t.space.md))
                        .cursor_pointer()
                        .hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(glyph_box(
                            Glyph::Named(IconName::Ellipsis),
                            p.roles.text_tertiary,
                            &p,
                        ))
                        .child(match hidden_steps {
                            0 => "Earlier notes".to_owned(),
                            n => plural(n, "earlier tool", "earlier tools"),
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_open(earlier_key.clone(), false, cx)
                        })),
                );
            }
            for row in rows.range(span.start + hidden..span.end) {
                list = list.child(if row.tool.is_some() {
                    self.render_step_v2(row, recovered, window, cx)
                } else {
                    self.render_note_v2(row, &session, window, cx)
                });
            }
            card = card.child(list);
        }
        card
    }

    /// An assistant note between calls in a merged turn: prose at the
    /// reading size, with room above it.
    fn render_note_v2(
        &mut self,
        row: &Row,
        session: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.tokens();
        let key = row.key;
        div()
            .debug_selector(move || format!("work-note-{key}"))
            .py(px(t.space.sm))
            .child(self.render_markdown(
                &format!("work-note:{session}:{key}"),
                &row.text,
                window,
                cx,
            ))
    }

    /// Skill, Subagent and Workflow calls (r2-transcript): one event row like
    /// any other call ("Delegated Session creation review"), its brief and
    /// its agents on the hairline guide beneath. No card, no icon tiles.
    pub(super) fn render_orchestration_v2(
        &mut self,
        row: &Row,
        extras: Vec<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tool = row.tool.as_ref().expect("tool row");
        let session = self.chat_owner();
        let identity = tools::identity(row);
        let element_key = format!("{session}-{identity}");
        let expanded = self
            .tool_expansion
            .get(&identity)
            .copied()
            .unwrap_or(tool.is_error);
        let mut preview = tool_preview::parse(
            &tool.name,
            &tool.input,
            tool.complete.then_some(tool.output.as_str()),
        );
        preview.target = tool.target();
        let key = row.key;
        let duration = match (row.timestamp_ms, tool.completed_at_ms) {
            (Some(start), Some(end)) if end - start >= 1000 => Some(format::duration(end - start)),
            _ => None,
        };
        let toggle = {
            let identity = identity.clone();
            cx.listener(move |this, _, _, cx| this.toggle_tool(identity.clone(), expanded, cx))
        };
        let category = tool.category();
        let running = !tool.complete && !tool.is_error;
        let origin = wks_native::child_agents::tool_kind(tool);
        let native = origin.is_some_and(|o| o != wks_native::child_agents::ChildKind::Workspacer);
        let children = self
            .child_ui
            .agents
            .by_tool
            .get(&tool.id)
            .cloned()
            .unwrap_or_default();
        let now = timing::now_ms();
        let connected = self.view.connected;
        let views: Vec<ChildView> = children
            .iter()
            .map(|c| child_view(c, connected, now))
            .collect();
        // R5.4: a delegation rolls up its worst child: never "Working"
        // beside a child that is not responding.
        let stalled = views.iter().any(|v| v.stalled);
        let verb = tool_verb(&tool.name, category, running)
            .unwrap_or("Used")
            .to_owned();
        let title = if category == "Skill" || preview.description.is_empty() {
            preview.target.clone()
        } else {
            preview.description.clone()
        };
        let role = tool.value()["subagent_type"]
            .as_str()
            .filter(|r| !r.is_empty() && !r.eq_ignore_ascii_case("general-purpose"))
            .map(str::to_owned);
        let mut event = EventRow::new(
            format!("{element_key}-toggle"),
            tool_glyph(&tool.name, category),
            verb,
        );
        event.target = (!title.is_empty()).then(|| title.clone().into());
        event.summary = role.map(Into::into);
        event.failed = tool.is_error;
        event.disclosure = Some(expanded);
        event.end = if tool.is_error {
            EventEnd::Badge(StatusBadge::new(StatusKind::Failed))
        } else if running && !stalled {
            EventEnd::Badge(StatusBadge::new(StatusKind::Working))
        } else if let Some(badge) = (views.len() > 1 || stalled)
            .then(|| children_badge(&views))
            .flatten()
        {
            EventEnd::Badge(badge)
        } else {
            duration.map_or(EventEnd::Nothing, EventEnd::Quiet)
        };
        let selector = if native {
            "native-spawn-icon"
        } else {
            "workspacer-spawn-icon"
        };
        let line = event_row(event, &p)
            .debug_selector(move || format!("tool-toggle-{key}"))
            .tooltip(primitives::tooltip_view(
                match (category, native) {
                    ("Skill", _) => "Skill",
                    ("Workflow", _) => "Workflow",
                    (_, true) => "Subagent run by the agent's own tools",
                    _ => "Agent session started by Workspacer",
                },
                None,
                &p,
            ))
            .child(div().debug_selector(move || selector.into()))
            .on_click(toggle);
        let below = !extras.is_empty() || row.truncated || expanded;
        div()
            .debug_selector(|| "orchestration-card".into())
            .w_full()
            .flex()
            .flex_col()
            .child(line)
            .when(below, |d| {
                d.child(
                    event_guide(&p)
                        .gap(px(t.space.xxs))
                        .pb(px(t.space.xs))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary))
                        .children(extras)
                        .when(row.truncated, |d| {
                            d.child(
                                div()
                                    .text_size(px(t.text.meta))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child(
                                        "Tool content was clipped to the retained-history limit.",
                                    ),
                            )
                        })
                        .when(expanded, |d| {
                            d.child(div().pt(px(t.space.xs)).child(self.tool_details_v2(
                                &preview,
                                tool,
                                &element_key,
                                window,
                                cx,
                            )))
                        }),
                )
            })
    }
}
impl Workspace {
    /// A delegation's brief (r2-transcript): one quiet line on the guide,
    /// the whole task (selectable) once opened.
    pub(super) fn dispatch_brief_v2(
        &self,
        key: &str,
        brief: &str,
        open_key: String,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let line = brief.split_whitespace().collect::<Vec<_>>().join(" ");
        let long = brief.trim().lines().count() > 1 || line.chars().count() > 120;
        div()
            .debug_selector(|| "task-brief".into())
            .min_h(px(tt.tool_row))
            .flex()
            .items_start()
            .gap(px(t.space.md))
            .text_size(px(t.text.body))
            .text_color(rgb(p.roles.text_secondary))
            .child(div().w(px(tt.event_icon)).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px((tt.tool_row - t.text.body * t.text.line_height) / 2.))
                    .line_height(relative(t.text.line_height))
                    .map(|d| {
                        if open {
                            d.child(super::transcript::literal(
                                format!("{key}-task"),
                                brief,
                                window,
                                cx,
                            ))
                        } else {
                            d.truncate().child(line)
                        }
                    }),
            )
            .when(long, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("{open_key}-toggle")))
                        .debug_selector(|| "task-brief-toggle".into())
                        .flex_shrink_0()
                        .h(px(tt.tool_row))
                        .flex()
                        .items_center()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .cursor_pointer()
                        .hover(move |s| s.text_color(rgb(p.text)))
                        .child(if open { "Show less" } else { "Show full task" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_open(open_key.clone(), false, cx)
                        })),
                )
            })
            .into_any_element()
    }
}

// ----- Messages ---------------------------------------------------------------

impl Workspace {
    /// One transcript row under v2: fleet wakes as cards, your turns as
    /// hugging bubbles, the agent's prose full width with its time and copy
    /// revealed on hover, tool rows with their own durations.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_message_v2(
        &mut self,
        row: &Row,
        namespace: &str,
        continued: bool,
        duration: Option<String>,
        timestamp: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.tokens();
        if row.role == "You"
            && !continued
            && let Some(fleet) = wks_native::transcript::fleet(&row.text)
        {
            let key = format!("{namespace}:{}:{}", self.chat_owner(), row.key);
            let card = self.render_fleet_card_v2(&key, fleet, row.copy_text(), window, cx);
            return div().w_full().flex().flex_col().child(card);
        }
        let content = self.render_message_content(row, namespace, continued, window, cx);
        if row.tool.is_some() || row.role != "Assistant" {
            return content;
        }
        let owner = self.chat_owner();
        let has_meta = duration.is_some() || timestamp.is_some();
        content.child(
            div()
                .h(px(t.space.xl + t.space.xs))
                .mt(px(t.space.xxs))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(t.space.xs))
                .child(self.copy_v2(
                    format!("copy-{namespace}:{owner}:{}", row.key),
                    row.copy_text(),
                ))
                .when(has_meta, |d| {
                    d.child(
                        self.timestamp_v2(duration.map(Into::into), timestamp)
                            .opacity(0.)
                            .group_hover("message", |s| s.opacity(1.)),
                    )
                }),
        )
    }

    /// Notices, compaction and task-list markers: quiet single lines.
    pub(super) fn render_quiet_row_v2(
        &mut self,
        row: &Row,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let p = self.palette();
        let t = p.tokens;
        match row.role.as_str() {
            "Plan" => self
                .render_plan_marker(key, &row.text, cx)
                .map(|marker| div().w_full().py(px(t.space.xxs)).child(marker)),
            "Notice" => Some(
                div()
                    .debug_selector(|| "notice-row".into())
                    .w_full()
                    .py(px(t.space.xs))
                    .flex()
                    .items_start()
                    .gap(px(t.space.md))
                    .text_size(px(t.text.meta))
                    .line_height(relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(
                        div()
                            .flex_shrink_0()
                            .h(px(t.text.meta * t.text.line_height))
                            .flex()
                            .items_center()
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(Icon::new(IconName::Info).size(px(t.icon.xs))),
                    )
                    .child(div().flex_1().min_w_0().child(row.text.clone())),
            ),
            "Compaction" => Some(
                div()
                    .debug_selector(|| "compaction-row".into())
                    .w_full()
                    .py(px(t.space.md))
                    .flex()
                    .items_center()
                    .gap(px(t.space.lg))
                    .text_size(px(t.text.overline))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(div().flex_1().h(px(t.stroke.hairline)).bg(rgb(p.border)))
                    .child(div().flex_shrink_0().child(row.text.clone()))
                    .child(div().flex_1().h(px(t.stroke.hairline)).bg(rgb(p.border))),
            ),
            _ => None,
        }
    }

    /// Your message's bubble: it hugs its text, up to `user_max` of the
    /// column, with no "You" label (the side and the fill say it).
    pub(super) fn user_bubble_v2(&self, body: Div) -> Div {
        let p = self.palette();
        let t = p.tokens;
        body.debug_selector(|| "user-bubble".into())
            .w_auto()
            .max_w(relative(t.transcript().user_max))
            .bg(rgb(p.user))
            .rounded(px(t.radius.panel))
            .px(px(t.space.lg + t.space.xxs))
            .py(px(t.space.md + t.space.xxs))
            .text_color(rgb(p.text))
    }

    /// Your turn: the bubble at the right edge, then one quiet line with
    /// what the agent did with it, its time and (on hover) its actions.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn user_turn_v2(
        &self,
        row: &Row,
        key: &str,
        bubble: Div,
        receipt: Option<&'static str>,
        actions: Vec<AnyElement>,
        panel: Option<Div>,
        copy: String,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let at = row
            .timestamp_ms
            .or_else(|| row.timestamp.as_deref().and_then(timing::parse_timestamp));
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(t.space.xxs))
            .child(bubble)
            .child(
                div()
                    .h(px(t.space.xl + t.space.xs))
                    .flex()
                    .items_center()
                    .gap(px(t.space.xs))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.xxs))
                            .opacity(0.)
                            .group_hover("message", |s| s.opacity(1.))
                            .children(actions),
                    )
                    .child(self.copy_v2(format!("copy-{key}"), copy))
                    .children(receipt.map(|label| {
                        div()
                            .debug_selector(|| "message-receipt".into())
                            .text_size(px(t.text.overline))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(format!("{label} ·"))
                    }))
                    .when(at.is_some(), |d| d.child(self.timestamp_v2(None, at))),
            )
            .children(panel.map(|panel| div().w_full().child(panel)))
    }

    /// What the turn changed (r2-transcript): one event row ("Changed 3
    /// files +12 −4", Review on the right) with the files on its guide. How
    /// the count was made stays on hover, never as default copy.
    pub(super) fn render_turn_changes_v2(
        &self,
        files: &[wks_native::transcript::FileChange],
        estimated: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tt = t.transcript();
        let added: usize = files.iter().map(|f| f.added).sum();
        let removed: usize = files.iter().map(|f| f.removed).sum();
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let connected = self.view.connected;
        let mut event = EventRow::new(
            "turn-changes-line",
            Glyph::Asset("lucide/file-diff.svg"),
            format!("Changed {}", plural(files.len(), "file", "files")),
        );
        // r3-transcript: counts say their scope ("This turn +5 −2"), so they
        // never read as the branch's (Changes: "Branch +17 −3").
        event.scope = Some("This turn".into());
        event.diff = (added, removed);
        if let [file] = files {
            // One file: named on the line itself.
            event.target = Some(file.path.clone().into());
            event.mono = Some(mono.clone());
        }
        // Changes has no per-turn filter yet: Review opens every uncommitted
        // change, and says so rather than implying this turn's alone.
        event.trailing.push(
            primitives::with_tooltip(
                primitives::button(
                    "turn-changes",
                    "Review",
                    Some(IconName::ArrowRight),
                    ButtonVariant::Tertiary,
                    ButtonSize::Small,
                    connected,
                    &p,
                ),
                "Review all uncommitted changes (this turn's files are listed here)",
                None,
                &p,
            )
            // Its label, not its padding, meets the column's right edge.
            .mr(px(-(t.space.md + t.space.xxs)))
            .when(connected, |d| {
                d.on_click(
                    cx.listener(|this, _, window, cx| {
                        this.open_feature(Screen::Changes, window, cx)
                    }),
                )
            })
            .into_any_element(),
        );
        let header = event_row(event, &p)
            .tooltip(primitives::tooltip_view(
                if estimated {
                    "Counted from the agent's edits this turn. Review shows the diff on disk."
                } else {
                    "Read from the working tree when the turn ended."
                },
                None,
                &p,
            ))
            .cursor_default();
        let shown = files.len().min(tt.visible_steps);
        div()
            .debug_selector(|| "turn-changes-card".into())
            .mt(px(t.space.xs))
            .w_full()
            .flex()
            .flex_col()
            .child(header)
            .when(files.len() > 1, |d| {
                d.child(
                    event_guide(&p)
                        .children(files.iter().take(shown).map(|f| {
                            div()
                                .h(px(tt.tool_row - t.space.xs))
                                .flex()
                                .items_center()
                                .gap(px(t.space.md))
                                .min_w_0()
                                .text_size(px(t.text.meta))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(mono.clone())
                                        .text_color(rgb(p.roles.text_secondary))
                                        .child(f.path.clone()),
                                )
                                .when(f.added > 0, |d| {
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child(format!("+{}", f.added)),
                                    )
                                })
                                .when(f.removed > 0, |d| {
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child(format!("−{}", f.removed)),
                                    )
                                })
                        }))
                        .when(files.len() > shown, |d| {
                            d.child(
                                div()
                                    .h(px(tt.tool_row - t.space.xs))
                                    .flex()
                                    .items_center()
                                    .text_size(px(t.text.meta))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child(plural(files.len() - shown, "more file", "more files")),
                            )
                        }),
                )
            })
    }
    /// The top-of-chat pager as a quiet centred link.
    pub(super) fn render_earlier_v2(&self, loading: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        div()
            .id("earlier-messages")
            .debug_selector(|| "earlier-messages".into())
            .py(px(t.space.lg))
            .flex()
            .justify_center()
            .child(
                primitives::button(
                    "earlier-messages-pill",
                    if loading {
                        "Loading earlier messages…"
                    } else {
                        "Earlier messages"
                    },
                    (!loading).then_some(IconName::ArrowUp),
                    ButtonVariant::Tertiary,
                    ButtonSize::Small,
                    !loading,
                    &p,
                )
                .when(!loading, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.command(Command::LoadOlder, cx)))
                }),
            )
    }

    /// "Jump to latest", outside the scroll content (R3.3): a solid pill
    /// with a shadow, anchored `shell().jump_gap` above the composer and
    /// decision stack, over a scrim (`shell().jump_scrim`) that dissolves
    /// the rows beneath into the canvas. With news it reads "↓ 3 new".
    /// The gap is measured from the first card, not from the dock's own
    /// top padding.
    pub(super) fn render_jump_latest_v2(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let sh = t.shell();
        let session = self.view.selected.clone().unwrap_or_default();
        let fresh = unseen(
            self.chat.seen.as_ref(),
            &session,
            self.view.transcript.rows.len(),
        );
        let label = if fresh > 0 {
            format!("{fresh} new")
        } else {
            "Jump to latest".to_owned()
        };
        div()
            .absolute()
            .left_0()
            .w_full()
            .bottom(
                self.composer_dock_bounds.size.height
                    - px(super::conversation::dock_first_surface(compact)),
            )
            // r4-calm: the transcript stops above this band
            // (`transcript_inset_v2`), so the pill floats over canvas only.
            .h(px(t.layout.control_height_sm + 2. * sh.jump_gap))
            .flex()
            .items_end()
            .justify_center()
            .pb(px(sh.jump_gap))
            .child(
                primitives::fade_mask(FadeEdge::Bottom, p.chat, &p)
                    .debug_selector(|| "jump-latest-scrim".into())
                    .h_full(),
            )
            .child(
                primitives::button(
                    "latest",
                    label,
                    Some(IconName::ArrowDown),
                    ButtonVariant::Secondary,
                    ButtonSize::Small,
                    true,
                    &p,
                )
                .debug_selector(|| "jump-latest".into())
                .when(fresh > 0, |d| {
                    d.tooltip(primitives::tooltip_view("Jump to latest", None, &p))
                        .child(div().debug_selector(|| "jump-latest-new".into()))
                })
                .relative()
                .rounded(px(t.radius.full))
                .bg(rgb(p.roles.raised))
                .map(|d| system::floating(d, &p))
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset {
                        item_ix: this.view.transcript.rows.len(),
                        offset_in_item: Pixels::ZERO,
                    });
                    cx.notify();
                })),
            )
    }

    /// Remember how far the reader has seen while following, for "n new".
    pub(super) fn note_seen_rows(&mut self) {
        if self.follow {
            let session = self.view.selected.clone().unwrap_or_default();
            self.chat.seen = Some((session, self.view.transcript.rows.len()));
        }
    }
}

// ----- Fleet and supervisor wakes ------------------------------------------------

impl Workspace {
    /// A fleet or supervisor wake (r3-transcript): another session's news,
    /// so never a card here. One line per agent in the shared lexicon
    /// ("Release checklist for 0.9 needs you · Go", "Usage meter rounding ·
    /// Done", "Queue priority ordering · Failed · Credit balance is too
    /// low"), no internal ids. Clicking a line opens what the wake carried
    /// (last reply, results, why it was raised) on a guide beneath.
    pub(super) fn render_fleet_card_v2(
        &mut self,
        key: &str,
        fleet: Fleet,
        copy: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let owner = self.view.selected.clone().unwrap_or_default();
        let lookup = |id: &str| self.view.sessions.iter().find(|s| s.id == id).cloned();
        let states: Vec<StatusKind> = fleet
            .entries
            .iter()
            .map(|entry| {
                let (label, _) = super::fleet_card::entry_status(
                    fleet.kind,
                    entry,
                    lookup(&entry.session_id).as_ref(),
                    p,
                );
                fleet_entry_kind(label)
            })
            .collect();
        let pending = states.contains(&StatusKind::NeedsYou);
        let eyebrow = fleet_eyebrow(fleet.kind, pending, fleet.entries.len());
        let names = fleet
            .entries
            .iter()
            .map(|entry| {
                lookup(&entry.session_id)
                    .map(|s| self.session_title(&s))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| entry.label.clone())
            })
            .collect::<Vec<_>>();
        let open_key = format!("{key}-v2-open");
        let open = self.chat.open.get(&open_key).copied().unwrap_or(false);
        // R5.4: other sessions' news is one quiet line with one button
        // ("Go to Release checklist"), set apart from this conversation:
        // an inbox mark, "From other sessions", tertiary words. An ask that
        // has since been answered reads as resolved, never as pending.
        let words: Vec<(String, StatusKind)> = fleet
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let (label, _) = super::fleet_card::entry_status(
                    fleet.kind,
                    entry,
                    lookup(&entry.session_id).as_ref(),
                    p,
                );
                let word = fleet_word(label);
                let line = match &entry.failed {
                    Some(why) => format!("{} {word}: {why}", names[index]),
                    None => format!("{} {word}", names[index]),
                };
                (line, states[index])
            })
            .collect();
        // The one button: the session that waits on you now (live, not the
        // wake's say-so), else the first one this conversation can open.
        let waits = |index: usize| {
            states[index] == StatusKind::NeedsYou
                && self.view.connected
                && lookup(&fleet.entries[index].session_id)
                    .is_some_and(|s| s.approval.is_some() || s.questions.is_some())
        };
        let target = (0..fleet.entries.len()).find(|&i| waits(i)).or_else(|| {
            (0..fleet.entries.len())
                .find(|&i| self.fleet_openable(&owner, &fleet.entries[i].session_id))
        });
        let worst = [StatusKind::NeedsYou, StatusKind::Failed]
            .into_iter()
            .find(|kind| {
                states
                    .iter()
                    .enumerate()
                    .any(|(i, k)| k == kind && (*kind != StatusKind::NeedsYou || waits(i)))
            });
        let button = target.map(|index| {
            let id = fleet.entries[index].session_id.clone();
            let owner = owner.clone();
            let name = super::status::short_title(&names[index], 32);
            primitives::button(
                SharedString::from(format!("{key}-open-{index}")),
                format!("Go to {name}"),
                Some(IconName::ArrowRight),
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                true,
                &p,
            )
            .debug_selector(move || format!("fleet-entry-open-{index}"))
            // Its label, not its padding, meets the column's right edge.
            .mr(px(-(t.space.md + t.space.xxs)))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let needs_you = this
                    .view
                    .sessions
                    .iter()
                    .any(|s| s.id == id && (s.approval.is_some() || s.questions.is_some()));
                if this.view.selected.as_deref() == Some(owner.as_str())
                    && this.view.connected
                    && (needs_you || this.fleet_openable(&owner, &id))
                {
                    this.command(Command::Select(id.clone()), cx);
                }
            }))
        });
        let summary = words
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let toggle = open_key.clone();
        let line = div()
            .id(SharedString::from(format!("{key}-line")))
            .debug_selector(|| "fleet-card-header".into())
            .group("event-row")
            .h(px(t.transcript().tool_row))
            .mx(px(-t.space.sm))
            .px(px(t.space.sm))
            .rounded(px(t.radius.control))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .min_w_0()
            .cursor_pointer()
            .hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_tertiary))
            .child(match worst {
                Some(kind) => glyph_box(Glyph::Status(kind), p.roles.text_tertiary, &p),
                None => glyph_box(Glyph::Named(IconName::Inbox), p.roles.text_tertiary, &p),
            })
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight(t.weight.medium))
                    .child("From other sessions"),
            )
            .child(div().min_w_0().flex_1().truncate().child(summary))
            .children((0..fleet.entries.len()).map(|index| {
                div()
                    .absolute()
                    .debug_selector(move || format!("fleet-entry-status-{index}"))
            }))
            .when(fleet.entries.len() == 1, |d| {
                d.child(
                    div()
                        .absolute()
                        .debug_selector(|| "fleet-card-status".into()),
                )
            })
            .children(button)
            .tooltip(primitives::tooltip_view(
                format!("From another session · {eyebrow}"),
                None,
                &p,
            ))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_open(toggle.clone(), false, cx)),
            );
        let lines = div()
            .debug_selector(|| "fleet-card".into())
            .pb(px(t.space.sm))
            .w_full()
            .flex()
            .flex_col()
            .child(line);
        if !open {
            return lines;
        }
        // What the wake carried, on the guide: each agent's note and last
        // reply, structured results, and the raw wake one disclosure away.
        let mut details = event_guide(&p)
            .debug_selector(|| "fleet-details".into())
            .gap(px(t.space.xs))
            .pb(px(t.space.sm))
            .text_size(px(t.text.meta))
            .text_color(rgb(p.roles.text_secondary));
        for (index, entry) in fleet.entries.iter().enumerate() {
            let quote = |text: String| {
                system::edge(
                    div().pl(px(t.space.md)),
                    system::Side::Left,
                    system::EdgeTone::Quote,
                    &p,
                )
                .text_size(px(t.text.body))
                .text_color(rgb(p.prose))
                .child(text)
            };
            let reply = format!("Re: {} — ", names[index]);
            let single = fleet.entries.len() == 1;
            details = details.child(
                div()
                    .debug_selector(move || format!("fleet-entry-details-{index}"))
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xs))
                    .when(!single, |d| {
                        d.child(
                            div()
                                .font_weight(FontWeight(t.weight.medium))
                                .text_color(rgb(p.text))
                                .child(names[index].clone()),
                        )
                    })
                    .when_some(entry.note.clone(), |d, note| d.child(quote(note)))
                    .when_some(entry.last_reply.clone(), |d, last| d.child(quote(last)))
                    .child(
                        div().flex().child(
                            primitives::button(
                                SharedString::from(format!("{key}-reply-{index}")),
                                "Reply",
                                Some(IconName::Undo2),
                                ButtonVariant::Tertiary,
                                ButtonSize::Small,
                                self.view.connected,
                                &p,
                            )
                            .ml(px(-t.space.md))
                            .debug_selector(move || format!("fleet-entry-reply-{index}"))
                            .on_click(cx.listener(
                                move |this, _, window, cx| this.prefill(&reply, window, cx),
                            )),
                        ),
                    ),
            );
        }
        for (i, (title, report)) in fleet.reports.iter().enumerate() {
            let report_key = format!("{key}-report-{i}");
            details = details.child(if title.starts_with("Structured result") {
                self.render_result(
                    &report_key,
                    &super::fleet_card::report_title(title),
                    report,
                    window,
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xs))
                    .child(
                        div()
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(super::fleet_card::report_title(title)),
                    )
                    .child(self.render_raw(&report_key, report, window, cx))
                    .into_any_element()
            });
        }
        let original = format!("{key}-original");
        let expanded = self.chat.open.get(&original).copied().unwrap_or(false);
        details = details.child(
            div()
                .flex()
                .items_center()
                .gap(px(t.space.md))
                .child(self.disclosure_v2(original.clone(), "Why this was raised", expanded, cx))
                .child(
                    primitives::button(
                        SharedString::from(format!("copy-{key}")),
                        "Copy",
                        Some(IconName::Copy),
                        ButtonVariant::Tertiary,
                        ButtonSize::Small,
                        true,
                        &p,
                    )
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
                ),
        );
        if expanded {
            details = details.child(self.render_raw(&original, &fleet.text, window, cx));
        }
        lines.child(details)
    }

    /// A small chevron toggle that keeps the reader's place.
    pub(super) fn disclosure_v2(
        &self,
        key: String,
        label: &'static str,
        open: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        chrome::interactive_control(
            div().id(SharedString::from(format!("toggle-{key}"))),
            p,
            true,
        )
        .debug_selector(move || format!("fleet-toggle-{label}"))
        .flex()
        .items_center()
        .gap(px(t.space.xs))
        .h(px(t.layout.control_height_sm))
        .ml(px(-t.space.xs))
        .px(px(t.space.xs))
        .rounded(px(t.radius.control))
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.medium))
        .text_color(rgb(p.roles.text_secondary))
        .hover(move |s| s.text_color(rgb(p.text)))
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(px(t.icon.xs)),
        )
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_open(key.clone(), false, cx)))
    }
}

// ----- Subagents -----------------------------------------------------------------

/// How one subagent reads right now (r2-transcript): one honest state. A
/// running agent that has gone quiet past `HEARTBEAT_STALE_MS` is stalled,
/// a fault reading "Not responding · 2m", never "Working" beside a warning.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChildView {
    pub kind: StatusKind,
    pub word: String,
    pub stalled: bool,
    /// Elapsed (running) or total (finished) time, when known.
    pub clock: Option<String>,
}

pub(super) fn child_view(child: &ChildAgent, connected: bool, now: i64) -> ChildView {
    let last = child
        .telemetry
        .last_activity_ms
        .or(child.telemetry.started_at_ms);
    if connected && child.running() && format::is_stale(last, now) {
        return ChildView {
            kind: StatusKind::NotResponding,
            word: format::not_responding(last, now),
            stalled: true,
            clock: None,
        };
    }
    let (kind, word) = child_status(&child.status, connected);
    let at = if connected {
        Some(now)
    } else {
        child
            .telemetry
            .last_activity_ms
            .or(child.telemetry.completed_at_ms)
    };
    ChildView {
        kind,
        word: word.into(),
        stalled: false,
        clock: at
            .and_then(|at| child.duration_ms(at))
            .map(|ms| format::duration(ms.min(i64::MAX as u64) as i64)),
    }
}

/// A batch of subagents in one status, most urgent first: stalled, then
/// blocked on you, working, failed, done. `None` while nothing has started.
pub(super) fn children_badge(views: &[ChildView]) -> Option<StatusBadge> {
    let count = |f: &dyn Fn(&ChildView) -> bool| views.iter().filter(|v| f(v)).count();
    let stalled = count(&|v| v.stalled);
    let blocked = count(&|v| v.kind == StatusKind::NeedsYou);
    let working = count(&|v| v.kind == StatusKind::Working);
    let failed = count(&|v| v.kind == StatusKind::Failed && !v.stalled);
    let done = count(&|v| v.kind == StatusKind::Done);
    let label = |n: usize, word: &str| {
        if views.len() == 1 {
            let mut w = word.to_owned();
            if let Some(first) = w.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            w
        } else {
            format!("{n} {word}")
        }
    };
    if stalled > 0 {
        let word = if views.len() == 1 {
            views.iter().find(|v| v.stalled).map(|v| v.word.clone())
        } else {
            None
        };
        return Some(
            StatusBadge::new(StatusKind::NotResponding)
                .label(word.unwrap_or_else(|| format!("{stalled} not responding"))),
        );
    }
    if blocked > 0 {
        return Some(StatusBadge::new(StatusKind::NeedsYou).label(label(blocked, "need you")));
    }
    if working > 0 {
        return Some(StatusBadge::new(StatusKind::Working).label(label(working, "working")));
    }
    if failed > 0 {
        return Some(StatusBadge::new(StatusKind::Failed).label(label(failed, "failed")));
    }
    (done > 0 && done == views.len()).then(|| StatusBadge::new(StatusKind::Done))
}

/// What a stalled subagent's Nudge does.
enum Nudge {
    /// A session of its own: send it [`NUDGE_TEXT`] to read at once.
    Session(String),
    /// A subagent inside the agent's tools: draft this note to the agent.
    Parent(String),
}

/// What Nudge sends a stalled agent session.
pub(super) const NUDGE_TEXT: &str = "Still there? Reply with where you are.";

/// The note Nudge drafts for the agent that runs a stalled subagent.
pub(super) fn nudge_parent_prompt(name: &str, state: &str) -> String {
    format!(
        "Your subagent “{name}” has gone quiet ({}). Check on it and tell me where it stands; stop it if it is stuck.",
        state.to_lowercase()
    )
}

/// A child row's name and its quieter context, never repeating the title
/// of the delegation it sits under: then it is named by role or model.
pub(super) fn child_names(child: &ChildAgent, parent_title: Option<&str>) -> (String, String) {
    let same = |a: &str| {
        parent_title.is_some_and(|t| !a.is_empty() && t.trim().eq_ignore_ascii_case(a.trim()))
    };
    let model = format::model_name(&child.model);
    let role = [child.agent_type.as_str(), child.label.as_str()]
        .into_iter()
        .find(|r| !r.is_empty() && !same(r) && *r != child.description)
        .unwrap_or("")
        .to_owned();
    let task = if same(&child.description) {
        ""
    } else {
        child.description.as_str()
    };
    let primary = [task, role.as_str(), model.as_str()]
        .into_iter()
        .find(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "Agent".into());
    let context = [role.as_str(), model.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty() && *s != primary)
        .collect::<Vec<_>>()
        .join(" · ");
    (primary, context)
}

/// Diagnostics a child row keeps for hover: raw model id, tools, tokens,
/// cost and where it runs.
fn child_details(child: &ChildAgent, managed: bool) -> String {
    let mut parts = vec![if managed {
        "Agent session started by Workspacer".to_owned()
    } else {
        "Subagent run by the agent's own tools".to_owned()
    }];
    if !child.model.is_empty() {
        parts.push(child.model.clone());
    }
    if let Some(tools) = child.telemetry.tool_calls {
        parts.push(plural(tools as usize, "tool call", "tool calls"));
    }
    if let Some(tokens) = child.telemetry.tokens {
        parts.push(format!("{} tokens", compact_count(tokens)));
    }
    if let Some(cost) = child.telemetry.cost_usd {
        parts.push(format!("${cost:.3}"));
    }
    parts.join(" · ")
}

fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.)
    } else {
        value.to_string()
    }
}

impl Workspace {
    /// Subagents as event rows (r2-transcript): a status glyph, the task
    /// (or role/model under a delegation that already names it), what it
    /// is doing now, and its time or state on the right. A stalled agent
    /// offers Check and Stop in place.
    pub(super) fn render_children_v2(
        &mut self,
        parent: &str,
        key: &str,
        children: &[ChildAgent],
        parent_title: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let now = timing::now_ms();
        let connected = self.view.connected;
        let mut body = div().flex().flex_col();
        for child in children {
            let child_key = format!("{key}-child-{}", child.id);
            let managed = child.session_id.is_some();
            let known = child
                .session_id
                .as_ref()
                .map(|id| {
                    self.view.sessions.iter().any(|s| {
                        &s.id == id
                            && (s.parent_session_id.is_empty() || s.parent_session_id == parent)
                    })
                })
                .unwrap_or_else(|| {
                    self.selected_session()
                        .and_then(|s| s.subagents.as_array())
                        .is_some_and(|items| items.iter().any(|s| s["id"] == child.id))
                });
            let enabled = connected && known;
            let view = child_view(child, connected, now);
            let (primary, context) = child_names(child, parent_title);
            let title = primary.clone();
            let last_tool = (!child.telemetry.last_tool_name.is_empty()
                && view.kind == StatusKind::Working)
                .then(|| {
                    if child.telemetry.last_tool_summary.is_empty() {
                        child.telemetry.last_tool_name.clone()
                    } else {
                        format!(
                            "{} {}",
                            child.telemetry.last_tool_name, child.telemetry.last_tool_summary
                        )
                    }
                });
            let mut event = EventRow::new(
                format!("{child_key}-open"),
                Glyph::Status(view.kind),
                primary,
            );
            event.quiet = true;
            event.target = (!context.is_empty()).then(|| context.into());
            event.summary = last_tool.map(Into::into);
            event.end = match view.kind {
                _ if view.stalled => {
                    EventEnd::Badge(StatusBadge::new(view.kind).label(view.word.clone()))
                }
                StatusKind::Failed | StatusKind::NeedsYou => {
                    EventEnd::Badge(StatusBadge::new(view.kind).label(view.word.clone()))
                }
                StatusKind::Working | StatusKind::Done => view
                    .clock
                    .clone()
                    .map_or(EventEnd::Nothing, EventEnd::Quiet),
                _ => EventEnd::Quiet(view.word.clone()),
            };
            let open = {
                let owner = parent.to_owned();
                let child_id = child.id.clone();
                let target = child.session_id.clone();
                move |this: &mut Self, cx: &mut Context<Self>| {
                    if this.view.selected.as_deref() != Some(&owner) || !this.view.connected {
                        return;
                    }
                    if let Some(id) = &target {
                        if this.view.sessions.iter().any(|s| {
                            &s.id == id
                                && (s.parent_session_id.is_empty() || s.parent_session_id == owner)
                        }) {
                            this.command(Command::Select(id.clone()), cx);
                        }
                    } else {
                        this.child_ui.page = 0;
                        this.child_ui.closed = None;
                        this.request(
                            wks_native::features::Request::SubagentHistory {
                                session: owner.clone(),
                                agent: child_id.clone(),
                            },
                            cx,
                        );
                    }
                }
            };
            let open = std::rc::Rc::new(open);
            // r3-transcript: a stalled agent says what can be done: look at
            // what it did, nudge it, or stop it (never a bare "Check").
            let actions = view.stalled.then(|| {
                let view_output = open.clone();
                let stop_target = child.session_id.clone();
                let nudge = match &child.session_id {
                    // Its own session: a message it reads at once.
                    Some(id) => Nudge::Session(id.clone()),
                    // A subagent inside the agent's own tools has no inbox of
                    // its own: ask the agent that runs it, drafted for you.
                    None => Nudge::Parent(nudge_parent_prompt(&title, &view.word)),
                };
                let nudge_tip = match &nudge {
                    // r4-calm (R4.6): what a nudge is, in one line.
                    Nudge::Session(_) => {
                        "Nudge · ask it to report back: sends “Still there? Reply with where you are.” now"
                    }
                    Nudge::Parent(_) => {
                        "Nudge · ask it to report back: drafts a note asking the agent that runs it to check on it"
                    }
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xs))
                    .flex_shrink_0()
                    .child(
                        primitives::button(
                            SharedString::from(format!("{child_key}-view")),
                            "View output",
                            None,
                            ButtonVariant::Tertiary,
                            ButtonSize::Small,
                            enabled,
                            &p,
                        )
                        .debug_selector(|| "stalled-child-check".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            view_output(this, cx)
                        })),
                    )
                    .child(
                        primitives::with_tooltip(
                            primitives::button(
                                SharedString::from(format!("{child_key}-nudge")),
                                "Nudge",
                                None,
                                ButtonVariant::Secondary,
                                ButtonSize::Small,
                                connected,
                                &p,
                            ),
                            nudge_tip,
                            None,
                            &p,
                        )
                        .debug_selector(|| "stalled-child-nudge".into())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                match &nudge {
                                    Nudge::Session(id) => this.command(
                                        Command::Act {
                                            session: id.clone(),
                                            action: Action::SendNow(NUDGE_TEXT.into()),
                                        },
                                        cx,
                                    ),
                                    Nudge::Parent(text) => this.prefill(text, window, cx),
                                }
                            },
                        )),
                    )
                    .when_some(stop_target, |d, id| {
                        d.child(
                            primitives::button(
                                SharedString::from(format!("{child_key}-stop")),
                                "Stop",
                                None,
                                ButtonVariant::Destructive,
                                ButtonSize::Small,
                                connected,
                                &p,
                            )
                            .debug_selector(|| "stalled-child-stop".into())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.command(
                                        Command::Act {
                                            session: id.clone(),
                                            action: Action::Stop,
                                        },
                                        cx,
                                    );
                                },
                            )),
                        )
                    })
            });
            event
                .trailing
                .extend(actions.map(IntoElement::into_any_element));
            let row = chrome::interactive_control(event_row(event, &p), p, enabled)
                .px(px(t.space.sm - t.stroke.focus))
                .debug_selector(move || {
                    if managed {
                        "open-spawned-session".into()
                    } else {
                        "open-provider-subagent".into()
                    }
                })
                .tooltip(primitives::tooltip_view(
                    child_details(child, managed),
                    None,
                    &p,
                ))
                .child(div().debug_selector(move || {
                    if managed {
                        "workspacer-child-icon".into()
                    } else {
                        "provider-child-icon".into()
                    }
                }))
                .when(enabled, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| open(this, cx)))
                })
                .when(!enabled, |d| d.cursor_default());
            let mut item = div()
                .id(SharedString::from(child_key.clone()))
                .debug_selector({
                    let id = child.id.clone();
                    move || format!("child-agent-{id}")
                })
                .flex()
                .flex_col()
                .child(row);
            if child.session_id.is_none() {
                item = item.child(
                    self.render_child_transcript(parent, &child.id, &title, &child_key, window, cx),
                );
            }
            body = body.child(item);
        }
        body
    }

    /// A batch of subagents with no delegation in view (r2-transcript): an
    /// event row ("3 subagents · 1 working") over their rows, open while any
    /// works, folded once they have all settled.
    pub(super) fn render_overview_v2(
        &mut self,
        session: &str,
        children: &[ChildAgent],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.palette();
        let t = p.tokens;
        let total = children.len();
        let settled = children.iter().all(ChildAgent::settled);
        let first = children.first().map(|c| c.id.as_str()).unwrap_or_default();
        let key = format!("overview:{session}:{first}:{settled}");
        let open = self.chat.open.get(&key).copied().unwrap_or(!settled);
        let now = timing::now_ms();
        let views: Vec<ChildView> = children
            .iter()
            .map(|c| child_view(c, self.view.connected, now))
            .collect();
        let span = || {
            let start = children
                .iter()
                .filter_map(|c| c.telemetry.started_at_ms)
                .min()?;
            let end = children
                .iter()
                .filter_map(|c| c.telemetry.completed_at_ms.or(c.telemetry.last_activity_ms))
                .max()?;
            (end - start >= 1000).then(|| format::duration(end - start))
        };
        let toggle = key.clone();
        let mut event = EventRow::new(
            format!("{key}-toggle"),
            Glyph::Named(IconName::Bot),
            // R5.4: this block knows one turn's delegations; the sidebar
            // counts the whole tree. Say which this is.
            format!("{total} delegated this turn"),
        );
        event.disclosure = Some(open);
        event.end = match children_badge(&views) {
            Some(badge) => EventEnd::Badge(badge),
            None => settled
                .then(span)
                .flatten()
                .map_or(EventEnd::Nothing, EventEnd::Quiet),
        };
        div()
            .id(SharedString::from(format!("{key}-card")))
            .debug_selector(|| "subagent-overview".into())
            .w_full()
            .flex()
            .flex_col()
            .child(
                event_row(event, &p)
                    .debug_selector(|| "subagent-overview-toggle".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle.clone(), !settled, cx)
                    })),
            )
            .when(open, |d| {
                d.child(
                    event_guide(&p)
                        .pb(px(t.space.xs))
                        .child(self.render_children_v2(session, &key, children, None, window, cx)),
                )
            })
    }
}
// ----- Reasoning ---------------------------------------------------------------

impl Workspace {
    /// The model's reasoning under v2: one quiet line in the tool-row
    /// anatomy ("Thought for 12s · opening line"), pulsing while it streams
    /// (never the brand mark), opening into its text on a hairline guide.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn thinking_v2(
        &self,
        row: &Row,
        key: &str,
        (label, preview): (String, String),
        open: bool,
        live: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let open_key = format!("{key}-thinking");
        let toggle_key = open_key.clone();
        let breath = if live {
            gpui::bounce(gpui::ease_in_out)(super::motion::looping(
                std::time::Duration::from_millis(t.motion.pulse_ms as u64),
                0.5,
            ))
        } else {
            1.
        };
        div()
            .debug_selector(|| "thinking-row".into())
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("{open_key}-toggle")))
                    .debug_selector(|| "thinking-toggle".into())
                    .group("tool-line")
                    .h(px(t.transcript().tool_row))
                    .mx(px(-t.space.sm))
                    .px(px(t.space.sm))
                    .rounded(px(t.radius.control))
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .min_w_0()
                    .cursor_pointer()
                    .text_size(px(t.text.meta))
                    .hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
                    .child(
                        div()
                            .size(px(t.transcript().event_icon))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(if live && !open {
                                primitives::status_indicator(StatusKind::Working, &p)
                                    .into_any_element()
                            } else {
                                Icon::new(if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(t.icon.xs))
                                .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(p.roles.text_secondary))
                            .opacity(0.55 + 0.45 * breath)
                            .child(label),
                    )
                    .when(!open && !preview.is_empty(), |d| {
                        d.child(
                            div()
                                .debug_selector(|| "thinking-preview".into())
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(p.roles.text_tertiary))
                                .child(preview),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle_key.clone(), false, cx)
                    })),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .debug_selector(|| "thinking-body".into())
                        .mt(px(t.space.xxs))
                        .mb(px(t.space.sm))
                        .ml(px(t.transcript().event_icon / 2.))
                        .pl(px(
                            t.transcript().event_icon / 2. + t.space.md - t.stroke.hairline
                        ))
                        .map(|d| system::edge(d, system::Side::Left, system::EdgeTone::Divider, &p))
                        .child(
                            self.render_markdown(
                                &format!("{key}-thinking-text"),
                                &row.text,
                                window,
                                cx,
                            )
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_secondary)),
                        ),
                )
            })
    }
}

// ----- r3-transcript: expand all (Ctrl+O) ---------------------------------------

gpui::actions!(native, [ExpandAllTools]);

impl Workspace {
    /// Ctrl+O (v2): every work block open with all its tools, or back to the
    /// defaults (finished work folded to one line). Keeps the reader's place.
    pub(super) fn toggle_expand_all(&mut self, cx: &mut Context<Self>) {
        if !self.design().is_v2() {
            return;
        }
        self.chat.expand_all = !self.chat.expand_all;
        self.pause_follow();
        let anchor = self.scroll_anchor();
        let count = self.view.transcript.rows.len();
        self.list.splice(0..count, count);
        self.list.scroll_to(anchor);
        cx.notify();
    }
}

// ----- Settings → Chat → Reading width ---------------------------------------

impl Workspace {
    /// Comfortable / Wide / Full, as one segmented control.
    pub(super) fn render_reading_width(&self, cx: &mut Context<Self>) -> Div {
        use wks_native::navigation::ReadingWidth;
        let p = self.palette();
        let current = self.settings.reading_width;
        primitives::segmented(&p)
            .debug_selector(|| "settings-reading-width".into())
            .children(ReadingWidth::ALL.into_iter().map(|width| {
                primitives::segment(
                    SharedString::from(format!("reading-width-{}", width.label().to_lowercase())),
                    width.label(),
                    width == current,
                    &p,
                )
                .debug_selector(move || format!("reading-width-{}", width.label().to_lowercase()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_reading_width(width, cx)))
            }))
    }

    pub(super) fn set_reading_width(
        &mut self,
        width: wks_native::navigation::ReadingWidth,
        cx: &mut Context<Self>,
    ) {
        if self.settings.reading_width == width {
            return;
        }
        self.settings.reading_width = width;
        // Rows re-measure at the new width.
        let count = self.view.transcript.rows.len();
        self.list.splice(0..count, count);
        self.save_settings(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, command: &str, failed: bool) -> Row {
        Row {
            tool: Some(wks_native::transcript::Tool {
                name: name.into(),
                input: serde_json::json!({ "command": command, "file_path": command }).to_string(),
                complete: true,
                is_error: failed,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_failure_is_recovered_only_by_a_passing_rerun() {
        // R5.4: cargo test failed, then passed: the run ends well.
        let fixed = [
            call("Bash", "cargo test", true),
            call("Edit", "src/a.rs", false),
            call("Bash", "cargo test", false),
        ];
        assert!(recovered(fixed.iter()));
        // A read after a failed build fixes nothing.
        let open = [
            call("Bash", "cargo test", true),
            call("Read", "src/a.rs", false),
        ];
        assert!(!recovered(open.iter()));
        // Nothing failed: nothing to recover from.
        assert!(!recovered([call("Read", "src/a.rs", false)].iter()));
    }

    #[test]
    fn tool_rows_say_what_a_call_produced_in_a_few_words() {
        assert_eq!(
            tool_result_meta("Read", "Read", "1 fn main() {\n2 }\n3\n").as_deref(),
            Some("3 lines")
        );
        assert_eq!(
            tool_result_meta("Search", "Grep", "src/main.rs:2: x").as_deref(),
            Some("1 match")
        );
        assert_eq!(
            tool_result_meta("Search", "Glob", "a.rs\nb.rs").as_deref(),
            Some("2 files")
        );
        assert_eq!(
            tool_result_meta("Search", "Grep", "").as_deref(),
            Some("No matches")
        );
        assert_eq!(tool_result_meta("Edit", "Edit", "File updated."), None);
        assert_eq!(tool_result_meta("Command", "Bash", ""), None);
    }

    #[test]
    fn short_output_and_failures_read_inline() {
        let fail = "Exit code 101\nerror[E0599]: no function\n --> tests/a.rs:5:32\nerror: could not compile";
        assert_eq!(exit_code(fail), Some(101));
        assert_eq!(exit_code("all good"), None);
        let (lines, before) = inline_output("Command", fail, true).unwrap();
        assert_eq!(before, 0);
        assert_eq!(lines.len(), 3, "the exit line stays on the row: {lines:?}");
        // Shared indentation goes.
        let (lines, _) = inline_output("Command", "    Checking a\n    Finished b", false).unwrap();
        assert_eq!(lines, ["Checking a", "Finished b"]);
        // Long output stays folded unless it failed (then its tail shows).
        let long = (0..9)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(inline_output("Command", &long, false), None);
        let (tail, before) = inline_output("Command", &long, true).unwrap();
        assert_eq!((tail.len(), before), (INLINE_LINES, 9 - INLINE_LINES));
        assert_eq!(tail.last().map(String::as_str), Some("line 8"));
        // Reads and edits never inline their file.
        assert_eq!(inline_output("Read", "1 fn main() {}", false), None);
        assert_eq!(inline_output("Edit", "updated", false), None);
        assert_eq!(inline_output("Command", "", false), None);
    }

    #[test]
    fn a_nudge_to_the_parent_names_the_subagent_and_its_silence() {
        let note = nudge_parent_prompt("Trace session restore paths", "Not responding · 7m");
        assert!(note.contains("“Trace session restore paths”"), "{note}");
        assert!(note.contains("not responding · 7m"), "{note}");
    }

    #[test]
    fn subagent_states_use_the_shared_vocabulary() {
        assert_eq!(child_status("running", true).0, StatusKind::Working);
        assert_eq!(child_status("completed", true), (StatusKind::Done, "Done"));
        assert_eq!(
            child_status("waiting_approval", true).0,
            StatusKind::NeedsYou
        );
        assert_eq!(child_status("error", true).0, StatusKind::Failed);
        assert_eq!(
            child_status("running", false),
            (StatusKind::Offline, "Offline")
        );
        assert_eq!(child_status("lost", true), (StatusKind::Failed, "Failed"));
        assert_eq!(
            child_status("starting", true),
            (StatusKind::Working, "Working")
        );
        assert_eq!(child_status("stopped", true), (StatusKind::Idle, "Idle"));
        assert_eq!(child_status("idle", true), (StatusKind::Idle, "Idle"));
    }

    #[test]
    fn a_stalled_subagent_is_a_fault_not_working() {
        let now = 1_000_000_000_000;
        let mut child = ChildAgent {
            status: "running".into(),
            ..Default::default()
        };
        child.telemetry.started_at_ms = Some(now - 35 * 86_400_000);
        child.telemetry.last_activity_ms = Some(now - 7 * 60_000);
        let view = child_view(&child, true, now);
        assert!(view.stalled);
        assert_eq!(view.kind, StatusKind::NotResponding);
        assert_ne!(view.kind, StatusKind::Working);
        assert_eq!(view.word, "Not responding · 7m");
        assert_eq!(view.clock, None);
        // The batch says so too, before anything else.
        let badge = children_badge(std::slice::from_ref(&view)).unwrap();
        assert_eq!(badge.kind, StatusKind::NotResponding);
        assert_eq!(
            badge.label.as_ref().map(|l| l.as_ref()),
            Some("Not responding · 7m")
        );
        // A live one shows its humanised duration.
        child.telemetry.started_at_ms = Some(now - 125_000);
        child.telemetry.last_activity_ms = Some(now - 1_000);
        let view = child_view(&child, true, now);
        assert_eq!((view.kind, view.stalled), (StatusKind::Working, false));
        assert_eq!(view.clock.as_deref(), Some("2m 05s"));
        // Offline is never a fault on the agent's part.
        assert!(!child_view(&child, false, now + 3_600_000).stalled);
    }

    #[test]
    fn child_rows_never_repeat_their_delegations_title() {
        let child = ChildAgent {
            label: "Session creation review".into(),
            description: "Session creation review".into(),
            model: "gpt-5.6-sol".into(),
            ..Default::default()
        };
        let (primary, context) = child_names(&child, Some("Session creation review"));
        assert_eq!(primary, "GPT-5.6 Sol");
        assert_eq!(context, "");
        let explore = ChildAgent {
            label: "Explore".into(),
            description: "Audit settings search".into(),
            model: "claude-haiku-4-5".into(),
            ..Default::default()
        };
        let (primary, context) = child_names(&explore, None);
        assert_eq!(primary, "Audit settings search");
        assert_eq!(context, "Explore · Haiku 4.5");
    }

    #[test]
    fn events_read_as_verbs_and_groups_as_worked() {
        assert_eq!(tool_verb("Read", "Read", false), Some("Read"));
        assert_eq!(tool_verb("Edit", "Edit", true), Some("Editing"));
        assert_eq!(tool_verb("Bash", "Command", false), Some("Ran"));
        assert_eq!(
            tool_verb("mcp__fs__write_file", "Edit", false),
            Some("Wrote")
        );
        assert_eq!(tool_verb("custom_tool", "Tool", false), None);
        assert_eq!(
            work_line(false, Some(340_000), 6),
            "Worked 5m 40s · 6 tools"
        );
        assert_eq!(work_line(true, None, 3), "Working · 3 tools");
        assert_eq!(work_line(false, Some(300), 1), "Worked · 1 tool");
    }

    #[test]
    fn fleet_eyebrows_follow_the_state_never_needed_beside_resolved() {
        assert_eq!(
            fleet_eyebrow(FleetKind::Blocked, true, 1),
            "Decision needed"
        );
        assert_eq!(fleet_eyebrow(FleetKind::Blocked, false, 1), "Decision made");
        assert_eq!(
            fleet_eyebrow(FleetKind::Finished, false, 2),
            "2 agents finished"
        );
        assert_eq!(
            fleet_eyebrow(FleetKind::Finished, false, 1),
            "Agent finished"
        );
        assert_eq!(fleet_entry_kind("Resolved"), StatusKind::Done);
        assert_eq!(fleet_entry_kind("Needs approval"), StatusKind::NeedsYou);
        assert_eq!(fleet_entry_kind("Failed"), StatusKind::Failed);
    }

    #[test]
    fn new_rows_count_only_for_the_session_they_were_seen_in() {
        let seen = ("a".to_owned(), 10);
        assert_eq!(unseen(Some(&seen), "a", 13), 3);
        assert_eq!(unseen(Some(&seen), "b", 13), 0);
        assert_eq!(unseen(None, "a", 13), 0);
        assert_eq!(unseen(Some(&seen), "a", 4), 0);
    }
}
