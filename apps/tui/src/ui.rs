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

use crate::app::{App, ChatMode, TabKind, View};
use crate::keys::{Action, Context};
use crate::theme::Theme;
use crate::types::{derive_stats, Agent, DerivedStats, Part, Role};
use serde_json::Value;
use tui_term::widget::PseudoTerminal;

pub fn render(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
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
            View::Agent { .. } => render_agent(f, body[1], app),
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
    if app.rename.is_some() {
        render_rename(f, f.area(), app);
    }
    if app.notes_view.is_some() {
        render_notes(f, f.area(), app);
    }
    if app.help {
        render_help(f, f.area(), app);
    }
}

fn render_rename(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.rename.as_ref() else { return };
    let w = area.width.saturating_sub(8).min(60).max(20);
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  name  ", Style::default().fg(t.dim)),
            Span::raw(form.input.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled(
            format!("  {}", crate::types::truncate(&form.cwd, w.saturating_sub(4) as usize)),
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
        Span::styled(" workspacer ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
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
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" agents ({}) ", app.agents.len()))
        .border_style(Style::default().fg(t.dim));

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
        let name = Line::from(vec![
            marker,
            Span::styled(app.agent_name(a), Style::default().add_modifier(Modifier::BOLD)),
        ]);
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        let meta = Line::from(Span::styled(meta_line(a, &stats), Style::default().fg(t.dim)));
        ListItem::new(vec![name, meta])
    }));

    let list = List::new(items).block(block).highlight_style(
        Style::default().bg(t.selection_bg).add_modifier(Modifier::BOLD),
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
    if stats.model.is_none() && stats.context_pct.is_none() && stats.cost.is_none() && a.tool_calls > 0 {
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
            Span::styled(badge(a.state()), Style::default().fg(state_color(t, a.state())).add_modifier(Modifier::BOLD)),
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
    lines.extend(ask_lines(t, a, area.width.saturating_sub(2)));

    let p = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
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
    let target = raw.get("tool_input").or_else(|| raw.get("input")).unwrap_or(raw);
    serde_json::to_string_pretty(target).unwrap_or_default()
}

/// The pending approval / question block, shared by the detail and chat panes.
fn ask_lines(t: &Theme, a: &Agent, width: u16) -> Vec<Line<'static>> {
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
    } else if let Some(qs) = a.questions() {
        if let Some(q) = qs.first() {
            out.push(Line::from(Span::styled(
                q.header.clone().unwrap_or_else(|| "Question".into()),
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            )));
            for piece in wrap(&q.question, w) {
                out.push(Line::raw(piece));
            }
            // Single-question, single-select → numbered options.
            if qs.len() == 1 && !q.multi_select && !q.options.is_empty() {
                for (i, o) in q.options.iter().enumerate().take(9) {
                    out.push(Line::from(vec![
                        Span::styled(format!(" {}. ", i + 1), Style::default().fg(t.accent)),
                        Span::raw(o.label.clone()),
                    ]));
                    if let Some(desc) = o.description.as_ref().filter(|d| !d.is_empty()) {
                        for piece in wrap(desc, w.saturating_sub(4)) {
                            out.push(Line::from(Span::styled(
                                format!("    {piece}"),
                                Style::default().fg(t.dim),
                            )));
                        }
                    }
                }
                out.push(Line::raw(""));
                out.push(Line::from(Span::styled(
                    "press 1-9 to answer, or i to type",
                    Style::default().fg(t.dim),
                )));
            } else {
                out.push(Line::from(Span::styled(
                    "press i to type your answer",
                    Style::default().fg(t.dim),
                )));
            }
        }
    }
    out
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
        .map(|a| ask_lines(&app.theme, a, area.width.saturating_sub(2)))
        .unwrap_or_default();
    let ask_h = if ask.is_empty() { 0 } else { (ask.len() as u16 + 2).min(area.height / 2) };
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
    let inner_w = rows[0].width.saturating_sub(2);
    let lines = transcript_lines(app, inner_w as usize);
    let viewport = rows[0].height.saturating_sub(2);
    let max_scroll = (lines.len().min(u16::MAX as usize) as u16).saturating_sub(viewport);
    if app.chat_follow {
        app.chat_scroll = max_scroll;
    } else {
        app.chat_scroll = app.chat_scroll.min(max_scroll);
    }
    let working = agent.as_ref().is_some_and(|a| a.is_busy());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(if working {
            Line::from(Span::styled(" working… ", Style::default().fg(app.theme.accent)))
        } else {
            Line::from("")
        })
        .border_style(Style::default().fg(app.theme.dim));
    let transcript = Paragraph::new(lines).block(block).scroll((app.chat_scroll, 0));
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
            Span::styled(format!(" {label} "), Style::default().fg(label_color).add_modifier(Modifier::BOLD)),
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
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))).block(block), area);
}

