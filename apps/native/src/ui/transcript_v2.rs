//! Design v2's transcript (area-transcript): the reading column, CLI-dense
//! tool rows, one card anatomy for subagents and fleet wakes, hugging user
//! bubbles, quiet metadata and "Jump to latest" with its new-message count.
//!
//! Everything here runs only under `Design::V2`; the v1 renderers in
//! `transcript.rs`, `work.rs`, `tools.rs`, `children.rs` and `fleet_card.rs`
//! branch into it once and otherwise stay as they were. Every style reads
//! tokens (`docs/design-v2.md`).
use super::primitives::{
    self, ButtonSize, ButtonVariant, CardHead, FadeEdge, StatusBadge, StatusKind,
};
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

/// A response card action in plain words (r2-transcript: no "Prefill").
pub(super) fn card_action_label(action: &wks_native::transcript::CardAction) -> String {
    use wks_native::transcript::CardAction;
    match action {
        CardAction::OpenWorker { label, .. } => format!("Open {label}"),
        CardAction::ViewDiff { label, .. } => format!("View diff · {label}"),
        CardAction::FillComposer { label, .. } => format!("Reply “{label}”"),
    }
}

/// A work group's headline: "Working" while a call runs, then "Worked
/// 2m 14s" (just "Worked" under a second).
pub(super) fn work_headline(running: bool, duration_ms: Option<i64>) -> String {
    match duration_ms {
        _ if running => "Working".into(),
        Some(ms) if ms >= 1000 => format!("Worked {}", format::duration(ms)),
        _ => "Worked".into(),
    }
}

/// A subagent's status in the shared vocabulary, with the word to show.
pub(super) fn child_status(status: &str, connected: bool) -> (StatusKind, &'static str) {
    if !connected {
        return (StatusKind::Idle, "Offline");
    }
    match status {
        "running" | "working" | "thinking" | "responding" | "streaming" | "background" => {
            (StatusKind::Working, "Working")
        }
        "complete" | "completed" | "done" => (StatusKind::Done, "Done"),
        "failed" | "error" => (StatusKind::Failed, "Failed"),
        "lost" => (StatusKind::Failed, "Unavailable"),
        "approval" | "waiting_approval" | "question" | "waiting_input" => {
            (StatusKind::NeedsYou, "Needs you")
        }
        "starting" | "initializing" => (StatusKind::Working, "Starting"),
        "stopped" | "ended" => (StatusKind::Idle, "Ended"),
        _ => (StatusKind::Idle, "Idle"),
    }
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

/// What sits on an event row's right edge.
#[derive(Clone, Debug)]
pub(super) enum EventEnd {
    Nothing,
    /// A quiet duration or age, tertiary.
    Quiet(String),
    /// A status (Working pulses, a fault is red).
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
    pub diff: (usize, usize),
    /// The result in a few words ("42 lines", "3 matches").
    pub summary: Option<SharedString>,
    pub end: EventEnd,
    /// `Some(open)` draws a disclosure chevron (on hover while closed).
    pub disclosure: Option<bool>,
    /// The call failed: glyph and verb turn the fault red.
    pub failed: bool,
    /// Regular-weight verb (child rows, whose title is not a verb).
    pub quiet: bool,
    /// Controls after the status (Check / Stop, Review).
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
            diff: (0, 0),
            summary: None,
            end: EventEnd::Nothing,
            disclosure: None,
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

/// Draws one [`EventRow`]: CLI-dense, one line, the hover fill hanging into
/// the gutter so glyphs line up with the prose's left edge.
pub(super) fn event_row(row: EventRow, p: &Palette) -> Stateful<Div> {
    let p = *p;
    let t = p.tokens;
    let tt = t.transcript();
    let glyph_color = if row.failed {
        p.error
    } else {
        p.roles.text_tertiary
    };
    let mono = row.mono.clone();
    div()
        .id(row.id)
        .group("event-row")
        .h(px(tt.tool_row))
        .ml(px(-t.space.sm))
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
                .text_color(rgb(if row.failed { p.error } else { p.text }))
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
        .when(row.diff.0 > 0, |d| {
            d.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.success))
                    .child(format!("+{}", row.diff.0)),
            )
        })
        .when(row.diff.1 > 0, |d| {
            d.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.error))
                    .child(format!("−{}", row.diff.1)),
            )
        })
        .when_some(row.summary, |d, summary| {
            d.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(SharedString::from(format!("· {summary}"))),
            )
        })
        .child(div().flex_1())
        .map(|d| match row.end {
            EventEnd::Nothing => d,
            EventEnd::Quiet(label) => d.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(label),
            ),
            EventEnd::Badge(badge) => d.child(badge.render(&p)),
        })
        .children(row.trailing)
        // The chevron's slot is always kept, so every row's right edge
        // (durations, states) lines up down the log.
        .child(
            div()
                .w(px(t.icon.xs))
                .flex_shrink_0()
                .text_color(rgb(p.roles.text_tertiary))
                .when_some(row.disclosure, |d, open| {
                    d.when(!open, |d| {
                        d.opacity(0.).group_hover("event-row", |s| s.opacity(1.))
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(t.icon.xs)),
                    )
                }),
        )
}

