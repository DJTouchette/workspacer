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

use super::app::App;

pub fn render(frame: &mut Frame, app: &App) {
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
    draw_hints(frame, chunks[4]);
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

fn draw_hints(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        " ".into(),
        "↑↓".bold(),
        " nav   ".into(),
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

