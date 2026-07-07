//! ratatui rendering. One entry point, [`render`], takes `&mut App` because it
//! resolves the transcript's follow-to-bottom flag into a concrete scroll
//! offset clamped to the content height — the only state the renderer writes.
//!
//! Colors come from `app.theme` (see `theme.rs`); no widget references a literal
//! color. Leaf helpers that don't get an `&App` take an explicit `&Theme`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, ChatMode, SplitDir, TabKind, View};
use crate::keys::{Action, Context};
use crate::theme::Theme;
use crate::types::{derive_stats, Agent, DerivedStats, Part, Role};
use serde_json::Value;
use tui_term::widget::PseudoTerminal;

pub fn render(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_header(f, root[0], app);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(20)])
        .split(root[1]);

    render_sidebar(f, body[0], app);
    if app.review.is_some() {
        // The review pane takes the whole content column (it wants the width).
        render_review(f, body[1], app);
    } else {
        match &app.view {
            View::List if app.dashboard_selected() => render_dashboard(f, body[1], app),
            View::List => render_detail(f, body[1], app),
            View::Agent { .. } => render_panes(f, body[1], app),
        }
    }

    render_footer(f, root[2], app);

    // Modals float over everything when open.
    if app.spawn_form.is_some() {
        render_spawn_modal(f, f.area(), app);
    }
    if app.palette.is_some() {
        render_palette(f, f.area(), app);
    }
    if app.picker.is_some() {
        render_picker(f, f.area(), app);
    }
    if app.search.is_some() {
        render_search(f, f.area(), app);
    }
    if app.rename.is_some() {
        render_rename(f, f.area(), app);
    }
    if app.notes_view.is_some() {
        render_notes(f, f.area(), app);
    }
    if app.help {
        render_help(f, f.area(), app);
    }
    // The which-key popup floats whenever a multi-key sequence is mid-flight.
    render_whichkey(f, root[1], app);
}

fn render_rename(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.rename.as_ref() else {
        return;
    };
    let w = area.width.saturating_sub(8).clamp(20, 60);
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  name  ", Style::default().fg(t.dim)),
            Span::raw(form.input.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled(
            format!(
                "  {}",
                crate::types::truncate(&form.cwd, w.saturating_sub(4) as usize)
            ),
            Style::default().fg(t.dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  enter save · empty clears · esc cancel",
            Style::default().fg(t.dim),
        )),
    ];
    let h = lines.len() as u16 + 2;
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h.min(area.height),
    };
    f.render_widget(ratatui::widgets::Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" rename agent ")
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── header ──────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let (dot, dot_color, status) = if app.connected {
        ("●", t.ok, "connected")
    } else {
        ("●", t.bad, "reconnecting…")
    };
    let mut spans = vec![
        Span::styled(
            " workspacer ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· tui", Style::default().fg(t.dim)),
    ];
    if let Some(toast) = app.toast() {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(toast.to_string(), Style::default().fg(t.warn)));
    }
    let left = Paragraph::new(Line::from(spans));
    f.render_widget(left, area);

    let right = Paragraph::new(Line::from(vec![
        Span::styled(format!("{dot} "), Style::default().fg(dot_color)),
        Span::styled(format!("{status} "), Style::default().fg(t.dim)),
    ]))
    .right_aligned();
    f.render_widget(right, area);
}

// ── sidebar ──────────────────────────────────────────────────────────────────

