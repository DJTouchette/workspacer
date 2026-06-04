//! ratatui rendering. One entry point, [`render`], takes `&mut App` because it
//! resolves the transcript's follow-to-bottom flag into a concrete scroll
//! offset clamped to the content height — the only state the renderer writes.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, ChatMode, TabKind, View};
use crate::types::{Agent, Part, Role};
use serde_json::Value;
use tui_term::widget::PseudoTerminal;

// Palette roughly matching the /remote client.
const ACCENT: Color = Color::Rgb(110, 168, 254);
const OK: Color = Color::Rgb(78, 201, 168);
const WARN: Color = Color::Rgb(224, 179, 65);
const BAD: Color = Color::Rgb(224, 108, 117);
const DIM: Color = Color::Rgb(139, 145, 156);

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
    match &app.view {
        View::List if app.dashboard_selected() => render_dashboard(f, body[1], app),
        View::List => render_detail(f, body[1], app),
        View::Agent { .. } => render_agent(f, body[1], app),
    }

    render_footer(f, root[2], app);

    // Modals float over everything when open.
    if app.spawn_form.is_some() {
        render_spawn_modal(f, f.area(), app);
    }
    if app.palette.is_some() {
        render_palette(f, f.area(), app);
    }
}

// ── header ──────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let (dot, dot_color, status) = if app.connected {
        ("●", OK, "connected")
    } else {
        ("●", BAD, "reconnecting…")
    };
    let mut spans = vec![
        Span::styled(" workspacer ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("· tui", Style::default().fg(DIM)),
    ];
    if let Some(toast) = app.toast() {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(toast.to_string(), Style::default().fg(WARN)));
    }
    let left = Paragraph::new(Line::from(spans));
    f.render_widget(left, area);

    let right = Paragraph::new(Line::from(vec![
        Span::styled(format!("{dot} "), Style::default().fg(dot_color)),
        Span::styled(format!("{status} "), Style::default().fg(DIM)),
    ]))
    .right_aligned();
    f.render_widget(right, area);
}

// ── sidebar ──────────────────────────────────────────────────────────────────

