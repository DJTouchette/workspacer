//! ratatui rendering for the watch TUI. Pure function of `&App`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use time::OffsetDateTime;

use crate::session::{state::Pending, SessionMode};

use super::app::{App, ChatState, View};

pub fn render(frame: &mut Frame, app: &App) {
    match &app.view {
        View::Dashboard => render_dashboard(frame, app),
        View::Chat(chat) => render_chat(frame, app, chat),
    }
}

fn render_dashboard(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(6),    // sessions list
            Constraint::Length(12),// details panel
            Constraint::Length(1), // toast / status
            Constraint::Length(1), // key hints
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_sessions(frame, chunks[1], app);
    draw_details(frame, chunks[2], app);
    draw_toast(frame, chunks[3], app);
    draw_dashboard_hints(frame, chunks[4]);
}

fn render_chat(frame: &mut Frame, app: &App, chat: &ChatState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(4),    // transcript
            Constraint::Length(3), // pending banner (or empty)
            Constraint::Length(3), // input box
            Constraint::Length(1), // toast
            Constraint::Length(1), // hints
        ])
        .split(area);

    draw_chat_header(frame, chunks[0], app, chat);
    draw_transcript(frame, chunks[1], app, chat);
    draw_pending_banner(frame, chunks[2], app, chat);
    draw_input(frame, chunks[3], chat);
    draw_toast(frame, chunks[4], app);
    draw_chat_hints(frame, chunks[5], app, chat);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let dot = if app.connected { "●".green() } else { "●".red() };
    let line = Line::from(vec![
        " claudemon watch ".bold(),
        format!("─ {} ─ ", app.api_url).into(),
        format!("{} sessions ─ ", app.sessions.len()).into(),
        dot,
        " ".into(),
        if app.connected {
            "connected".green()
        } else {
            "disconnected".red()
        },
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_sessions(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .order
        .iter()
        .enumerate()
        .filter_map(|(i, id)| app.sessions.get(id).map(|s| (i, s)))
        .map(|(i, s)| {
            let selected = i == app.selected;
            let cursor = if selected { "▸ " } else { "  " };
            let badge = mode_badge(s.mode);
            let suffix = match &s.pending {
                Some(Pending::Approval { tool, summary, .. }) => {
                    let t = tool.as_deref().unwrap_or("?");
                    let s = summary.as_deref().unwrap_or("");
                    if s.is_empty() {
                        format!("{t}")
                    } else {
                        format!("{t}: {s}")
                    }
                }
                Some(Pending::Question { questions, .. }) => questions
                    .first()
                    .map(|q| q.question.clone())
                    .unwrap_or_default(),
                None => s
                    .cwd
                    .clone()
                    .unwrap_or_else(|| "—".into()),
            };
            let gate = if app.gate_on(&s.session_id) { " [gate]" } else { "" };
            let line = Line::from(vec![
                cursor.into(),
                Span::styled(
                    short_id(&s.session_id),
                    Style::default().fg(Color::Gray),
                ),
                "  ".into(),
                badge,
                " ".into(),
                Span::styled(suffix, Style::default().fg(Color::White)),
                Span::styled(gate, Style::default().fg(Color::Yellow)),
            ]);
            let style = if selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    let widget = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" sessions "),
    );
    frame.render_widget(widget, area);
}

fn draw_details(frame: &mut Frame, area: Rect, app: &App) {
    let Some(s) = app.selected_session() else {
        let p = Paragraph::new("no session selected")
            .block(Block::default().borders(Borders::ALL).title(" details "));
        frame.render_widget(p, area);
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("session   ", Style::default().fg(Color::DarkGray)),
        s.session_id.clone().into(),
    ]));
    lines.push(Line::from(vec![
        Span::styled("mode      ", Style::default().fg(Color::DarkGray)),
        mode_badge(s.mode),
    ]));
    if let Some(cwd) = &s.cwd {
        lines.push(Line::from(vec![
            Span::styled("cwd       ", Style::default().fg(Color::DarkGray)),
            cwd.clone().into(),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("started   ", Style::default().fg(Color::DarkGray)),
        format!("{} ({})", format_rfc(&s.started_at), ago(&s.started_at)).into(),
    ]));
    if let Some(last) = &s.last_event {
        lines.push(Line::from(vec![
            Span::styled("last event", Style::default().fg(Color::DarkGray)),
            format!(" {} ({})", last, ago(&s.updated_at)).into(),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("tool calls", Style::default().fg(Color::DarkGray)),
        format!(" {}", s.tool_calls).into(),
    ]));
    let gate = app.gate_on(&s.session_id);
    lines.push(Line::from(vec![
        Span::styled("gate      ", Style::default().fg(Color::DarkGray)),
        if gate { "ON".yellow().bold() } else { "off".into() },
    ]));

    if let Some(pending) = &s.pending {
        lines.push(Line::from(""));
        match pending {
            Pending::Approval { tool, summary, .. } => {
                lines.push(Line::from(vec![
                    "▶ ".yellow(),
                    "pending approval".bold(),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  tool   ", Style::default().fg(Color::DarkGray)),
                    tool.clone().unwrap_or_else(|| "?".into()).into(),
                ]));
                if let Some(sum) = summary {
                    lines.push(Line::from(vec![
                        Span::styled("  summary", Style::default().fg(Color::DarkGray)),
                        format!(" {sum}").into(),
                    ]));
                }
            }
            Pending::Question { questions, .. } => {
                lines.push(Line::from(vec![
                    "▶ ".cyan(),
                    "pending question".bold(),
                ]));
                for q in questions {
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        q.question.clone().into(),
                    ]));
                    for (i, opt) in q.options.iter().enumerate() {
                        let key = (i + 1).to_string();
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("    [{key}] "),
                                Style::default().fg(Color::Yellow),
                            ),
                            opt.label.clone().into(),
                        ]));
                    }
                }
            }
        }
    }

    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" details — {} ", short_id(&s.session_id))),
        );
    frame.render_widget(p, area);
}