fn render_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let title = if app.hidden_count > 0 {
        format!(
            " agents ({} · +{} stopped) ",
            app.agents.len(),
            app.hidden_count
        )
    } else {
        format!(" agents ({}) ", app.agents.len())
    };
    let editing = app.filter_editing;
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if editing { t.accent } else { t.dim }));
    // Show the active `/` filter along the bottom edge (with a cursor while
    // it's being typed).
    if let Some(q) = &app.filter {
        let txt = if editing {
            format!(" /{q}▏ ")
        } else {
            format!(" /{q} ")
        };
        block = block.title_bottom(Line::from(Span::styled(
            txt,
            Style::default().fg(if editing { t.accent } else { t.dim }),
        )));
    }

    // Pinned Dashboard row, then one row per agent.
    let mut items: Vec<ListItem> = Vec::with_capacity(app.agents.len() + 1);
    items.push(ListItem::new(vec![
        Line::from(vec![
            Span::styled("▣ ", Style::default().fg(t.accent)),
            Span::styled("Dashboard", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("overview", Style::default().fg(t.dim))),
    ]));
    items.extend(app.agents.iter().map(|a| {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        let mut name_spans = vec![
            marker,
            Span::styled(
                app.agent_name(a),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ];
        // Harpoon pin badge: the 1-based slot, so `<leader>N` is discoverable.
        if let Some(slot) = app.harpoon.iter().position(|s| s == &a.session_id) {
            name_spans.push(Span::styled(
                format!(" ⚓{}", slot + 1),
                Style::default().fg(t.accent),
            ));
        }
        let name = Line::from(name_spans);
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        let meta = Line::from(Span::styled(
            meta_line(a, &stats),
            Style::default().fg(t.dim),
        ));
        ListItem::new(vec![name, meta])
    }));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn meta_line(a: &Agent, stats: &DerivedStats) -> String {
    let mut s = badge(a.state());
    if let Some(m) = &stats.model {
        s.push_str(&format!("  {}", short_model(m)));
    }
    if let Some(p) = stats.context_pct {
        s.push_str(&format!("  {p:.0}% ctx"));
    }
    if let Some(c) = stats.cost {
        s.push_str(&format!("  ${c:.2}"));
    }
    // No usage/statusLine yet — fall back to a raw tool-call count.
    if stats.model.is_none()
        && stats.context_pct.is_none()
        && stats.cost.is_none()
        && a.tool_calls > 0
    {
        s.push_str(&format!("  {} tools", a.tool_calls));
    }
    s
}

/// Trim the `claude-` prefix for a compact model label (e.g. `opus-4-8`).
fn short_model(model: &str) -> &str {
    model.strip_prefix("claude-").unwrap_or(model)
}

fn badge(state: &str) -> String {
    let s = if state.is_empty() { "idle" } else { state };
    s.to_lowercase()
}

/// Map a session state to a theme role color: waiting/input → warn, error → bad,
/// everything else → ok.
fn state_color(t: &Theme, state: &str) -> Color {
    match state.to_lowercase().as_str() {
        "input" | "waiting" => t.warn,
        "error" => t.bad,
        _ => t.ok,
    }
}

/// Colour a rate-limit percentage: ok < 75% < warn < 90% < bad.
fn rate_color(t: &Theme, pct: f64) -> Color {
    if pct >= 90.0 {
        t.bad
    } else if pct >= 75.0 {
        t.warn
    } else {
        t.ok
    }
}

// ── detail (list view right pane) ─────────────────────────────────────────────

fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" details ")
        .border_style(Style::default().fg(t.dim));

    let Some(a) = app.selected_agent() else {
        let p = Paragraph::new(Line::from(Span::styled(
            "select an agent — enter to open",
            Style::default().fg(t.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    };

    let mut lines: Vec<Line> = vec![
        kv(t, "cwd", a.cwd_str()),
        Line::from(vec![
            Span::styled("state  ", Style::default().fg(t.dim)),
            Span::styled(
                badge(a.state()),
                Style::default()
                    .fg(state_color(t, a.state()))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    let stats = derive_stats(a, app.status_lines.get(&a.session_id));
    if let Some(m) = &stats.model {
        lines.push(kv(t, "model", m));
    }
    if let Some(p) = stats.context_pct {
        lines.push(kv(t, "context", &format!("{p:.0}%")));
    }
    if let Some(c) = stats.cost {
        lines.push(kv(t, "cost", &format!("${c:.2}")));
    }
    // Account-wide rate-limit windows, when Claude reports them (Pro/Max).
    if let Some(sl) = app.status_lines.get(&a.session_id) {
        if let Some(p) = sl.five_hour_pct {
            lines.push(kv(t, "5h", &format!("{p:.0}% used")));
        }
        if let Some(p) = sl.seven_day_pct {
            lines.push(kv(t, "7d", &format!("{p:.0}% used")));
        }
    }
    if a.tool_calls > 0 {
        lines.push(kv(t, "tools", &a.tool_calls.to_string()));
    }
    if let Some(ev) = a.last_event.as_deref().filter(|e| !e.is_empty()) {
        lines.push(kv(t, "event", ev));
    }
    lines.push(Line::raw(""));
    lines.extend(ask_lines(
        t,
        a,
        app.question_flow.as_ref(),
        area.width.saturating_sub(2),
    ));

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

fn kv<'a>(t: &Theme, k: &'a str, v: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{k:<7}"), Style::default().fg(t.dim)),
        Span::raw(v.to_string()),
    ])
}

/// Pretty-print the most relevant slice of a permission-request hook payload —
/// the tool input if we can find it, else the whole thing.
fn approval_input(raw: &Value) -> String {
    let target = raw
        .get("tool_input")
        .or_else(|| raw.get("input"))
        .unwrap_or(raw);
    serde_json::to_string_pretty(target).unwrap_or_default()
}

/// The pending approval / question block, shared by the detail and chat panes.
///
/// Multi-question sets render ONE question at a time with a `Q n of m`
/// progress marker, stepping via `flow` (see [`crate::app::QuestionFlow`]);
/// multi-select questions render ☐/☑ checkboxes that digits toggle and Enter
/// confirms.
fn ask_lines(
    t: &Theme,
    a: &Agent,
    flow: Option<&crate::app::QuestionFlow>,
    width: u16,
) -> Vec<Line<'static>> {
    let w = width.max(10) as usize;
    let mut out = Vec::new();
    if let Some((tool, raw)) = a.approval() {
        out.push(Line::from(Span::styled(
            format!("⚠ wants to run {tool}"),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )));
        let pretty = approval_input(raw);
        for line in pretty.lines().take(12) {
            for piece in wrap(line, w) {
                out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
            }
        }
        out.push(Line::raw(""));
        out.push(Line::from(Span::styled(
            "[y]es  [n]o  [a]lways",
            Style::default().fg(t.ok),
        )));
    } else if let Some(qs) = a.questions().filter(|q| !q.is_empty()) {
        let n = qs.len();
        // The stepper's flow only applies when it tracks this exact set
        // (same session, length, AND content — not a superseded look-alike).
        let flow = flow.filter(|f| f.tracks(&a.session_id, qs));
        let idx = flow.map(|f| f.idx.min(n - 1)).unwrap_or(0);
        let q = &qs[idx];

        let mut head = vec![Span::styled(
            q.header.clone().unwrap_or_else(|| "Question".into()),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )];
        if n > 1 {
            head.push(Span::styled(
                format!("  · Q {} of {n}", idx + 1),
                Style::default().fg(t.dim),
            ));
        }
        out.push(Line::from(head));
        for piece in wrap(&q.question, w) {
            out.push(Line::raw(piece));
        }

        // The recorded pick for a revisited question renders highlighted.
        let prev_pick = flow.and_then(|f| f.answers[idx].as_deref());
        let picks = flow.map(|f| &f.picks[idx]);
        if q.multi_select && !q.options.is_empty() {
            for (i, o) in q.options.iter().enumerate().take(9) {
                let checked = picks.is_some_and(|p| p.contains(&i));
                let (bx, style) = if checked {
                    ("☑", Style::default().fg(t.ok))
                } else {
                    ("☐", Style::default().fg(t.dim))
                };
                out.push(Line::from(vec![
                    Span::styled(format!(" {}. ", i + 1), Style::default().fg(t.accent)),
                    Span::styled(format!("{bx} "), style),
                    Span::raw(o.label.clone()),
                ]));
                push_option_desc(&mut out, t, w, o);
            }
            out.push(Line::raw(""));
            out.push(Line::from(Span::styled(
                back_hint("1-9 toggle · enter confirm", idx),
                Style::default().fg(t.dim),
            )));
        } else if !q.options.is_empty() {
            for (i, o) in q.options.iter().enumerate().take(9) {
                let chosen = prev_pick == Some((i + 1).to_string().as_str());
                let label_style = if chosen {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let marker = if chosen { "❯" } else { " " };
                out.push(Line::from(vec![
                    Span::styled(marker.to_string(), Style::default().fg(t.accent)),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(t.accent)),
                    Span::styled(o.label.clone(), label_style),
                ]));
                push_option_desc(&mut out, t, w, o);
            }
            out.push(Line::raw(""));
            out.push(Line::from(Span::styled(
                back_hint("press 1-9 to answer, or i to type", idx),
                Style::default().fg(t.dim),
            )));
        } else {
            out.push(Line::from(Span::styled(
                back_hint("press i to type your answer", idx),
                Style::default().fg(t.dim),
            )));
        }
    }
    out
}

/// A question option's dim description lines, wrapped and indented.
fn push_option_desc(
    out: &mut Vec<Line<'static>>,
    t: &Theme,
    w: usize,
    o: &crate::types::QuestionOption,
) {
    if let Some(desc) = o.description.as_ref().filter(|d| !d.is_empty()) {
        for piece in wrap(desc, w.saturating_sub(4)) {
            out.push(Line::from(Span::styled(
                format!("    {piece}"),
                Style::default().fg(t.dim),
            )));
        }
    }
}

/// Append the mid-set `esc back` hint to a question footer.
fn back_hint(base: &str, idx: usize) -> String {
    if idx > 0 {
        format!("{base} · esc back")
    } else {
        base.to_string()
    }
}

// ── chat view ────────────────────────────────────────────────────────────────

fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
    let agent = app.chat_agent().cloned();
    let title = agent
        .as_ref()
        .map(|a| format!(" {} ", app.agent_name(a)))
        .unwrap_or_else(|| " session ended ".into());

    // Reserve space for the ask block (if any) and the composer.
    let ask = agent
        .as_ref()
        .map(|a| {
            ask_lines(
                &app.theme,
                a,
                app.question_flow.as_ref(),
                area.width.saturating_sub(2),
            )
        })
        .unwrap_or_default();
    let ask_h = if ask.is_empty() {
        0
    } else {
        (ask.len() as u16 + 2).min(area.height / 2)
    };
    let composer_h = 3;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(ask_h),
            Constraint::Length(composer_h),
        ])
        .split(area);

    // Transcript — manually wrapped so the scroll offset maps to visible lines.
    // The folded + wrapped lines are memoized on the app (the main loop draws
    // on every event — PTY chunks, SSE nudges, keystrokes, the tick — and a
    // full markdown re-parse of a long conversation per draw is far too slow);
    // the cache is invalidated whenever turns/echo change, and rebuilt here
    // when the width differs.
    let inner_w = rows[0].width.saturating_sub(2) as usize;
    let stale = app
        .transcript_cache
        .as_ref()
        .is_none_or(|c| c.width != inner_w);
    if stale {
        let lines = transcript_lines(app, inner_w);
        app.transcript_cache = Some(crate::app::TranscriptCache {
            width: inner_w,
            lines,
        });
    }
    let cache = app.transcript_cache.as_ref().expect("cache just ensured");
    let total = cache.lines.len();
    let viewport = rows[0].height.saturating_sub(2) as usize;
    let max_scroll = total.saturating_sub(viewport);
    let scroll = if app.chat_follow {
        max_scroll
    } else {
        app.chat_scroll.min(max_scroll)
    };
    // Only the visible window feeds the widget: scrolling by slice keeps the
    // offset in usize (no u16 ceiling on very long transcripts) and clones a
    // viewport's worth of lines instead of the whole conversation.
    let visible: Vec<Line> = cache.lines[scroll..(scroll + viewport).min(total)].to_vec();
    app.chat_scroll = scroll;
    let working = agent.as_ref().is_some_and(|a| a.is_busy());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(if working {
            Line::from(Span::styled(
                " working… ",
                Style::default().fg(app.theme.accent),
            ))
        } else {
            Line::from("")
        })
        .border_style(Style::default().fg(app.theme.dim));
    let transcript = Paragraph::new(visible).block(block);
    f.render_widget(transcript, rows[0]);

    if ask_h > 0 {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.warn));
        f.render_widget(Paragraph::new(ask).block(block), rows[1]);
    }

    render_composer(f, rows[2], app, &agent);
}

fn render_composer(f: &mut Frame, area: Rect, app: &App, agent: &Option<Agent>) {
    let t = &app.theme;
    let (label, label_color) = if app.insert_mode {
        ("INSERT", t.accent)
    } else {
        ("NORMAL", t.dim)
    };
    let answering = app.insert_mode && agent.as_ref().is_some_and(|a| a.has_question());
    let hint = if answering { "answer" } else { "message" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(vec![
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(label_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{hint} "), Style::default().fg(t.dim)),
        ]))
        .border_style(Style::default().fg(if app.insert_mode { t.accent } else { t.dim }));
    let text = if app.insert_mode {
        format!("{}▏", app.input)
    } else if app.input.is_empty() {
        "press i to type".to_string()
    } else {
        app.input.clone()
    };
    let style = if !app.insert_mode && app.input.is_empty() {
        Style::default().fg(t.dim)
    } else {
        Style::default()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text, style))).block(block),
        area,
    );
}

