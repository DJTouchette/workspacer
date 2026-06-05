//! Dashboard view rendering — session list, details panel, and hints.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::session::{state::Pending, SessionMode};

use crate::tui::app::App;

use super::{ago, hint, kv, label, mode_badge, mode_badge_padded, short_id, draw_toast};

pub(super) fn render_dashboard(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // header
            Constraint::Min(6),     // sessions
            Constraint::Length(11), // details
            Constraint::Length(1),  // toast
            Constraint::Length(1),  // hints
        ])
        .split(area);

    draw_dashboard_header(frame, chunks[0], app);
    draw_sessions(frame, chunks[1], app);
    draw_details(frame, chunks[2], app);
    draw_toast(frame, chunks[3], app);
    draw_dashboard_hints(frame, chunks[4], app);
}

fn draw_dashboard_header(frame: &mut Frame, area: Rect, app: &App) {
    let dot = if app.connected {
        "●".green()
    } else {
        "●".red()
    };
    let link = if app.connected {
        Span::styled("live", Style::default().fg(Color::Green))
    } else {
        Span::styled("offline", Style::default().fg(Color::Red))
    };
    let line = Line::from(vec![
        " ".into(),
        "claudemon".bold(),
        Span::styled(
            format!("  {}  ", app.api_url),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!(
                "{} {}",
                app.sessions.len(),
                if app.sessions.len() == 1 {
                    "session"
                } else {
                    "sessions"
                }
            ),
            Style::default().fg(Color::White),
        ),
        "   ".into(),
        dot,
        " ".into(),
        link,
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_sessions(frame: &mut Frame, area: Rect, app: &App) {
    if app.order.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  no sessions yet",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  start the daemon and wrap a claude session:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(vec![
                "    ".into(),
                Span::styled("$ ", Style::default().fg(Color::DarkGray)),
                Span::styled("claudemon serve", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                "    ".into(),
                Span::styled("$ ", Style::default().fg(Color::DarkGray)),
                Span::styled("claudemon init", Style::default().fg(Color::White)),
                Span::styled(
                    "    # install hooks (one-time)",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(vec![
                "    ".into(),
                Span::styled("$ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "claudemon wrap -- claude",
                    Style::default().fg(Color::White),
                ),
            ]),
        ];
        let p = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(" sessions "),
        );
        frame.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = app
        .order
        .iter()
        .enumerate()
        .filter_map(|(i, id)| app.sessions.get(id).map(|s| (i, s)))
        .map(|(i, s)| session_row(app, i, s))
        .collect();
    let widget = List::new(items).block(Block::default().borders(Borders::ALL).title(" sessions "));
    frame.render_widget(widget, area);
}

fn session_row<'a>(app: &App, i: usize, s: &'a crate::session::SessionState) -> ListItem<'a> {
    let selected = i == app.selected;
    let cursor = if selected { "▸ " } else { "  " };
    let id_span = Span::styled(short_id(&s.session_id), Style::default().fg(Color::Gray));

    let badge = mode_badge_padded(s.mode);

    let context = match &s.pending {
        Some(Pending::Approval { tool, summary, .. }) => {
            let t = tool.as_deref().unwrap_or("tool");
            match summary.as_deref() {
                Some(s) if !s.is_empty() => format!("{t}: {s}"),
                _ => t.to_string(),
            }
        }
        Some(Pending::Question { questions, .. }) => questions
            .first()
            .map(|q| q.question.clone())
            .unwrap_or_else(|| "question".into()),
        None => s.cwd.clone().unwrap_or_else(|| "—".into()),
    };

    let context_style = match &s.pending {
        Some(Pending::Approval { .. }) => Style::default().fg(Color::Yellow),
        Some(Pending::Question { .. }) => Style::default().fg(Color::Magenta),
        None => Style::default().fg(Color::White),
    };

    let gate = if app.gate_on(&s.session_id) {
        Span::styled(" gate", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };

    let line = Line::from(vec![
        cursor.into(),
        id_span,
        "  ".into(),
        badge,
        "  ".into(),
        Span::styled(context, context_style),
        gate,
    ]);
    let style = if selected {
        Style::default().bg(Color::Rgb(35, 35, 50))
    } else {
        Style::default()
    };
    ListItem::new(line).style(style)
}

fn draw_details(frame: &mut Frame, area: Rect, app: &App) {
    let Some(s) = app.selected_session() else {
        let p = Paragraph::new(Line::from(Span::styled(
            "  (no session selected — ↑↓ to pick, Enter to open)",
            Style::default().fg(Color::DarkGray),
        )))
        .block(Block::default().borders(Borders::ALL).title(" details "));
        frame.render_widget(p, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(kv("session", &s.session_id));
    lines.push(Line::from(vec![label("mode"), mode_badge(s.mode)]));
    if let Some(cwd) = &s.cwd {
        lines.push(kv("cwd", cwd));
    }
    lines.push(kv("started", &ago(&s.started_at)));
    if let Some(last) = &s.last_event {
        lines.push(Line::from(vec![
            label("last"),
            last.clone().into(),
            Span::styled(
                format!("  {}", ago(&s.updated_at)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    lines.push(kv("tools", &s.tool_calls.to_string()));
    let gate_on = app.gate_on(&s.session_id);
    lines.push(Line::from(vec![
        label("gate"),
        if gate_on {
            "on".yellow().bold()
        } else {
            "off".dark_gray()
        },
    ]));

    if let Some(pending) = &s.pending {
        lines.push(Line::from(""));
        match pending {
            Pending::Approval { tool, summary, .. } => {
                lines.push(Line::from(vec![
                    "  ▶ ".yellow(),
                    "approval".yellow().bold(),
                    "  ".into(),
                    Span::styled(
                        tool.clone().unwrap_or_else(|| "tool".into()),
                        Style::default().fg(Color::White),
                    ),
                ]));
                if let Some(sum) = summary {
                    lines.push(Line::from(vec![
                        "    ".into(),
                        Span::styled(sum.clone(), Style::default().fg(Color::Gray)),
                    ]));
                }
            }
            Pending::Question { questions, .. } => {
                let q = questions.first();
                lines.push(Line::from(vec![
                    "  ▶ ".magenta(),
                    "question".magenta().bold(),
                ]));
                if let Some(q) = q {
                    lines.push(Line::from(vec![
                        "    ".into(),
                        Span::styled(q.question.clone(), Style::default().fg(Color::White)),
                    ]));
                }
            }
        }
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" details — {} ", short_id(&s.session_id))),
    );
    frame.render_widget(p, area);
}

fn draw_dashboard_hints(frame: &mut Frame, area: Rect, app: &App) {
    // Show pending-state shortcuts only when there's something to act on.
    let has_approval = app
        .sessions
        .values()
        .any(|s| s.mode == SessionMode::Approval);
    let has_question = app
        .sessions
        .values()
        .any(|s| s.mode == SessionMode::Question);

    let mut spans = vec![
        " ".into(),
        "↑↓".bold(),
        hint(" nav  "),
        "Enter".bold().cyan(),
        hint(" open  "),
    ];
    if has_approval {
        spans.extend(vec![
            "a".bold().green(),
            hint(" allow  "),
            "d".bold().red(),
            hint(" deny  "),
        ]);
    }
    if has_question {
        spans.extend(vec!["1-9".bold(), hint(" answer  ")]);
    }
    spans.extend(vec![
        "g".bold(),
        hint(" gate  "),
        "r".bold(),
        hint(" refresh  "),
        "q".bold(),
        hint(" quit "),
    ]);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