fn draw_toast(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(t) = app.current_toast() {
        frame.render_widget(
            Paragraph::new(format!(" {t}")).style(Style::default().fg(Color::Yellow)),
            area,
        );
    }
}

fn draw_dashboard_hints(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        " ".into(),
        "↑↓".bold(),
        " nav   ".into(),
        "Enter".bold().cyan(),
        " open   ".into(),
        "a".bold().green(),
        " approve   ".into(),
        "d".bold().red(),
        " deny   ".into(),
        "1-9".bold(),
        " answer   ".into(),
        "g".bold(),
        " gate   ".into(),
        "r".bold(),
        " refresh   ".into(),
        "q".bold(),
        " quit ".into(),
    ]);
    frame.render_widget(Paragraph::new(line).style(Style::default().fg(Color::DarkGray)), area);
}

fn draw_chat_hints(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let mut spans: Vec<Span> = vec![
        " ".into(),
        "Enter".bold().cyan(),
        " send   ".into(),
        "Esc".bold(),
        " back   ".into(),
        "PgUp/PgDn".bold(),
        " scroll   ".into(),
    ];
    // Surface the most relevant action keys based on current pending state.
    if let Some(s) = app.sessions.get(&chat.session_id) {
        match s.mode {
            SessionMode::Approval => spans.extend(vec![
                "a".bold().green(),
                " approve   ".into(),
                "d".bold().red(),
                " deny   ".into(),
            ]),
            SessionMode::Question => {
                spans.extend(vec!["1-9".bold(), " option   ".into()])
            }
            _ => {}
        }
    }
    spans.extend(vec!["r".bold(), " refresh   ".into(), "q".bold(), " quit ".into()]);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn draw_chat_header(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let state = app.sessions.get(&chat.session_id);
    let mode = state.map(|s| s.mode).unwrap_or(SessionMode::Unknown);
    let cwd = state.and_then(|s| s.cwd.as_deref()).unwrap_or("—");
    let dot = if app.connected { "●".green() } else { "●".red() };
    let line = Line::from(vec![
        " ".into(),
        "←".cyan(),
        " ".into(),
        Span::styled(short_id(&chat.session_id), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        "  ".into(),
        Span::styled(cwd.to_string(), Style::default().fg(Color::Gray)),
        "  ".into(),
        mode_badge(mode),
        "  ".into(),
        dot,
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_transcript(frame: &mut Frame, area: Rect, _app: &App, chat: &ChatState) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    if chat.transcript.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no transcript yet — Claude hasn't produced one for this cwd, or hooks aren't wired up)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for msg in &chat.transcript.messages {
            let prefix = match msg.role.as_str() {
                "user" => Span::styled("[you] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                "assistant" => Span::styled("[claude] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                "system" => Span::styled("[sys] ", Style::default().fg(Color::DarkGray)),
                other => Span::styled(format!("[{other}] "), Style::default().fg(Color::Gray)),
            };
            let text = msg.text.clone().unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let prefix_len = prefix.content.chars().count();
            let wrap_width = inner_width.saturating_sub(prefix_len).max(20);
            let mut first = true;
            for raw_line in text.lines() {
                for chunk in wrap_str(raw_line, wrap_width) {
                    if first {
                        lines.push(Line::from(vec![prefix.clone(), chunk.into()]));
                        first = false;
                    } else {
                        let indent: String =
                            std::iter::repeat(' ').take(prefix_len).collect();
                        lines.push(Line::from(vec![indent.into(), chunk.into()]));
                    }
                }
            }
            lines.push(Line::from(""));
        }
    }

    // Auto-tail: show the last `viewport` lines, offset upward by scroll_offset.
    let viewport = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let end = total.saturating_sub(chat.scroll_offset as usize);
    let start = end.saturating_sub(viewport);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(end - start).collect();

    let title = if let Some(path) = chat.transcript.path.as_deref() {
        format!(" transcript — {} ", path.split('/').last().unwrap_or(path))
    } else {
        " transcript ".to_string()
    };

    let p = Paragraph::new(visible)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(p, area);
}

fn draw_pending_banner(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let Some(state) = app.sessions.get(&chat.session_id) else {
        frame.render_widget(Paragraph::new(""), area);
        return;
    };
    match &state.pending {
        Some(Pending::Approval { tool, summary, .. }) => {
            let t = tool.as_deref().unwrap_or("?");
            let s = summary.as_deref().unwrap_or("");
            let title = if s.is_empty() {
                format!(" ▶ pending approval: {t} ")
            } else {
                format!(" ▶ pending approval: {t} — {s} ")
            };
            let body = Line::from(vec![
                "  [a]".green().bold(),
                " approve   ".into(),
                "[d]".red().bold(),
                " deny".into(),
            ]);
            let p = Paragraph::new(body).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title),
            );
            frame.render_widget(p, area);
        }
        Some(Pending::Question { questions, .. }) => {
            let title = questions
                .first()
                .map(|q| format!(" ▶ {} ", q.question))
                .unwrap_or_else(|| " ▶ pending question ".to_string());
            let opts: Vec<Span> = questions
                .first()
                .map(|q| {
                    let mut out: Vec<Span> = vec!["  ".into()];
                    for (i, opt) in q.options.iter().enumerate() {
                        let key = i + 1;
                        out.push(format!("[{key}]").yellow().bold());
                        out.push(format!(" {}   ", opt.label).into());
                    }
                    out
                })
                .unwrap_or_default();
            let p = Paragraph::new(Line::from(opts)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Magenta))
                    .title(title),
            );
            frame.render_widget(p, area);
        }
        None => {
            frame.render_widget(Paragraph::new(""), area);
        }
    }
}