/// One buffered tool call in a consecutive run (see [`flush_tool_run`]).
struct ToolRow {
    name: String,
    summary: String,
    result: Option<String>,
    /// Edit/MultiEdit `(old, new)` pairs — rendered as a compact diff and kept
    /// visible even when the run collapses to a summary line.
    edits: Vec<(String, String)>,
}

/// Build the fully-wrapped, styled transcript lines for the current turns.
///
/// Message text renders through the TUI's own markdown renderer
/// ([`crate::render::markdown_lines`]). Consecutive tool-only assistant turns
/// are coalesced into one compact "N tool calls · …" line so a workflow's long
/// tool runs don't flood the view (the terminal analogue of the desktop's
/// grouped WorkCard) — except Edit/MultiEdit rows, whose diffs stay visible
/// beneath the summary. A pending optimistic send echo renders as a trailing
/// user turn.
fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = &app.theme;
    let w = width.max(10);
    let mut out: Vec<Line> = Vec::new();
    if app.turns.is_empty() && app.pending_echo.is_none() {
        out.push(Line::from(Span::styled(
            "no messages yet",
            Style::default().fg(t.dim),
        )));
        return out;
    }
    let mut run: Vec<ToolRow> = Vec::new();
    for turn in &app.turns {
        let tool_only = turn.role == Role::Assistant
            && !turn.parts.is_empty()
            && turn.parts.iter().all(|p| matches!(p, Part::Tool { .. }));
        if tool_only {
            for p in &turn.parts {
                if let Part::Tool {
                    name,
                    summary,
                    result,
                    edits,
                } = p
                {
                    run.push(ToolRow {
                        name: name.clone(),
                        summary: summary.clone(),
                        result: result.clone(),
                        edits: edits.clone(),
                    });
                }
            }
            continue;
        }
        flush_tool_run(&mut out, &mut run, t, w);

        push_role_label(&mut out, t, turn.role);
        for part in &turn.parts {
            match part {
                Part::Text(text) => {
                    out.extend(crate::render::markdown_lines(text, t, w));
                }
                Part::Tool {
                    name,
                    summary,
                    result,
                    edits,
                } => {
                    push_tool_row(&mut out, t, w, name, summary);
                    push_edit_diff(&mut out, t, w, edits);
                    if let Some(res) = result {
                        push_tool_result(&mut out, res, t, w);
                    }
                }
            }
        }
        out.push(Line::raw(""));
    }
    flush_tool_run(&mut out, &mut run, t, w);

    // Optimistic echo: the just-sent message, until a refold carries it.
    if let Some(echo) = app.pending_echo.as_deref() {
        push_role_label(&mut out, t, Role::User);
        out.extend(crate::render::markdown_lines(echo, t, w));
        out.push(Line::from(Span::styled(
            "…sending",
            Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
        )));
        out.push(Line::raw(""));
    }
    out
}