/// Build the fully-wrapped, styled transcript lines for the current turns.
///
/// Consecutive tool-only assistant turns are coalesced into one compact
/// "N tool calls · …" line so a workflow's long tool runs don't flood the view
/// (the terminal analogue of the desktop's grouped WorkCard). Turns that carry
/// text render in full.
fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = &app.theme;
    let w = width.max(10);
    let mut out: Vec<Line> = Vec::new();
    if app.turns.is_empty() {
        out.push(Line::from(Span::styled("no messages yet", Style::default().fg(t.dim))));
        return out;
    }
    let mut run: Vec<(String, String, Option<String>)> = Vec::new();
    for turn in &app.turns {
        let tool_only = turn.role == Role::Assistant
            && !turn.parts.is_empty()
            && turn.parts.iter().all(|p| matches!(p, Part::Tool { .. }));
        if tool_only {
            for p in &turn.parts {
                if let Part::Tool { name, summary, result } = p {
                    run.push((name.clone(), summary.clone(), result.clone()));
                }
            }
            continue;
        }
        flush_tool_run(&mut out, &mut run, t, w);

        let (label, color) = match turn.role {
            Role::User => ("▍ you", t.accent),
            Role::Assistant => ("▍ claude", t.ok),
        };
        out.push(Line::from(Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD))));
        for part in &turn.parts {
            match part {
                Part::Text(text) => {
                    for paragraph in text.split('\n') {
                        if paragraph.is_empty() {
                            out.push(Line::raw(""));
                        } else {
                            for piece in wrap(paragraph, w) {
                                out.push(Line::raw(piece));
                            }
                        }
                    }
                }
                Part::Tool { name, summary, result } => {
                    let text = if summary.is_empty() {
                        format!("⚙ {name}")
                    } else {
                        format!("⚙ {name} · {summary}")
                    };
                    for piece in wrap(&text, w) {
                        out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
                    }
                    if let Some(res) = result {
                        push_tool_result(&mut out, res, t, w);
                    }
                }
            }
        }
        out.push(Line::raw(""));
    }
    flush_tool_run(&mut out, &mut run, t, w);
    out
}

/// Render a tool's result as a dimmed, indented `↳` snippet (red when it's an
/// error). Already truncated to ~200 chars upstream; cap at a few lines.
fn push_tool_result(out: &mut Vec<Line<'static>>, res: &str, t: &Theme, w: usize) {
    let color = if res.starts_with("error: ") { t.bad } else { t.dim };
    for (i, line) in res.lines().take(3).enumerate() {
        let prefix = if i == 0 { "  ↳ " } else { "    " };
        for piece in wrap(line, w.saturating_sub(4)) {
            out.push(Line::from(Span::styled(format!("{prefix}{piece}"), Style::default().fg(color))));
        }
    }
}