/// The hairline guide that holds a group's rows under its header, from the
/// header glyph's centre down.
pub(super) fn event_guide(p: &Palette) -> Div {
    let t = p.tokens;
    let tt = t.transcript();
    div()
        .flex()
        .flex_col()
        .ml(px(tt.event_icon / 2.))
        .pl(px(tt.event_icon / 2. + t.space.md - t.stroke.hairline))
        .border_l(px(t.stroke.hairline))
        .border_color(rgb(p.border))
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
        if self.chat_width <= px(0.) {
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
        let expanded = self
            .tool_expansion
            .get(&identity)
            .copied()
            .unwrap_or(tool.is_error);
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
        event.summary = (tool.complete && !tool.is_error)
            .then(|| tool_result_meta(tool.category(), &tool.name, &tool.output))
            .flatten()
            .map(Into::into);
        event.failed = tool.is_error;
        event.disclosure = Some(expanded);
        event.end = if tool.is_error {
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
            .border(px(t.stroke.hairline))
            .border_color(rgb(p.roles.card_border))
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
        let open = summary.failed > 0
            || steps == 1
            || self.chat.open.get(&card_key).copied().unwrap_or(true);
        let earlier_key = format!("{card_key}:earlier");
        let show_earlier = self.chat.open.get(&earlier_key).copied().unwrap_or(false);
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
            // r2-transcript: "Worked 2m 14s · 18 tools · 3 files changed".
            let toggle_key = card_key.clone();
            let mut event = EventRow::new(
                format!("{card_key}-header"),
                Glyph::Named(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                }),
                work_headline(summary.running > 0, summary.duration_ms),
            );
            event.summary = Some(
                std::iter::once(plural(steps, "tool", "tools"))
                    .chain((!summary.text.is_empty()).then(|| summary.text.clone()))
                    .collect::<Vec<_>>()
                    .join(" · ")
                    .into(),
            );
            event.diff = (summary.added, summary.removed);
            event.end = if summary.running > 0 {
                EventEnd::Badge(
                    StatusBadge::new(StatusKind::Working)
                        .label(format!("{} running", summary.running)),
                )
            } else if summary.failed > 0 {
                EventEnd::Badge(
                    StatusBadge::new(StatusKind::Failed)
                        .label(format!("{} failed", summary.failed)),
                )
            } else {
                EventEnd::Nothing
            };
            card = card.child(
                event_row(event, &p)
                    .debug_selector(|| "work-card-toggle".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle_key.clone(), true, cx)
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
                        .ml(px(-t.space.sm))
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
                    self.render_step_v2(row, window, cx)
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
        } else if running {
            EventEnd::Badge(StatusBadge::new(StatusKind::Working))
        } else if let Some(badge) = (views.len() > 1).then(|| children_badge(&views)).flatten() {
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

/// A transcript card: the shared card on its surface and edge, at the
/// transcript's tighter padding.
pub(super) fn transcript_card(p: &Palette) -> Div {
    let t = p.tokens;
    primitives::card(p)
        .p(px(t.space.lg))
        .gap(px(t.space.md))
        .w_full()
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
            let card = self.render_fleet_card(&key, fleet, row.copy_text(), window, cx);
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
        event.diff = (added, removed);
        if let [file] = files {
            // One file: named on the line itself.
            event.target = Some(file.path.clone().into());
            event.mono = Some(mono.clone());
        }
        event.trailing.push(
            primitives::button(
                "turn-changes",
                "Review",
                Some(IconName::ArrowRight),
                ButtonVariant::Tertiary,
                ButtonSize::Small,
                connected,
                &p,
            )
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
    pub(super) fn render_jump_latest_v2(&self, cx: &mut Context<Self>) -> Div {
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
            .bottom(self.composer_dock_bounds.size.height)
            .h(px(sh.jump_scrim + t.layout.control_height_sm + sh.jump_gap))
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
                .shadow(primitives::elevation(&p, t.elevation.floating))
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.follow = true;
                    this.list.scroll_to(ListOffset {
                        item_ix: this.view.transcript.rows.len(),
                        offset_in_item: px(0.),
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
    /// A fleet or supervisor wake. While something in it still needs you it
    /// is a full card (icon, title, status, meta, actions per worker); once
    /// nothing does, it collapses to one line saying how it ended
    /// ("Decision made · Session creation review · Done"), opened on click.
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
        let states: Vec<(StatusKind, &'static str)> = fleet
            .entries
            .iter()
            .map(|entry| {
                let (label, _) = super::fleet_card::entry_status(
                    fleet.kind,
                    entry,
                    lookup(&entry.session_id).as_ref(),
                    p,
                );
                (fleet_entry_kind(label), label)
            })
            .collect();
        let pending = states.iter().any(|(k, _)| *k == StatusKind::NeedsYou);
        let failed = states
            .iter()
            .filter(|(k, _)| *k == StatusKind::Failed)
            .count();
        let working = states.iter().any(|(k, _)| *k == StatusKind::Working);
        let eyebrow = fleet_eyebrow(fleet.kind, pending, fleet.entries.len());
        let summary = if pending {
            StatusKind::NeedsYou
        } else if failed > 0 {
            StatusKind::Failed
        } else if working || fleet.kind == FleetKind::Progress {
            StatusKind::Working
        } else {
            StatusKind::Done
        };
        let title = self.fleet_title(&fleet);
        let single = fleet.entries.len() == 1;
        let open_key = format!("{key}-v2-open");
        let open = pending || self.chat.open.get(&open_key).copied().unwrap_or(false);
        let status_badge = |kind: StatusKind, label: &'static str| {
            StatusBadge::new(kind).label(match (kind, label) {
                (StatusKind::Done, "Finished") => "Done",
                (StatusKind::NeedsYou, _) => "Needs you",
                _ => label,
            })
        };
        let header_status = if single {
            let (kind, label) = states[0];
            status_badge(kind, label)
        } else if failed > 0 {
            StatusBadge::new(StatusKind::Failed).label(format!("{failed} failed"))
        } else {
            StatusBadge::new(summary)
        };
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
        if !open {
            // One line: state, what it was about, how it ended.
            let toggle = open_key.clone();
            return div().debug_selector(|| "fleet-card".into()).w_full().child(
                div()
                    .id(SharedString::from(format!("{key}-v2-line")))
                    .debug_selector(|| "fleet-card-header".into())
                    .group("tool-line")
                    .h(px(t.transcript().tool_row))
                    .ml(px(-t.space.sm))
                    .px(px(t.space.sm))
                    .rounded(px(t.radius.control))
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .min_w_0()
                    .cursor_pointer()
                    .hover(move |s| s.bg(primitives::tonal(&p, t.wash.hover)))
                    .child(glyph_box(Glyph::Status(summary), p.roles.text_tertiary, &p))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(t.text.body))
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(p.text))
                            .child(eyebrow),
                    )
                    .child(
                        div()
                            .debug_selector(|| "fleet-card-title".into())
                            .min_w_0()
                            .truncate()
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(names.join(", ")),
                    )
                    .child(div().flex_1())
                    .child(
                        header_status
                            .render(&p)
                            .debug_selector(|| "fleet-card-status".into()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(p.roles.text_tertiary))
                            .opacity(0.)
                            .group_hover("tool-line", |s| s.opacity(1.))
                            .child(Icon::new(IconName::ChevronRight).size(px(t.icon.xs))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(toggle.clone(), false, cx)
                    })),
            );
        }
        let eyebrow_color = summary.text_color(&p);
        let icon = match fleet.kind {
            FleetKind::Blocked | FleetKind::Escalated | FleetKind::Threshold => {
                IconName::TriangleAlert
            }
            FleetKind::Failed => IconName::CircleX,
            FleetKind::CatchUp => IconName::Inbox,
            FleetKind::Progress => IconName::Bot,
            FleetKind::Finished => IconName::CircleCheck,
        };
        let collapse = (!pending).then(|| {
            let toggle = open_key.clone();
            primitives::icon_button(
                SharedString::from(format!("{key}-v2-collapse")),
                IconName::ChevronDown,
                "Collapse",
                ButtonVariant::Tertiary,
                true,
                &p,
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_open(toggle.clone(), false, cx)),
            )
        });
        // The status sits on the right, where the one-line form keeps it.
        let mut head = CardHead::new(title);
        if !single {
            head = head.meta(names.join(", "));
        }
        let head = head
            .action(
                header_status
                    .render(&p)
                    .debug_selector(|| "fleet-card-status".into()),
            )
            .action(
                primitives::icon_button(
                    SharedString::from(format!("copy-{key}")),
                    IconName::Copy,
                    "Copy wake",
                    ButtonVariant::Tertiary,
                    true,
                    &p,
                )
                .opacity(0.)
                .group_hover("message", |s| s.opacity(1.))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                }),
            );
        let head = match collapse {
            Some(collapse) => head.action(collapse),
            None => head,
        }
        .render(&p);
        let mut card = transcript_card(&p)
            .debug_selector(|| "fleet-card".into())
            .child(
                div()
                    .debug_selector(|| "fleet-card-header".into())
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xs))
                    .child(
                        // r2-transcript: a bare glyph beside the eyebrow, no tile.
                        div()
                            .flex()
                            .items_center()
                            .gap(px(t.space.sm))
                            .text_size(px(t.text.meta))
                            .font_weight(FontWeight(t.weight.medium))
                            .text_color(rgb(eyebrow_color))
                            .child(Icon::new(icon).size(px(t.icon.sm)))
                            .child(eyebrow),
                    )
                    .child(head),
            );
        for (index, entry) in fleet.entries.iter().enumerate() {
            let live = lookup(&entry.session_id);
            let (kind, label) = states[index];
            let openable = self.fleet_openable(&owner, &entry.session_id);
            let model = live
                .as_ref()
                .map(Session::display_model)
                .unwrap_or_default();
            let id = entry.session_id.clone();
            let cwd = entry.cwd.clone().filter(|c| c != "?");
            let reply = format!("Re: session:{id} ({}) — ", entry.label);
            let id_tooltip = if live.is_some() {
                format!("session:{id}")
            } else {
                format!("session:{id} · no longer in this workspace")
            };
            let mut meta = Vec::new();
            if !model.is_empty() {
                meta.push(model);
            }
            if let Some(cwd) = &cwd {
                meta.push(chrome::project_label(cwd).to_owned());
            }
            let open_session = {
                let owner = owner.clone();
                let id = id.clone();
                primitives::icon_button(
                    SharedString::from(format!("{key}-open-{id}")),
                    IconName::ArrowRight,
                    if openable {
                        "Open session"
                    } else {
                        "Not an available worker of this conversation"
                    },
                    ButtonVariant::Tertiary,
                    openable,
                    &p,
                )
                .debug_selector(move || format!("fleet-entry-open-{index}"))
                .when(openable, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        if this.view.selected.as_deref() == Some(owner.as_str())
                            && this.fleet_openable(&owner, &id)
                        {
                            this.command(Command::Select(id.clone()), cx);
                        }
                    }))
                })
            };
            let reply_button = primitives::icon_button(
                SharedString::from(format!("{key}-reply-{}", entry.session_id)),
                IconName::Undo2,
                "Reply to this session",
                ButtonVariant::Tertiary,
                self.view.connected,
                &p,
            )
            .debug_selector(move || format!("fleet-entry-reply-{index}"))
            .on_click(cx.listener(move |this, _, window, cx| this.prefill(&reply, window, cx)));
            let mut row = div()
                .debug_selector(move || format!("fleet-entry-{index}"))
                .pt(px(t.space.md))
                .border_t(px(t.stroke.hairline))
                .border_color(rgb(p.border))
                .flex()
                .flex_col()
                .gap(px(t.space.xs))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.space.md))
                        .min_w_0()
                        .when(!single, |d| {
                            d.child(glyph_box(
                                Glyph::Named(IconName::Bot),
                                p.roles.text_tertiary,
                                &p,
                            ))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .when(!single, |d| {
                                    d.child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(t.text.body))
                                            .font_weight(FontWeight(t.weight.medium))
                                            .text_color(rgb(p.text))
                                            .child(names[index].clone()),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(t.space.sm))
                                        .min_w_0()
                                        .text_size(px(t.text.meta))
                                        .text_color(rgb(p.roles.text_tertiary))
                                        .when(!meta.is_empty(), |d| {
                                            d.child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(format!("{} ·", meta.join(" · "))),
                                            )
                                        })
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("{key}-id-{index}")))
                                                .debug_selector(move || {
                                                    format!("fleet-entry-id-{index}")
                                                })
                                                .flex_shrink_0()
                                                .font_family(super::mono_font())
                                                .child(super::fleet_card::short_id(&id))
                                                .tooltip(primitives::tooltip_view(
                                                    id_tooltip, None, &p,
                                                )),
                                        ),
                                ),
                        )
                        .when(!single, |d| {
                            d.child(
                                status_badge(kind, label)
                                    .render(&p)
                                    .debug_selector(move || format!("fleet-entry-status-{index}")),
                            )
                        })
                        .child(open_session)
                        .child(reply_button),
                );
            let indent = if single {
                0.
            } else {
                t.transcript().event_icon + t.space.md
            };
            if let Some(failed) = &entry.failed {
                row = row.child(
                    div()
                        .pl(px(indent))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.error))
                        .child(failed.clone()),
                );
            }
            if let Some(crossed) = &entry.crossed {
                row = row.child(
                    div()
                        .pl(px(indent))
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.warning))
                        .child(crossed.clone()),
                );
            }
            if let Some(note) = &entry.note {
                row = row.child(
                    div()
                        .ml(px(indent))
                        .pl(px(t.space.md))
                        .border_l(px(t.stroke.focus))
                        .border_color(rgb(p.border))
                        .text_size(px(t.text.body))
                        .text_color(rgb(p.prose))
                        .child(note.clone()),
                );
            }
            if let Some(last) = &entry.last_reply {
                let reply_key = format!("{key}-reply-{index}");
                let shown = self.chat.open.get(&reply_key).copied().unwrap_or(false);
                row = row.child(div().pl(px(indent)).child(self.disclosure_v2(
                    reply_key,
                    "Last reply",
                    shown,
                    cx,
                )));
                if shown {
                    row = row.child(
                        div()
                            .ml(px(indent))
                            .pl(px(t.space.md))
                            .border_l(px(t.stroke.focus))
                            .border_color(rgb(p.border))
                            .text_size(px(t.text.body))
                            .text_color(rgb(p.prose))
                            .child(last.clone()),
                    );
                }
            }
            card = card.child(row);
        }
        for (i, (title, report)) in fleet.reports.iter().enumerate() {
            let report_key = format!("{key}-report-{i}");
            card = card.child(if title.starts_with("Structured result") {
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
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(super::fleet_card::report_title(title)),
                    )
                    .child(self.render_raw(&report_key, report, window, cx))
                    .into_any_element()
            });
        }
        // What the agent read stays one quiet disclosure away.
        let original = format!("{key}-original");
        let expanded = self.chat.open.get(&original).copied().unwrap_or(false);
        card =
            card.child(self.disclosure_v2(original.clone(), "Why this was raised", expanded, cx));
        if expanded {
            card = card.child(self.render_raw(&original, &fleet.text, window, cx));
        }
        card
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
            let actions = view.stalled.then(|| {
                let check = open.clone();
                let stop_target = child.session_id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xs))
                    .flex_shrink_0()
                    .child(
                        primitives::button(
                            SharedString::from(format!("{child_key}-check")),
                            "Check",
                            None,
                            ButtonVariant::Secondary,
                            ButtonSize::Small,
                            enabled,
                            &p,
                        )
                        .debug_selector(|| "stalled-child-check".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            check(this, cx)
                        })),
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
            plural(total, "subagent", "subagents"),
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
                    .ml(px(-t.space.sm))
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
                        .border_l(px(t.stroke.hairline))
                        .border_color(rgb(p.border))
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
            (StatusKind::Idle, "Offline")
        );
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
        assert_eq!(work_headline(false, Some(134_000)), "Worked 2m 14s");
        assert_eq!(work_headline(true, None), "Working");
        assert_eq!(work_headline(false, Some(300)), "Worked");
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