/// The `▍ you` / `▍ claude` turn header.
fn push_role_label(out: &mut Vec<Line<'static>>, t: &Theme, role: Role) {
    let (label, color) = match role {
        Role::User => ("▍ you", t.accent),
        Role::Assistant => ("▍ claude", t.ok),
    };
    out.push(Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
}

/// The dim `⚙ name · summary` line for one tool call.
fn push_tool_row(out: &mut Vec<Line<'static>>, t: &Theme, w: usize, name: &str, summary: &str) {
    let text = if summary.is_empty() {
        format!("⚙ {name}")
    } else {
        format!("⚙ {name} · {summary}")
    };
    for piece in wrap(&text, w) {
        out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
    }
}

/// Render a tool's result as a dimmed, indented `↳` snippet (red when it's an
/// error). Already truncated to ~200 chars upstream; cap at a few lines.
fn push_tool_result(out: &mut Vec<Line<'static>>, res: &str, t: &Theme, w: usize) {
    let color = if res.starts_with("error: ") {
        t.bad
    } else {
        t.dim
    };
    for (i, line) in res.lines().take(3).enumerate() {
        let prefix = if i == 0 { "  ↳ " } else { "    " };
        for piece in wrap(line, w.saturating_sub(4)) {
            out.push(Line::from(Span::styled(
                format!("{prefix}{piece}"),
                Style::default().fg(color),
            )));
        }
    }
}

/// A compact colored diff for an Edit/MultiEdit call: `-` old lines in the
/// bad role, `+` new lines in the ok role (the review pane's convention),
/// capped with a `… +k more lines` tail.
fn push_edit_diff(out: &mut Vec<Line<'static>>, t: &Theme, w: usize, edits: &[(String, String)]) {
    const MAX_LINES: usize = 12;
    if edits.is_empty() {
        return;
    }
    let mut rows: Vec<(char, &str)> = Vec::new();
    for (old, new) in edits {
        rows.extend(old.lines().map(|l| ('-', l)));
        rows.extend(new.lines().map(|l| ('+', l)));
    }
    let total = rows.len();
    for (sign, text) in rows.into_iter().take(MAX_LINES) {
        let color = if sign == '-' { t.bad } else { t.ok };
        let line = crate::types::truncate(&format!("  {sign} {text}"), w);
        out.push(Line::from(Span::styled(line, Style::default().fg(color))));
    }
    if total > MAX_LINES {
        out.push(Line::from(Span::styled(
            format!("    … +{} more lines", total - MAX_LINES),
            Style::default().fg(t.dim),
        )));
    }
}

/// Emit the buffered run of consecutive tool calls and clear it: a single
/// detailed `⚙ name · summary` line for one call, or a collapsed
/// `⚙ N tool calls · names` summary for several. Edit/MultiEdit rows survive
/// the collapse — their rows + diffs still render beneath the summary
/// (desktop parity: edits are the part of a work run you want to see).
fn flush_tool_run(out: &mut Vec<Line<'static>>, run: &mut Vec<ToolRow>, t: &Theme, w: usize) {
    if run.is_empty() {
        return;
    }
    if run.len() == 1 {
        let row = &run[0];
        push_tool_row(out, t, w, &row.name, &row.summary);
        push_edit_diff(out, t, w, &row.edits);
        if let Some(res) = &row.result {
            push_tool_result(out, res, t, w);
        }
    } else {
        let names: Vec<&str> = run.iter().map(|r| r.name.as_str()).collect();
        let text = format!(
            "⚙ {} tool calls · {}",
            run.len(),
            summarize_tool_names(&names)
        );
        for piece in wrap(&text, w) {
            out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
        }
        // Edits stay visible under the collapsed summary.
        for row in run.iter().filter(|r| !r.edits.is_empty()) {
            push_tool_row(out, t, w, &row.name, &row.summary);
            push_edit_diff(out, t, w, &row.edits);
        }
    }
    out.push(Line::raw(""));
    run.clear();
}

/// "Read ×4, Edit ×3, Bash, Grep ×2 +2 more" — per-tool counts in first-seen
/// order, capped so the collapsed line stays short.
fn summarize_tool_names(names: &[&str]) -> String {
    use std::collections::HashMap;
    let mut order: Vec<&str> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &n in names {
        if !counts.contains_key(n) {
            order.push(n);
        }
        *counts.entry(n).or_insert(0) += 1;
    }
    const MAX: usize = 6;
    let mut parts: Vec<String> = order
        .iter()
        .take(MAX)
        .map(|&n| {
            let c = counts[n];
            if c > 1 {
                format!("{n} ×{c}")
            } else {
                n.to_string()
            }
        })
        .collect();
    if order.len() > MAX {
        parts.push(format!("+{} more", order.len() - MAX));
    }
    parts.join(", ")
}

// ── terminal view (raw PTY) ───────────────────────────────────────────────────

fn render_terminal(f: &mut Frame, area: Rect, app: &mut App) {
    let (accent, dim) = (app.theme.accent, app.theme.dim);
    let title = app
        .chat_agent()
        .map(|a| format!(" {} ", app.agent_name(a)))
        .unwrap_or_else(|| " session ended ".into());
    let border = if app.term_attached() { accent } else { dim };
    let bottom = if app.term_attached() {
        Line::from(Span::styled(
            " ● attached — Ctrl-] to detach ",
            Style::default().fg(accent),
        ))
    } else {
        Line::from(Span::styled(
            " i/enter to attach · t transcript ",
            Style::default().fg(dim),
        ))
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(bottom)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    let sid = app.open_session_id();

    // Keep the open emulator sized to the pane; remember a size change so the
    // main loop can tell claudemon to reflow the real PTY.
    if let Some(s) = sid.as_ref() {
        if let Some(term) = app.terms.get_mut(s) {
            if term.resize(inner.height, inner.width) {
                app.term_resizes
                    .insert(s.clone(), (inner.width, inner.height)); // (cols, rows)
            }
        }
    }

    match sid.as_ref().and_then(|s| app.terms.get(s)) {
        Some(term) => {
            let pty = PseudoTerminal::new(term.screen()).block(block);
            f.render_widget(pty, area);
        }
        None => {
            let p = Paragraph::new(Line::from(Span::styled(
                "starting terminal…",
                Style::default().fg(dim),
            )))
            .block(block);
            f.render_widget(p, area);
        }
    }
}

// ── window splits (tiled panes) ────────────────────────────────────────────────

/// The content area when an agent is open. With a single tile it's just the
/// agent view; with more, it tiles each agent — the focused one fully
/// interactive, the rest a live read-only terminal.
fn render_panes(f: &mut Frame, area: Rect, app: &mut App) {
    if app.tiles.len() <= 1 {
        render_agent(f, area, app);
        return;
    }
    let dir = match app.split_dir {
        SplitDir::Columns => Direction::Horizontal,
        SplitDir::Rows => Direction::Vertical,
    };
    let n = app.tiles.len() as u32;
    let cells = Layout::default()
        .direction(dir)
        .constraints((0..n).map(|_| Constraint::Ratio(1, n)).collect::<Vec<_>>())
        .split(area);
    // Clone the tile list so we can hand `render_agent` a `&mut App`.
    let tiles = app.tiles.clone();
    let focus = app.tile_focus;
    for (i, sid) in tiles.iter().enumerate() {
        if i == focus {
            render_agent(f, cells[i], app);
        } else {
            render_watch_pane(f, cells[i], app, sid);
        }
    }
}

/// A non-focused tile: the agent's live terminal, read-only, dim-bordered.
fn render_watch_pane(f: &mut Frame, area: Rect, app: &mut App, sid: &str) {
    let (dim, warn) = (app.theme.dim, app.theme.warn);
    let agent = app.all_agents.iter().find(|a| a.session_id == sid);
    let name = agent
        .map(|a| app.agent_name(a))
        .unwrap_or_else(|| "session ended".into());
    let waiting = agent.is_some_and(|a| a.is_waiting());
    // A waiting agent gets an amber marker so it still draws the eye when it's
    // not the focused pane.
    let title = Line::from(vec![
        Span::styled(if waiting { " ● " } else { " " }, Style::default().fg(warn)),
        Span::styled(format!("{name} "), Style::default().fg(dim)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " Ctrl-w w to focus ",
            Style::default().fg(dim),
        )))
        .border_style(Style::default().fg(if waiting { warn } else { dim }));
    let inner = block.inner(area);

    if let Some(term) = app.terms.get_mut(sid) {
        if term.resize(inner.height, inner.width) {
            app.term_resizes
                .insert(sid.to_string(), (inner.width, inner.height));
        }
    }
    match app.terms.get(sid) {
        Some(term) => {
            let pty = PseudoTerminal::new(term.screen()).block(block);
            f.render_widget(pty, area);
        }
        None => {
            let msg = if app.no_terminal.contains(sid) {
                "transcript only — Ctrl-w w to read"
            } else {
                "starting terminal…"
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(dim))))
                    .block(block),
                area,
            );
        }
    }
}

// ── agent view: tab bar + active pane ─────────────────────────────────────────

fn render_agent(f: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    render_tab_bar(f, rows[0], app);

    let on_shell = matches!(app.active_tab().map(|t| t.kind), Some(TabKind::Shell));
    if on_shell || app.chat_mode == ChatMode::Terminal {
        render_terminal(f, rows[1], app);
    } else {
        render_chat(f, rows[1], app);
    }
}