/// Emit the buffered run of consecutive tool calls and clear it: a single
/// detailed `⚙ name · summary` line for one call, or a collapsed
/// `⚙ N tool calls · names` summary for several.
fn flush_tool_run(
    out: &mut Vec<Line<'static>>,
    run: &mut Vec<(String, String, Option<String>)>,
    t: &Theme,
    w: usize,
) {
    if run.is_empty() {
        return;
    }
    if run.len() == 1 {
        let (name, summary, result) = &run[0];
        let text = if summary.is_empty() {
            format!("⚙ {name}")
        } else {
            format!("⚙ {name} · {summary}")
        };
        for piece in wrap(&text, w) {
            out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
        }
        if let Some(res) = result {
            push_tool_result(out, res, t, w);
        }
    } else {
        let names: Vec<&str> = run.iter().map(|(n, _, _)| n.as_str()).collect();
        let text = format!("⚙ {} tool calls · {}", run.len(), summarize_tool_names(&names));
        for piece in wrap(&text, w) {
            out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
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
        Line::from(Span::styled(" ● attached — Ctrl-] to detach ", Style::default().fg(accent)))
    } else {
        Line::from(Span::styled(" i/enter to attach · t transcript ", Style::default().fg(dim)))
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
    let mut changed = None;
    if let Some(term) = sid.as_ref().and_then(|s| app.terms.get_mut(s)) {
        if term.resize(inner.height, inner.width) {
            changed = Some((inner.width, inner.height)); // (cols, rows)
        }
    }
    if changed.is_some() {
        app.term_resize = changed;
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
    spans.push(Span::styled("  T:term  [ ]:tabs  w:close", Style::default().fg(t.dim)));
    // Inspector: branch + changed-file count for the open agent's work tree.
    if let Some((branch, changed)) = app.chat_agent().and_then(|a| app.git_summary.get(a.cwd_str())) {
        if let Some(b) = branch {
            spans.push(Span::styled(format!("   ⎇ {b}"), Style::default().fg(t.dim)));
        }
        if *changed > 0 {
            spans.push(Span::styled(format!(" ±{changed}"), Style::default().fg(t.warn)));
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

    let total = app.agents.len();
    let waiting = app.agents.iter().filter(|a| a.is_waiting()).count();
    let busy = app.agents.iter().filter(|a| a.is_busy()).count();
    let idle = total.saturating_sub(waiting + busy);
    let cost: f64 = app
        .agents
        .iter()
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
            format!("workspacer · {total} agent{}", if total == 1 { "" } else { "s" }),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("needs you ", Style::default().fg(t.dim)),
            Span::styled(format!("{waiting}"), Style::default().fg(t.warn).add_modifier(Modifier::BOLD)),
            Span::styled("    working ", Style::default().fg(t.dim)),
            Span::styled(format!("{busy}"), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
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
            spans.push(Span::styled(format!("5h {p:.0}%"), Style::default().fg(rate_color(t, p))));
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

    // Compact roster, attention first (the agents are already sorted that way).
    for a in &app.agents {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        let mut row = vec![
            marker,
            Span::styled(format!("{:<28}", crate::types::truncate(&app.agent_name(a), 28)), Style::default()),
            Span::styled(
                format!("{:<10}", a.state()),
                Style::default().fg(state_color(t, a.state())),
            ),
        ];
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        if let Some(p) = stats.context_pct {
            row.push(Span::styled(format!(" {p:.0}%"), Style::default().fg(t.dim)));
        }
        if let Some(c) = stats.cost {
            row.push(Span::styled(format!("  ${c:.2}"), Style::default().fg(t.dim)));
        }
        lines.push(Line::from(row));
    }
    if total == 0 {
        lines.push(Line::from(Span::styled(
            "no sessions yet — press c to spawn an agent",
            Style::default().fg(t.dim),
        )));
    }

    let p = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

// ── spawn modal ───────────────────────────────────────────────────────────────

fn render_spawn_modal(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.spawn_form.as_ref() else { return };

    let w = area.width.saturating_sub(8).min(72).max(20);
    let inner_w = w.saturating_sub(2) as usize;

    let profile = app.profiles.get(form.profile_idx);
    let profile_name = profile.map(|p| p.name.as_str()).unwrap_or("Default");
    let n = app.profiles.len().max(1);
    let extra = profile
        .filter(|p| !p.extra_args.is_empty())
        .map(|p| format!("  ({})", p.extra_args.join(" ")))
        .unwrap_or_default();

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  cwd      ", Style::default().fg(t.dim)),
            Span::raw(form.cwd.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(vec![
            Span::styled("  profile  ", Style::default().fg(t.dim)),
            Span::styled("‹ ", Style::default().fg(t.accent)),
            Span::styled(profile_name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" ›", Style::default().fg(t.accent)),
            Span::styled(format!("  {}/{}", form.profile_idx + 1, n), Style::default().fg(t.dim)),
            Span::styled(extra, Style::default().fg(t.dim)),
        ]),
    ];

    // Tab-completion candidates, when the path is ambiguous.
    if !form.completions.is_empty() {
        let joined = form.completions.join("  ");
        let shown = crate::types::truncate(&joined, inner_w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {} {}", form.completions.len(), "matches:"),
            Style::default().fg(t.dim),
        )));
        lines.push(Line::from(Span::styled(format!("  {shown}"), Style::default().fg(t.accent))));
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
        "  type a path · tab complete · ↑↓ profile · enter spawn · esc cancel",
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
    let Some(p) = app.palette.as_ref() else { return };

    let w = area.width.saturating_sub(8).min(76).max(24);
    let max_rows = area.height.saturating_sub(6).min(14).max(3);
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
        lines.push(Line::from(Span::styled("no matches", Style::default().fg(t.dim))));
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
    let w = area.width.saturating_sub(6).min(64).max(24);

    let mut lines = vec![Line::from(Span::styled(
        "Keybindings — edit ~/.config/workspacer/tui.json to remap",
        Style::default().fg(t.dim),
    ))];
    lines.push(Line::raw(""));
    lines.extend(binding_lines(t, app, "global", Context::Global));
    lines.extend(binding_lines(t, app, "sidebar / dashboard", Context::List));
    lines.extend(binding_lines(t, app, "agent · terminal", Context::AgentTerminal));
    lines.extend(binding_lines(t, app, "agent · transcript", Context::AgentTranscript));
    lines.push(Line::from(vec![
        Span::styled("answer keys ", Style::default().fg(t.dim)),
        Span::styled("1-9", Style::default().fg(t.ok)),
        Span::styled("  (positional, not remappable)", Style::default().fg(t.dim)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("themes: ", Style::default().fg(t.dim)),
        Span::styled(crate::theme::BUILTINS.join(", "), Style::default().fg(t.accent)),
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

// ── notes scratchpad ──────────────────────────────────────────────────────────

fn render_notes(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(n) = app.notes_view.as_ref() else { return };

    let w = area.width.saturating_sub(6).min(76).max(24);
    let h = area.height.saturating_sub(4).min(24).max(6);
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
        .title(format!(" {mode} · {} ", crate::types::truncate(&n.cwd, w.saturating_sub(12) as usize)))
        .title_bottom(Line::from(Span::styled(bottom, Style::default().fg(t.dim))))
        .border_style(Style::default().fg(t.accent));

    let body = if n.text.is_empty() && !n.editing {
        Paragraph::new(Line::from(Span::styled(
            "empty — press i to write",
            Style::default().fg(t.dim),
        )))
    } else {
        // Show a trailing cursor while editing.
        let text = if n.editing { format!("{}▏", n.text) } else { n.text.clone() };
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
        .title(format!(" review · {branch} · {view} ({} files) ", r.files.len()))
        .border_style(Style::default().fg(t.accent));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let composing = r.commit_msg.is_some();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(if composing { 3 } else { 0 })])
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
            Span::styled("commit ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
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
            Line::from(Span::styled("git unavailable", Style::default().fg(t.bad).add_modifier(Modifier::BOLD))),
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
            let sc = if staged.is_empty() { '·' } else { staged.chars().next().unwrap() };
            let uc = if unstaged.is_empty() { '·' } else { unstaged.chars().next().unwrap() };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{sc}"), Style::default().fg(t.ok)),
                Span::styled(format!("{uc} "), Style::default().fg(t.warn)),
                Span::styled(crate::types::truncate(&file.display_path(), 30), Style::default()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(t.selection_bg).add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    state.select(Some(r.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_review_diff(f: &mut Frame, area: Rect, t: &Theme, r: &crate::app::ReviewState) {
    let path = r.selected_file().map(|file| file.path.as_str()).unwrap_or("");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", if path.is_empty() { "diff" } else { path }))
        .border_style(Style::default().fg(t.dim));

    if r.diff.trim().is_empty() {
        let msg = if r.files.is_empty() { "nothing to review" } else { "no changes in this view" };
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
            } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
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
        Paragraph::new(lines).block(block).scroll((r.diff_scroll, 0)),
        area,
    );
}

// ── footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
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
    } else if !in_agent {
        "j/k move · enter open · ^K palette · T term · c new · m attention · ? help · q quit"
    } else if app.insert_mode {
        "enter send · esc normal"
    } else if on_shell {
        "i attach · [ ] tabs · T term · w close · x/X stop · ? help · esc back"
    } else if app.chat_mode == ChatMode::Terminal {
        "i attach · t transcript · [ ] tabs · T term · w close · ? help · esc back"
    } else {
        "i type · j/k scroll · t terminal · [ ] tabs · y/n/a · 1-9 · ? help · esc back"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {hint}"), Style::default().fg(app.theme.dim)))),
        area,
    );
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

// ── text wrapping ───────────────────────────────────────────────────────────

/// Greedy word-wrap to `width` columns, hard-splitting tokens longer than the
/// line. Good enough for transcript/JSON display; avoids pulling in a crate.
fn wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        if word.chars().count() > width {
            // Flush, then hard-split the long token.
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(std::mem::take(&mut chunk));
                }
                chunk.push(ch);
            }
            cur = chunk;
            continue;
        }
        let extra = if cur.is_empty() { 0 } else { 1 };
        if cur.chars().count() + extra + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}