fn draw_input(frame: &mut Frame, area: Rect, chat: &ChatState) {
    let cursor = if chat.input.is_empty() {
        Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED))
    } else {
        Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED))
    };
    let line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(Color::Cyan)),
        chat.input.clone().into(),
        cursor,
    ]);
    let p = Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL).title(" message "));
    frame.render_widget(p, area);
}

fn wrap_str(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in s.split_inclusive(' ') {
        if current.chars().count() + word.chars().count() > width {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            if word.chars().count() > width {
                // Hard-split very long words.
                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if chunk.chars().count() >= width {
                        out.push(std::mem::take(&mut chunk));
                    }
                }
                current = chunk;
            } else {
                current.push_str(word);
            }
        } else {
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn mode_badge(mode: SessionMode) -> Span<'static> {
    match mode {
        SessionMode::Unknown => "[ unknown    ]".dark_gray(),
        SessionMode::Input => "[ input      ]".cyan(),
        SessionMode::Responding => "[ responding ]".blue(),
        SessionMode::Approval => "[ APPROVAL   ]".yellow().bold(),
        SessionMode::Question => "[ QUESTION   ]".magenta().bold(),
        SessionMode::Stopped => "[ stopped    ]".dark_gray(),
    }
}

fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        format!("{}…", &id[..8])
    }
}

fn ago(t: &OffsetDateTime) -> String {
    let now = OffsetDateTime::now_utc();
    let delta = now - *t;
    let secs = delta.whole_seconds();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

fn format_rfc(t: &OffsetDateTime) -> String {
    let fmt = time::format_description::well_known::Rfc3339;
    t.format(&fmt).unwrap_or_default()
}