fn render_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(ws) = app.workspace() else { return };
    let mut spans = Vec::new();
    for (i, tab) in ws.tabs.iter().enumerate() {
        let style = if i == ws.active {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.dim)
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::styled("│", Style::default().fg(t.dim)));
    }
    spans.push(Span::styled(
        "  T:term  [ ]:tabs  w:close",
        Style::default().fg(t.dim),
    ));
    // Inspector: branch + changed-file count for the open agent's work tree.
    if let Some((branch, changed)) = app
        .chat_agent()
        .and_then(|a| app.git_summary.get(a.cwd_str()))
    {
        if let Some(b) = branch {
            spans.push(Span::styled(
                format!("   ⎇ {b}"),
                Style::default().fg(t.dim),
            ));
        }
        if *changed > 0 {
            spans.push(Span::styled(
                format!(" ±{changed}"),
                Style::default().fg(t.warn),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── dashboard ──────────────────────────────────────────────────────────────────

fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" dashboard ")
        .border_style(Style::default().fg(t.accent));

    // Fleet totals reflect the whole live set (not the `/`-filtered sidebar view),
    // but exclude TUI-spawned shells — they aren't agents.
    let fleet = || {
        app.all_agents
            .iter()
            .filter(|a| !app.is_shell_session(&a.session_id))
    };
    let total = fleet().count();
    let waiting = fleet().filter(|a| a.is_waiting()).count();
    let busy = fleet().filter(|a| a.is_busy()).count();
    let idle = total.saturating_sub(waiting + busy);
    let cost: f64 = fleet()
        .filter_map(|a| derive_stats(a, app.status_lines.get(&a.session_id)).cost)
        .sum();
    // Rate limits are account-wide (identical across sessions) — show the first
    // session that reports them.
    let rate = app
        .status_lines
        .values()
        .find(|s| s.five_hour_pct.is_some() || s.seven_day_pct.is_some());

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "workspacer · {total} agent{}",
                if total == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("needs you ", Style::default().fg(t.dim)),
            Span::styled(
                format!("{waiting}"),
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    working ", Style::default().fg(t.dim)),
            Span::styled(
                format!("{busy}"),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    idle ", Style::default().fg(t.dim)),
            Span::styled(format!("{idle}"), Style::default().fg(t.ok)),
        ]),
        Line::from(vec![
            Span::styled("total cost ", Style::default().fg(t.dim)),
            Span::styled(format!("${cost:.2}"), Style::default().fg(t.ok)),
        ]),
    ];
    if let Some(s) = rate {
        let mut spans = vec![Span::styled("rate limit ", Style::default().fg(t.dim))];
        if let Some(p) = s.five_hour_pct {
            spans.push(Span::styled(
                format!("5h {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        if let Some(p) = s.seven_day_pct {
            spans.push(Span::styled(
                format!("   7d {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));

    // Compact roster over the whole fleet (ignores the sidebar filter, skips shells).
    for a in app
        .all_agents
        .iter()
        .filter(|a| !app.is_shell_session(&a.session_id))
    {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        let mut row = vec![
            marker,
            Span::styled(
                format!("{:<28}", crate::types::truncate(&app.agent_name(a), 28)),
                Style::default(),
            ),
            Span::styled(
                format!("{:<10}", a.state()),
                Style::default().fg(state_color(t, a.state())),
            ),
        ];
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        if let Some(p) = stats.context_pct {
            row.push(Span::styled(
                format!(" {p:.0}%"),
                Style::default().fg(t.dim),
            ));
        }
        if let Some(c) = stats.cost {
            row.push(Span::styled(
                format!("  ${c:.2}"),
                Style::default().fg(t.dim),
            ));
        }
        lines.push(Line::from(row));
    }
    if total == 0 {
        lines.push(Line::from(Span::styled(
            "no sessions yet — press c to spawn an agent",
            Style::default().fg(t.dim),
        )));
    }

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

// ── spawn modal ───────────────────────────────────────────────────────────────

fn render_spawn_modal(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.spawn_form.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(8).clamp(20, 72);
    let inner_w = w.saturating_sub(2) as usize;

    let profile = app.profiles.get(form.profile_idx);
    let profile_name = profile.map(|p| p.name.as_str()).unwrap_or("Default");
    let n = app.profiles.len().max(1);
    let extra = profile
        .filter(|p| !p.extra_args.is_empty())
        .map(|p| format!("  ({})", p.extra_args.join(" ")))
        .unwrap_or_default();
    let providers = crate::app::SPAWN_PROVIDERS;
    let provider = providers
        .get(form.provider_idx)
        .copied()
        .unwrap_or("claude");
    let is_claude = provider == "claude";

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  cwd      ", Style::default().fg(t.dim)),
            Span::raw(form.cwd.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(vec![
            Span::styled("  provider ", Style::default().fg(t.dim)),
            Span::styled("‹ ", Style::default().fg(t.accent)),
            Span::styled(
                provider.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ›", Style::default().fg(t.accent)),
            Span::styled(
                format!("  {}/{}", form.provider_idx + 1, providers.len()),
                Style::default().fg(t.dim),
            ),
        ]),
    ];
    // The profile picker only applies to claude (managed providers ignore it).
    if is_claude {
        lines.push(Line::from(vec![
            Span::styled("  profile  ", Style::default().fg(t.dim)),
            Span::styled("‹ ", Style::default().fg(t.accent)),
            Span::styled(
                profile_name.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ›", Style::default().fg(t.accent)),
            Span::styled(
                format!("  {}/{}", form.profile_idx + 1, n),
                Style::default().fg(t.dim),
            ),
            Span::styled(extra, Style::default().fg(t.dim)),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  managed session — profile not used",
            Style::default().fg(t.dim),
        )));
    }

    // Tab-completion candidates, when the path is ambiguous.
    if !form.completions.is_empty() {
        let joined = form.completions.join("  ");
        let shown = crate::types::truncate(&joined, inner_w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {} {}", form.completions.len(), "matches:"),
            Style::default().fg(t.dim),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {shown}"),
            Style::default().fg(t.accent),
        )));
    }

    // When seeding a library prompt, show what will be inserted.
    if let Some(prompt) = form.initial_prompt.as_ref() {
        let first = prompt.lines().next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled("  prompt   ", Style::default().fg(t.dim)),
            Span::styled(
                crate::types::truncate(first, inner_w.saturating_sub(12)),
                Style::default().fg(t.ok),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  type a path · tab complete · ←→ provider · ↑↓ profile · enter spawn · esc",
        Style::default().fg(t.dim),
    )));

    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    // Clear underneath so the list doesn't bleed through.
    f.render_widget(ratatui::widgets::Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" new agent ")
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── command palette ─────────────────────────────────────────────────────────

fn render_palette(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(p) = app.palette.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(8).clamp(24, 76);
    let max_rows = area.height.saturating_sub(6).clamp(3, 14);
    let visible: Vec<_> = p.visible().collect();
    let shown = (visible.len() as u16).min(max_rows);
    let h = shown + 4; // search line + borders + padding
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 2,
        width: w,
        height: h.min(area.height),
    };
    f.render_widget(ratatui::widgets::Clear, rect);

    let inner_w = w.saturating_sub(2) as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("› ", Style::default().fg(t.accent)),
        Span::raw(p.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    // Scroll the list so the selection stays visible.
    let start = p.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, item) in visible.iter().skip(start).take(shown as usize).enumerate() {
        let i = start + offset;
        let selected = i == p.selected;
        let marker = if selected { "❯ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(item.label.clone(), label_style),
        ];
        if !item.hint.is_empty() {
            let room = inner_w.saturating_sub(item.label.len() + 6);
            if room > 4 {
                spans.push(Span::styled(
                    format!("  {}", crate::types::truncate(&item.hint, room)),
                    Style::default().fg(t.dim),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches",
            Style::default().fg(t.dim),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" command palette ")
        .title_bottom(Line::from(Span::styled(
            " ↑↓ move · enter run · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The model / handoff-provider picker: a query line over a small filtered
/// list. The model picker also accepts free text (a model id not in the list).
fn render_picker(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(p) = app.picker.as_ref() else { return };

    let w = area.width.saturating_sub(8).clamp(24, 64);
    let max_rows = area.height.saturating_sub(6).clamp(3, 12);
    let shown = (p.matched.len() as u16).min(max_rows);
    let h = (shown + 5).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, rect);
    let inner_w = w.saturating_sub(2) as usize;

    let mut lines = vec![Line::from(vec![
        Span::styled("› ", Style::default().fg(t.accent)),
        Span::raw(p.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    let start = p.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, &mi) in p
        .matched
        .iter()
        .skip(start)
        .take(shown as usize)
        .enumerate()
    {
        let i = start + offset;
        let selected = i == p.selected;
        let marker = if selected { "❯ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(
                crate::types::truncate(&p.items[mi].label, inner_w.saturating_sub(4)),
                label_style,
            ),
        ]));
    }
    if p.pending {
        lines.push(Line::from(Span::styled(
            "  loading models…",
            Style::default().fg(t.dim),
        )));
    } else if p.matched.is_empty() {
        let hint = if p.allow_free_text {
            "type a model id and press enter"
        } else {
            "no matches"
        };
        lines.push(Line::from(Span::styled(
            format!("  {hint}"),
            Style::default().fg(t.dim),
        )));
    }

    let foot = if p.allow_free_text {
        " ↑↓ move · enter apply (or typed id) · esc close "
    } else {
        " ↑↓ move · enter apply · esc close "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", p.title))
        .title_bottom(Line::from(Span::styled(foot, Style::default().fg(t.dim))))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The cross-agent content-search modal: a query line plus matching transcript
/// lines (agent name + snippet), with an indexing-progress note in the title.
fn render_search(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(s) = app.search.as_ref() else { return };

    let w = area.width.saturating_sub(8).clamp(36, 100);
    let max_rows = area.height.saturating_sub(6).clamp(3, 16);
    let shown = (s.matched.len() as u16).min(max_rows);
    let body_rows = if s.matched.is_empty() { 1 } else { shown };
    let h = (body_rows + 4).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, rect);

    const NAME_COL: usize = 16;
    let inner_w = w.saturating_sub(2) as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("/ ", Style::default().fg(t.accent)),
        Span::raw(s.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    let start = s.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, &idx) in s
        .matched
        .iter()
        .skip(start)
        .take(shown as usize)
        .enumerate()
    {
        let i = start + offset;
        let hit = &s.entries[idx];
        let selected = i == s.selected;
        let marker = if selected { "❯ " } else { "  " };
        let room = inner_w.saturating_sub(NAME_COL + 4);
        let snippet = crate::types::truncate(hit.line.trim(), room.max(8));
        let snippet_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(
                format!(
                    "{:<width$}",
                    crate::types::truncate(&hit.name, NAME_COL),
                    width = NAME_COL
                ),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("  {snippet}"), snippet_style),
        ]));
    }
    if s.query.is_empty() {
        lines.push(Line::from(Span::styled(
            "type to search every agent's transcript",
            Style::default().fg(t.dim),
        )));
    } else if s.matched.is_empty() {
        let msg = if s.pending > 0 {
            "indexing…"
        } else {
            "no matches"
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(t.dim))));
    }

    let title = if s.pending > 0 {
        format!(" search · indexing {} more… ", s.pending)
    } else {
        format!(" search · {} matches ", s.matched.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " ↑↓ move · enter open transcript · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── help / keybindings overlay ────────────────────────────────────────────────

/// Friendly label for an action (snake_case → spaced).
fn action_label(a: Action) -> String {
    a.name().replace('_', " ")
}

/// A two-column block of `chord  action` rows for one context, generated from
/// the live keymap so it can never drift from what the keys actually do.
fn binding_lines(t: &Theme, app: &App, title: &str, ctx: Context) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ))];
    for (chord, action) in app.keymap.bindings(ctx) {
        out.push(Line::from(vec![
            Span::styled(format!("  {chord:<10}"), Style::default().fg(t.ok)),
            Span::styled(action_label(action), Style::default().fg(t.dim)),
        ]));
    }
    out.push(Line::raw(""));
    out
}

fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let w = area.width.saturating_sub(6).clamp(24, 64);

    let mut lines = vec![Line::from(Span::styled(
        "Keybindings — edit ~/.config/workspacer/tui.json to remap",
        Style::default().fg(t.dim),
    ))];
    lines.push(Line::from(vec![
        Span::styled("press ", Style::default().fg(t.dim)),
        Span::styled(app.keymap.leader().display(), Style::default().fg(t.ok)),
        Span::styled(
            " for the leader menu (which-key)",
            Style::default().fg(t.dim),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.extend(binding_lines(t, app, "global", Context::Global));
    lines.extend(binding_lines(t, app, "sidebar / dashboard", Context::List));
    lines.extend(binding_lines(
        t,
        app,
        "agent · terminal",
        Context::AgentTerminal,
    ));
    lines.extend(binding_lines(
        t,
        app,
        "agent · transcript",
        Context::AgentTranscript,
    ));
    lines.push(Line::from(vec![
        Span::styled("answer keys ", Style::default().fg(t.dim)),
        Span::styled("1-9", Style::default().fg(t.ok)),
        Span::styled("  (positional, not remappable)", Style::default().fg(t.dim)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("themes: ", Style::default().fg(t.dim)),
        Span::styled(
            crate::theme::BUILTINS.join(", "),
            Style::default().fg(t.accent),
        ),
    ]));

    // Cap height to the viewport; the box scrolls via Paragraph if it overflows.
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" help ")
        .title_bottom(Line::from(Span::styled(
            " any key to close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The which-key popup: when a multi-key sequence is in flight (e.g. after the
/// leader), float a box listing the chords that can come next and what they do.
/// Renders nothing when no sequence is pending or the prefix is a dead end.
fn render_whichkey(f: &mut Frame, area: Rect, app: &App) {
    if app.pending_keys.is_empty() {
        return;
    }
    let t = &app.theme;
    let ctxs = [Context::Global, app.key_context()];
    let conts = app.keymap.continuations(&ctxs, &app.pending_keys);
    if conts.is_empty() {
        return;
    }

    let mut rows: Vec<Line> = conts
        .iter()
        .map(|c| {
            let key = c.chord.display();
            let label = match c.action {
                Some(a) => action_label(a),
                None => "▸ …".to_string(),
            };
            Line::from(vec![
                Span::styled(
                    format!(" {key:<7}"),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(label, Style::default().fg(t.fg)),
            ])
        })
        .collect();
    // The positional harpoon jumps (`<leader>1..9`) aren't in the keymap, so
    // surface them as a hint when the leader prefix is up and pins exist.
    if app.pending_keys == [app.keymap.leader()] && !app.harpoon.is_empty() {
        rows.push(Line::from(vec![
            Span::styled(
                format!(" {:<7}", "1-9"),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("jump to pinned agent (⚓1-{})", app.harpoon.len()),
                Style::default().fg(t.fg),
            ),
        ]));
    }

    let prefix = crate::keys::display_seq(&app.pending_keys);
    let title = format!(" {prefix}… ");
    let inner_w = rows
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(title.chars().count()) as u16;
    let w = (inner_w + 2).clamp(16, area.width.saturating_sub(2));
    let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(1));
    // Bottom-anchored, like which-key.nvim — out of the way of the content.
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + area.height.saturating_sub(h),
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " esc cancel ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(rows).block(block), rect);
}

// ── notes scratchpad ──────────────────────────────────────────────────────────

fn render_notes(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(n) = app.notes_view.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(6).clamp(24, 76);
    let h = area.height.saturating_sub(4).clamp(6, 24);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, rect);

    let mode = if n.editing { "editing" } else { "notes" };
    let bottom = if n.editing {
        " esc save · enter newline "
    } else {
        " i edit · j/k scroll · esc close "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {mode} · {} ",
            crate::types::truncate(&n.cwd, w.saturating_sub(12) as usize)
        ))
        .title_bottom(Line::from(Span::styled(bottom, Style::default().fg(t.dim))))
        .border_style(Style::default().fg(t.accent));

    let body = if n.text.is_empty() && !n.editing {
        Paragraph::new(Line::from(Span::styled(
            "empty — press i to write",
            Style::default().fg(t.dim),
        )))
    } else {
        // Show a trailing cursor while editing.
        let text = if n.editing {
            format!("{}▏", n.text)
        } else {
            n.text.clone()
        };
        Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((n.scroll, 0))
    };
    f.render_widget(body.block(block), rect);
}

// ── git review pane ───────────────────────────────────────────────────────────

fn render_review(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(r) = app.review.as_ref() else { return };

    let branch = r.branch.as_deref().unwrap_or("(detached)");
    let view = if r.staged_view { "staged" } else { "unstaged" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " review · {branch} · {view} ({} files) ",
            r.files.len()
        ))
        .border_style(Style::default().fg(t.accent));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let composing = r.commit_msg.is_some();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if composing { 3 } else { 0 }),
        ])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(20)])
        .split(rows[0]);

    render_review_files(f, cols[0], t, r);
    render_review_diff(f, cols[1], t, r);

    if composing {
        let msg = r.commit_msg.as_deref().unwrap_or("");
        let p = Paragraph::new(Line::from(vec![
            Span::styled(
                "commit ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{msg}▏")),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" message · enter commit · esc cancel ")
                .border_style(Style::default().fg(t.accent)),
        );
        f.render_widget(p, rows[1]);
    }
}

fn render_review_files(f: &mut Frame, area: Rect, t: &Theme, r: &crate::app::ReviewState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" files ")
        .border_style(Style::default().fg(t.dim));
    if let Some(err) = &r.error {
        let p = Paragraph::new(vec![
            Line::from(Span::styled(
                "git unavailable",
                Style::default().fg(t.bad).add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled(err.clone(), Style::default().fg(t.dim))),
        ])
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(block);
        f.render_widget(p, area);
        return;
    }
    if r.files.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "working tree clean",
            Style::default().fg(t.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = r
        .files
        .iter()
        .map(|file| {
            let staged = file.staged.trim();
            let unstaged = file.unstaged.trim();
            let sc = if staged.is_empty() {
                '·'
            } else {
                staged.chars().next().unwrap()
            };
            let uc = if unstaged.is_empty() {
                '·'
            } else {
                unstaged.chars().next().unwrap()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{sc}"), Style::default().fg(t.ok)),
                Span::styled(format!("{uc} "), Style::default().fg(t.warn)),
                Span::styled(
                    crate::types::truncate(&file.display_path(), 30),
                    Style::default(),
                ),
            ]))
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(r.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_review_diff(f: &mut Frame, area: Rect, t: &Theme, r: &crate::app::ReviewState) {
    let path = r
        .selected_file()
        .map(|file| file.path.as_str())
        .unwrap_or("");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", if path.is_empty() { "diff" } else { path }))
        .border_style(Style::default().fg(t.dim));

    if r.diff.trim().is_empty() {
        let msg = if r.files.is_empty() {
            "nothing to review"
        } else {
            "no changes in this view"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(t.dim)))).block(block),
            area,
        );
        return;
    }

    let lines: Vec<Line> = r
        .diff
        .lines()
        .map(|line| {
            let style = if line.starts_with("@@") {
                Style::default().fg(t.accent)
            } else if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("diff ")
                || line.starts_with("index ")
            {
                Style::default().fg(t.dim)
            } else if line.starts_with('+') {
                Style::default().fg(t.ok)
            } else if line.starts_with('-') {
                Style::default().fg(t.bad)
            } else {
                Style::default()
            };
            Line::from(Span::styled(line.to_string(), style))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .scroll((r.diff_scroll, 0)),
        area,
    );
}

// ── footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    // The `:` command line takes over the footer while it's open.
    if let Some(cmd) = &app.cmdline {
        let chip = Span::styled(
            " CMD ",
            Style::default()
                .bg(app.theme.accent)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                chip,
                Span::styled(format!(" :{cmd}▏"), Style::default().fg(app.theme.fg)),
            ])),
            area,
        );
        return;
    }
    let in_agent = matches!(app.view, View::Agent { .. });
    let on_shell = matches!(app.active_tab().map(|t| t.kind), Some(TabKind::Shell));
    let hint = if app.notes_view.as_ref().is_some_and(|n| n.editing) {
        "type notes · enter newline · esc save"
    } else if app.notes_view.is_some() {
        "i edit · j/k scroll · esc close"
    } else if app.rename.is_some() {
        "type a name · enter save · esc cancel"
    } else if app.review.as_ref().is_some_and(|r| r.commit_msg.is_some()) {
        "type message · enter commit · esc cancel"
    } else if app.review.is_some() {
        "j/k file · J/K scroll · t staged · s stage · u unstage · a all · c commit · P push · esc back"
    } else if app.spawn_form.is_some() {
        "type path · tab complete · ↑↓ profile · enter spawn · esc cancel"
    } else if app.term_attached() {
        "● attached — keys go to Claude · Ctrl-] to detach"
    } else if app.filter_editing {
        "type to filter · enter keep · esc clear"
    } else if !in_agent {
        "j/k move · / filter · enter open · ^K palette · c new · m attention · ? help · q quit"
    } else if app.insert_mode {
        "enter send · esc normal"
    } else if on_shell {
        "i attach · [ ] tabs · T term · ^w split · w close · x/X stop · esc back"
    } else if app.chat_mode == ChatMode::Terminal {
        "i attach · t transcript · [ ] tabs · ^w split · w close · ? help · esc back"
    } else {
        "i type · j/k scroll · t terminal · ^w split · y/n/a · 1-9 · ? help · esc back"
    };
    // In any normal/navigation mode (not a text field or raw terminal), point at
    // the leader menu so it's discoverable.
    let in_text = app.notes_view.is_some()
        || app.rename.is_some()
        || app.review.is_some()
        || app.spawn_form.is_some()
        || app.term_attached()
        || app.filter_editing
        || (in_agent && app.insert_mode);
    let body = if in_text {
        format!(" {hint} ")
    } else {
        format!(" {hint} · {} menu", app.keymap.leader().display())
    };
    // lualine-style mode chip on the left, then the contextual hint.
    let (label, color) = mode_chip(app, in_agent, on_shell);
    let mut spans = vec![Span::styled(
        format!(" {label} "),
        Style::default()
            .bg(color)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    )];
    // Pending vim count (e.g. while typing `12` before `G`).
    if let Some(n) = app.count {
        spans.push(Span::styled(
            format!(" {n}"),
            Style::default()
                .fg(app.theme.warn)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(body, Style::default().fg(app.theme.dim)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The current editing/navigation mode, as a (label, colour) chip for the
/// footer — so the modal state is never ambiguous.
fn mode_chip(app: &App, in_agent: bool, on_shell: bool) -> (&'static str, Color) {
    let t = &app.theme;
    if app.notes_view.as_ref().is_some_and(|n| n.editing) {
        ("NOTES", t.ok)
    } else if app.rename.is_some() {
        ("RENAME", t.accent)
    } else if app.review.as_ref().is_some_and(|r| r.commit_msg.is_some()) {
        ("COMMIT", t.ok)
    } else if app.review.is_some() {
        ("REVIEW", t.accent)
    } else if app.spawn_form.is_some() {
        ("SPAWN", t.accent)
    } else if app.term_attached() {
        ("TERM", t.bad)
    } else if app.filter_editing {
        ("FILTER", t.accent)
    } else if in_agent && app.insert_mode {
        ("INSERT", t.ok)
    } else if in_agent && on_shell {
        ("SHELL", t.warn)
    } else {
        ("NORMAL", t.accent)
    }
}

// ── text wrapping ───────────────────────────────────────────────────────────

/// Greedy word-wrap to `width` display columns, hard-splitting tokens longer
/// than the line. Display-width-aware (wide glyphs count 2) — see
/// [`crate::render::wrap`].
fn wrap(s: &str, width: usize) -> Vec<String> {
    crate::render::wrap_plain(s, width)
}

// ── state_color characterization tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // state_color() maps a state string to a theme role. These pin the mapping
    // (input/waiting → warn, error → bad, else → ok) against the default theme,
    // so a future palette change can't silently change which *role* a state uses.

    fn sc(s: &str) -> Color {
        state_color(&Theme::default(), s)
    }
    fn ok() -> Color {
        Theme::default().ok
    }
    fn warn() -> Color {
        Theme::default().warn
    }
    fn bad() -> Color {
        Theme::default().bad
    }

    // ── modes that claudemon actually emits ──────────────────────────────────

    /// "input" — user's turn; renders as warn (amber), same as "waiting".
    #[test]
    fn state_color_input_is_warn() {
        assert_eq!(sc("input"), warn());
    }

    /// "approval" — tool approval pending; not in an explicit arm, falls to ok.
    #[test]
    fn state_color_approval_is_ok() {
        assert_eq!(sc("approval"), ok());
    }

    /// "question" — structured question; falls to ok.
    #[test]
    fn state_color_question_is_ok() {
        assert_eq!(sc("question"), ok());
    }

    /// "responding" — generating a turn; falls to ok.
    #[test]
    fn state_color_responding_is_ok() {
        assert_eq!(sc("responding"), ok());
    }

    /// "stopped" — session ended; falls to ok.
    #[test]
    fn state_color_stopped_is_ok() {
        assert_eq!(sc("stopped"), ok());
    }

    /// "unknown" — emitted by Agent::state() when mode is absent; falls to ok.
    #[test]
    fn state_color_unknown_is_ok() {
        assert_eq!(sc("unknown"), ok());
    }

    /// "other" — emitted by Agent::state() for any unrecognised AgentMode;
    /// falls to ok (catch-all).
    #[test]
    fn state_color_other_is_ok() {
        assert_eq!(sc("other"), ok());
    }

    // ── aliased and legacy strings ───────────────────────────────────────────

    /// "waiting" — alias for "input" in state_color(); also yields warn.
    #[test]
    fn state_color_waiting_alias_is_warn() {
        assert_eq!(sc("waiting"), warn());
    }

    /// "thinking", "running", "streaming" — daemon never emits these; they fall
    /// to the ok catch-all.
    #[test]
    fn state_color_removed_dead_branches_fall_to_ok() {
        assert_eq!(sc("thinking"), ok());
        assert_eq!(sc("running"), ok());
        assert_eq!(sc("streaming"), ok());
    }

    /// "error" — explicit bad arm is still present.
    #[test]
    fn state_color_error_is_bad() {
        assert_eq!(sc("error"), bad());
    }

    /// state_color() normalises to lowercase before matching.
    #[test]
    fn state_color_case_insensitive() {
        assert_eq!(sc("INPUT"), warn(), "uppercase INPUT must also be warn");
        assert_eq!(sc("Error"), bad(), "mixed-case Error must also be bad");
        assert_eq!(sc("STOPPED"), ok(), "uppercase STOPPED must also be ok");
    }

    #[test]
    fn summarize_tool_names_counts_in_first_seen_order() {
        let names = ["Read", "Read", "Bash", "Read", "Edit"];
        assert_eq!(summarize_tool_names(&names), "Read ×3, Bash, Edit");
    }

    #[test]
    fn summarize_tool_names_caps_with_more() {
        let names = ["A", "B", "C", "D", "E", "F", "G", "H"];
        // 6 shown + "+2 more".
        assert_eq!(summarize_tool_names(&names), "A, B, C, D, E, F, +2 more");
    }

    #[test]
    fn summarize_tool_names_single() {
        assert_eq!(summarize_tool_names(&["Grep"]), "Grep");
    }

    // ── transcript rendering: diffs, collapsed runs, echo, questions ─────────

    fn line_texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn edit_diff_renders_minus_bad_plus_ok() {
        let t = Theme::default();
        let mut out = Vec::new();
        push_edit_diff(
            &mut out,
            &t,
            60,
            &[("old line".to_string(), "new line".to_string())],
        );
        let texts = line_texts(&out);
        assert_eq!(texts, vec!["  - old line", "  + new line"]);
        assert_eq!(out[0].spans[0].style.fg, Some(t.bad), "- lines in bad");
        assert_eq!(out[1].spans[0].style.fg, Some(t.ok), "+ lines in ok");
    }

    #[test]
    fn edit_diff_caps_with_a_more_lines_tail() {
        let t = Theme::default();
        let old = (1..=10)
            .map(|i| format!("o{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (1..=10)
            .map(|i| format!("n{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = Vec::new();
        push_edit_diff(&mut out, &t, 60, &[(old, new)]);
        assert_eq!(out.len(), 13, "12 diff lines + the tail");
        let texts = line_texts(&out);
        assert_eq!(texts[12], "    … +8 more lines");
        assert_eq!(out[12].spans[0].style.fg, Some(t.dim));
    }

    #[test]
    fn collapsed_tool_run_keeps_edit_diffs_visible() {
        let t = Theme::default();
        let row = |name: &str, edits: Vec<(String, String)>| ToolRow {
            name: name.into(),
            summary: if edits.is_empty() {
                String::new()
            } else {
                "/a.rs".into()
            },
            result: None,
            edits,
        };
        let mut out = Vec::new();
        let mut run = vec![
            row("Read", Vec::new()),
            row("Edit", vec![("foo".into(), "bar".into())]),
            row("Bash", Vec::new()),
        ];
        flush_tool_run(&mut out, &mut run, &t, 60);
        let texts = line_texts(&out);
        assert!(
            texts[0].starts_with("⚙ 3 tool calls ·"),
            "run collapses: {:?}",
            texts[0]
        );
        assert!(
            texts.iter().any(|l| l.contains("Edit · /a.rs")),
            "the edit row still renders beneath the summary: {texts:?}"
        );
        assert!(texts.iter().any(|l| l == "  - foo"));
        assert!(texts.iter().any(|l| l == "  + bar"));
    }

    #[test]
    fn ask_lines_renders_the_stepper_and_multiselect_checkboxes() {
        let t = Theme::default();
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Pick one", "options": [{"label": "A"}, {"label": "B"}]},
                {"question": "Choose", "multi_select": true,
                 "options": [{"label": "X"}, {"label": "Y"}]}
            ]}
        }))
        .unwrap();

        // Before any interaction: Q1 renders with its progress marker.
        let texts = line_texts(&ask_lines(&t, &a, None, 60));
        assert!(texts[0].contains("Q 1 of 2"), "got {:?}", texts[0]);
        assert!(texts.iter().any(|l| l.contains("1. A")));
        assert!(
            !texts.iter().any(|l| l.contains("esc back")),
            "no back hint on the first question"
        );

        // Mid-set on the multi-select: checkboxes reflect the toggles.
        let mut flow = crate::app::QuestionFlow::new("s1".into(), a.questions().unwrap());
        flow.idx = 1;
        flow.answers[0] = Some("2".into());
        flow.picks[1].insert(0);
        let texts = line_texts(&ask_lines(&t, &a, Some(&flow), 60));
        assert!(texts[0].contains("Q 2 of 2"), "got {:?}", texts[0]);
        assert!(texts.iter().any(|l| l.contains("☑ X")), "{texts:?}");
        assert!(texts.iter().any(|l| l.contains("☐ Y")), "{texts:?}");
        assert!(texts
            .iter()
            .any(|l| l.contains("enter confirm") && l.contains("esc back")));

        // Revisiting Q1: the recorded pick renders highlighted.
        flow.idx = 0;
        let lines = ask_lines(&t, &a, Some(&flow), 60);
        let texts = line_texts(&lines);
        let b_row = texts.iter().position(|l| l.contains("2. B")).unwrap();
        assert!(texts[b_row].starts_with('❯'), "got {:?}", texts[b_row]);
        let b_label = lines[b_row]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "B")
            .expect("label span");
        assert!(b_label.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn transcript_renders_a_pending_echo_as_a_user_turn() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            crate::claudemon::Claudemon::new("http://127.0.0.1:59999".into()),
            Vec::new(),
            Vec::new(),
            crate::config::Config::default(),
            tx,
            ptx,
        );
        app.pending_echo = Some("on my way".into());
        let texts = line_texts(&transcript_lines(&app, 40));
        assert!(texts.iter().any(|l| l == "▍ you"), "{texts:?}");
        assert!(texts.iter().any(|l| l == "on my way"));
        assert!(texts.iter().any(|l| l == "…sending"));
    }

    /// Exhaustive table for all known inputs.
    #[test]
    fn state_color_table() {
        let cases: &[(&str, Color)] = &[
            ("input", warn()),
            ("approval", ok()),
            ("question", ok()),
            ("responding", ok()),
            ("stopped", ok()),
            ("unknown", ok()),
            ("other", ok()),
            ("waiting", warn()),
            ("thinking", ok()),
            ("running", ok()),
            ("streaming", ok()),
            ("error", bad()),
            ("", ok()),
            ("anything", ok()),
        ];
        for (state, want) in cases {
            assert_eq!(sc(state), *want, "state_color({state:?}) expected {want:?}");
        }
    }
}