fn render_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" agents ({}) ", app.agents.len()))
        .border_style(Style::default().fg(DIM));

    // Pinned Dashboard row, then one row per agent.
    let mut items: Vec<ListItem> = Vec::with_capacity(app.agents.len() + 1);
    items.push(ListItem::new(vec![
        Line::from(vec![
            Span::styled("▣ ", Style::default().fg(ACCENT)),
            Span::styled("Dashboard", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("overview", Style::default().fg(DIM))),
    ]));
    items.extend(app.agents.iter().map(|a| {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(WARN))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(ACCENT))
        } else {
            Span::styled("· ", Style::default().fg(DIM))
        };
        let name = Line::from(vec![
            marker,
            Span::styled(a.short_cwd(), Style::default().add_modifier(Modifier::BOLD)),
        ]);
        let meta = Line::from(Span::styled(meta_line(a), Style::default().fg(DIM)));
        ListItem::new(vec![name, meta])
    }));

    let list = List::new(items).block(block).highlight_style(
        Style::default().bg(Color::Rgb(29, 32, 38)).add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn meta_line(a: &Agent) -> String {
    let mut s = badge(a.state());
    if let Some(u) = &a.usage {
        if let Some(m) = &u.model {
            s.push_str(&format!("  {}", short_model(m)));
        }
        if u.context_limit > 0 && u.context_tokens > 0 {
            let pct = (u.context_tokens as f64 / u.context_limit as f64 * 100.0).round();
            s.push_str(&format!("  {pct:.0}% ctx"));
        }
        if u.cost_usd > 0.0 {
            s.push_str(&format!("  ${:.2}", u.cost_usd));
        }
    } else if a.tool_calls > 0 {
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

fn state_color(state: &str) -> Color {
    match state.to_lowercase().as_str() {
        "input" | "waiting" => WARN,
        "thinking" | "running" | "streaming" => ACCENT,
        "error" => BAD,
        _ => OK,
    }
}

// ── detail (list view right pane) ─────────────────────────────────────────────

fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" details ")
        .border_style(Style::default().fg(DIM));

    let Some(a) = app.selected_agent() else {
        let p = Paragraph::new(Line::from(Span::styled(
            "select an agent — enter to open",
            Style::default().fg(DIM),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    };

    let mut lines: Vec<Line> = vec![
        kv("cwd", a.cwd_str()),
        Line::from(vec![
            Span::styled("state  ", Style::default().fg(DIM)),
            Span::styled(badge(a.state()), Style::default().fg(state_color(a.state())).add_modifier(Modifier::BOLD)),
        ]),
    ];
    if let Some(u) = &a.usage {
        if let Some(m) = &u.model {
            lines.push(kv("model", m));
        }
        if u.context_limit > 0 && u.context_tokens > 0 {
            let pct = (u.context_tokens as f64 / u.context_limit as f64 * 100.0).round();
            lines.push(kv(
                "context",
                &format!("{} / {} ({pct:.0}%)", u.context_tokens, u.context_limit),
            ));
        }
        if u.cost_usd > 0.0 {
            lines.push(kv("cost", &format!("${:.2}", u.cost_usd)));
        }
    }
    if a.tool_calls > 0 {
        lines.push(kv("tools", &a.tool_calls.to_string()));
    }
    if let Some(ev) = a.last_event.as_deref().filter(|e| !e.is_empty()) {
        lines.push(kv("event", ev));
    }
    lines.push(Line::raw(""));
    lines.extend(ask_lines(a, area.width.saturating_sub(2)));

    let p = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

fn kv<'a>(k: &'a str, v: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{k:<7}"), Style::default().fg(DIM)),
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
fn ask_lines(a: &Agent, width: u16) -> Vec<Line<'static>> {
    let w = width.max(10) as usize;
    let mut out = Vec::new();
    if let Some((tool, raw)) = a.approval() {
        out.push(Line::from(Span::styled(
            format!("⚠ wants to run {tool}"),
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        )));
        let pretty = approval_input(raw);
        for line in pretty.lines().take(12) {
            for piece in wrap(line, w) {
                out.push(Line::from(Span::styled(piece, Style::default().fg(DIM))));
            }
        }
        out.push(Line::raw(""));
        out.push(Line::from(Span::styled(
            "[y]es  [n]o  [a]lways",
            Style::default().fg(OK),
        )));
    } else if let Some(qs) = a.questions() {
        if let Some(q) = qs.first() {
            out.push(Line::from(Span::styled(
                q.header.clone().unwrap_or_else(|| "Question".into()),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            )));
            for piece in wrap(&q.question, w) {
                out.push(Line::raw(piece));
            }
            // Single-question, single-select → numbered options.
            if qs.len() == 1 && !q.multi_select && !q.options.is_empty() {
                for (i, o) in q.options.iter().enumerate().take(9) {
                    out.push(Line::from(vec![
                        Span::styled(format!(" {}. ", i + 1), Style::default().fg(ACCENT)),
                        Span::raw(o.label.clone()),
                    ]));
                    if let Some(desc) = o.description.as_ref().filter(|d| !d.is_empty()) {
                        for piece in wrap(desc, w.saturating_sub(4)) {
                            out.push(Line::from(Span::styled(
                                format!("    {piece}"),
                                Style::default().fg(DIM),
                            )));
                        }
                    }
                }
                out.push(Line::raw(""));
                out.push(Line::from(Span::styled(
                    "press 1-9 to answer, or i to type",
                    Style::default().fg(DIM),
                )));
            } else {
                out.push(Line::from(Span::styled(
                    "press i to type your answer",
                    Style::default().fg(DIM),
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
        .map(|a| format!(" {} ", a.short_cwd()))
        .unwrap_or_else(|| " session ended ".into());

    // Reserve space for the ask block (if any) and the composer.
    let ask = agent.as_ref().map(|a| ask_lines(a, area.width.saturating_sub(2))).unwrap_or_default();
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
    let max_scroll = (lines.len() as u16).saturating_sub(viewport);
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
            Line::from(Span::styled(" working… ", Style::default().fg(ACCENT)))
        } else {
            Line::from("")
        })
        .border_style(Style::default().fg(DIM));
    let transcript = Paragraph::new(lines).block(block).scroll((app.chat_scroll, 0));
    f.render_widget(transcript, rows[0]);

    if ask_h > 0 {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(WARN));
        f.render_widget(Paragraph::new(ask).block(block), rows[1]);
    }

    render_composer(f, rows[2], app, &agent);
}

fn render_composer(f: &mut Frame, area: Rect, app: &App, agent: &Option<Agent>) {
    let (label, label_color) = if app.insert_mode {
        ("INSERT", ACCENT)
    } else {
        ("NORMAL", DIM)
    };
    let answering = app.insert_mode && agent.as_ref().is_some_and(|a| a.has_question());
    let hint = if answering { "answer" } else { "message" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(vec![
            Span::styled(format!(" {label} "), Style::default().fg(label_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{hint} "), Style::default().fg(DIM)),
        ]))
        .border_style(Style::default().fg(if app.insert_mode { ACCENT } else { DIM }));
    let text = if app.insert_mode {
        format!("{}▏", app.input)
    } else if app.input.is_empty() {
        "press i to type".to_string()
    } else {
        app.input.clone()
    };
    let style = if !app.insert_mode && app.input.is_empty() {
        Style::default().fg(DIM)
    } else {
        Style::default()
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))).block(block), area);
}

/// Build the fully-wrapped, styled transcript lines for the current turns.
fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let w = width.max(10);
    let mut out: Vec<Line> = Vec::new();
    if app.turns.is_empty() {
        out.push(Line::from(Span::styled("no messages yet", Style::default().fg(DIM))));
        return out;
    }
    for turn in &app.turns {
        let (label, color) = match turn.role {
            Role::User => ("▍ you", ACCENT),
            Role::Assistant => ("▍ claude", OK),
        };
        out.push(Line::from(Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD))));
        for part in &turn.parts {
            match part {
                Part::Text(t) => {
                    for paragraph in t.split('\n') {
                        if paragraph.is_empty() {
                            out.push(Line::raw(""));
                        } else {
                            for piece in wrap(paragraph, w) {
                                out.push(Line::raw(piece));
                            }
                        }
                    }
                }
                Part::Tool { name, summary } => {
                    let text = if summary.is_empty() {
                        format!("⚙ {name}")
                    } else {
                        format!("⚙ {name} · {summary}")
                    };
                    for piece in wrap(&text, w) {
                        out.push(Line::from(Span::styled(piece, Style::default().fg(DIM))));
                    }
                }
            }
        }
        out.push(Line::raw(""));
    }
    out
}

// ── terminal view (raw PTY) ───────────────────────────────────────────────────

fn render_terminal(f: &mut Frame, area: Rect, app: &mut App) {
    let title = app
        .chat_agent()
        .map(|a| format!(" {} ", a.short_cwd()))
        .unwrap_or_else(|| " session ended ".into());
    let border = if app.term_attached() { ACCENT } else { DIM };
    let bottom = if app.term_attached() {
        Line::from(Span::styled(" ● attached — Ctrl-] to detach ", Style::default().fg(ACCENT)))
    } else {
        Line::from(Span::styled(" i/enter to attach · t transcript ", Style::default().fg(DIM)))
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
                Style::default().fg(DIM),
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
    let Some(ws) = app.workspace() else { return };
    let mut spans = Vec::new();
    for (i, tab) in ws.tabs.iter().enumerate() {
        let style = if i == ws.active {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::styled("│", Style::default().fg(DIM)));
    }
    spans.push(Span::styled("  T:term  [ ]:tabs  w:close", Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── dashboard ──────────────────────────────────────────────────────────────────

fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" dashboard ")
        .border_style(Style::default().fg(ACCENT));

    let total = app.agents.len();
    let waiting = app.agents.iter().filter(|a| a.is_waiting()).count();
    let busy = app.agents.iter().filter(|a| a.is_busy()).count();
    let idle = total.saturating_sub(waiting + busy);
    let cost: f64 = app.agents.iter().filter_map(|a| a.usage.as_ref()).map(|u| u.cost_usd).sum();

    let mut lines = vec![
        Line::from(Span::styled(
            format!("workspacer · {total} agent{}", if total == 1 { "" } else { "s" }),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("needs you ", Style::default().fg(DIM)),
            Span::styled(format!("{waiting}"), Style::default().fg(WARN).add_modifier(Modifier::BOLD)),
            Span::styled("    working ", Style::default().fg(DIM)),
            Span::styled(format!("{busy}"), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("    idle ", Style::default().fg(DIM)),
            Span::styled(format!("{idle}"), Style::default().fg(OK)),
        ]),
        Line::from(vec![
            Span::styled("total cost ", Style::default().fg(DIM)),
            Span::styled(format!("${cost:.2}"), Style::default().fg(OK)),
        ]),
        Line::raw(""),
    ];

    // Compact roster, attention first (the agents are already sorted that way).
    for a in &app.agents {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(WARN))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(ACCENT))
        } else {
            Span::styled("· ", Style::default().fg(DIM))
        };
        let mut row = vec![
            marker,
            Span::styled(format!("{:<28}", crate::types::truncate(a.cwd_str(), 28)), Style::default()),
            Span::styled(
                format!("{:<10}", a.state()),
                Style::default().fg(state_color(a.state())),
            ),
        ];
        if let Some(u) = &a.usage {
            if u.context_limit > 0 && u.context_tokens > 0 {
                let pct = (u.context_tokens as f64 / u.context_limit as f64 * 100.0).round();
                row.push(Span::styled(format!(" {pct:.0}%"), Style::default().fg(DIM)));
            }
            if u.cost_usd > 0.0 {
                row.push(Span::styled(format!("  ${:.2}", u.cost_usd), Style::default().fg(DIM)));
            }
        }
        lines.push(Line::from(row));
    }
    if total == 0 {
        lines.push(Line::from(Span::styled(
            "no sessions yet — press c to spawn an agent",
            Style::default().fg(DIM),
        )));
    }

    let p = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

// ── spawn modal ───────────────────────────────────────────────────────────────

fn render_spawn_modal(f: &mut Frame, area: Rect, app: &App) {
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
            Span::styled("  cwd      ", Style::default().fg(DIM)),
            Span::raw(form.cwd.clone()),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]),
        Line::from(vec![
            Span::styled("  profile  ", Style::default().fg(DIM)),
            Span::styled("‹ ", Style::default().fg(ACCENT)),
            Span::styled(profile_name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" ›", Style::default().fg(ACCENT)),
            Span::styled(format!("  {}/{}", form.profile_idx + 1, n), Style::default().fg(DIM)),
            Span::styled(extra, Style::default().fg(DIM)),
        ]),
    ];

    // Tab-completion candidates, when the path is ambiguous.
    if !form.completions.is_empty() {
        let joined = form.completions.join("  ");
        let shown = crate::types::truncate(&joined, inner_w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {} {}", form.completions.len(), "matches:"),
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(Span::styled(format!("  {shown}"), Style::default().fg(ACCENT))));
    }

    // When seeding a library prompt, show what will be inserted.
    if let Some(prompt) = form.initial_prompt.as_ref() {
        let first = prompt.lines().next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled("  prompt   ", Style::default().fg(DIM)),
            Span::styled(
                crate::types::truncate(first, inner_w.saturating_sub(12)),
                Style::default().fg(OK),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  type a path · tab complete · ↑↓ profile · enter spawn · esc cancel",
        Style::default().fg(DIM),
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
        .border_style(Style::default().fg(ACCENT));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── command palette ─────────────────────────────────────────────────────────

fn render_palette(f: &mut Frame, area: Rect, app: &App) {
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
        Span::styled("› ", Style::default().fg(ACCENT)),
        Span::raw(p.query.clone()),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ])];

    // Scroll the list so the selection stays visible.
    let start = p.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, item) in visible.iter().skip(start).take(shown as usize).enumerate() {
        let i = start + offset;
        let selected = i == p.selected;
        let marker = if selected { "❯ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(ACCENT)),
            Span::styled(item.label.clone(), label_style),
        ];
        if !item.hint.is_empty() {
            let room = inner_w.saturating_sub(item.label.len() + 6);
            if room > 4 {
                spans.push(Span::styled(
                    format!("  {}", crate::types::truncate(&item.hint, room)),
                    Style::default().fg(DIM),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled("no matches", Style::default().fg(DIM))));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" command palette ")
        .title_bottom(Line::from(Span::styled(
            " ↑↓ move · enter run · esc close ",
            Style::default().fg(DIM),
        )))
        .border_style(Style::default().fg(ACCENT));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let in_agent = matches!(app.view, View::Agent { .. });
    let on_shell = matches!(app.active_tab().map(|t| t.kind), Some(TabKind::Shell));
    let hint = if app.spawn_form.is_some() {
        "type path · tab complete · ↑↓ profile · enter spawn · esc cancel"
    } else if app.term_attached() {
        "● attached — keys go to Claude · Ctrl-] to detach"
    } else if !in_agent {
        "j/k move · enter open · ^K palette · T term · c new · m attention · q quit"
    } else if app.insert_mode {
        "enter send · esc normal"
    } else if on_shell {
        "i attach · [ ] tabs · T term · w close · x/X stop · esc back · q quit"
    } else if app.chat_mode == ChatMode::Terminal {
        "i attach · t transcript · [ ] tabs · T term · w close · esc back · q quit"
    } else {
        "i type · j/k scroll · t terminal · [ ] tabs · T term · y/n/a · 1-9 · esc back"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {hint}"), Style::default().fg(DIM)))),
        area,
    );
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
