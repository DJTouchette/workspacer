//! ratatui rendering for the watch TUI. Pure function of `&App`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use time::OffsetDateTime;

use crate::session::{
    state::Pending,
    transcript::{self, Block as MsgBlock},
    SessionMode,
};

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
    // Input box height grows with content (up to 10 lines visible).
    // Inner width = area.width - 2 borders - 2 padding.
    let inner_input_width = area.width.saturating_sub(4).max(1);
    let input_rows = chat.editor.visual_rows(inner_input_width).clamp(1, 10);
    let input_box_height = input_rows + 2; // borders

    let pending_height = if pending_banner_height(app, chat) > 0 {
        3
    } else {
        0
    };

    let mut constraints = vec![
        Constraint::Length(1),                 // header
        Constraint::Min(4),                    // transcript
    ];
    if pending_height > 0 {
        constraints.push(Constraint::Length(pending_height));
    }
    constraints.push(Constraint::Length(input_box_height));
    constraints.push(Constraint::Length(1));   // toast
    constraints.push(Constraint::Length(1));   // hints

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut i = 0;
    draw_chat_header(frame, chunks[i], app, chat); i += 1;
    draw_transcript(frame, chunks[i], app, chat); i += 1;
    if pending_height > 0 {
        draw_pending_banner(frame, chunks[i], app, chat); i += 1;
    }
    let input_area = chunks[i];
    draw_input(frame, input_area, chat); i += 1;
    draw_toast(frame, chunks[i], app); i += 1;
    draw_chat_hints(frame, chunks[i], app, chat);

    // Place the OS cursor at the editor's visual position inside the
    // input box so the user can see where they're typing.
    let (cx, cy) = chat.editor.visual_cursor(inner_input_width);
    // input_area.x + 1 (border) + 1 (left padding) gives column 0 of content.
    let cur_x = input_area.x.saturating_add(2 + cx);
    let cur_y = input_area.y.saturating_add(1 + cy);
    // Clamp to the input area's bounds.
    let cx_clamped = cur_x.min(input_area.x + input_area.width.saturating_sub(2));
    let cy_clamped = cur_y.min(input_area.y + input_area.height.saturating_sub(2));
    frame.set_cursor_position((cx_clamped, cy_clamped));
}

fn pending_banner_height(app: &App, chat: &ChatState) -> u16 {
    let Some(state) = app.sessions.get(&chat.session_id) else { return 0; };
    if state.pending.is_some() { 3 } else { 0 }
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
        "Alt+Enter".bold(),
        " newline   ".into(),
        "↑↓".bold(),
        " history   ".into(),
        "Esc".bold(),
        " back   ".into(),
    ];
    // Pending-state shortcuts (visible only when relevant).
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
    spans.extend(vec!["q".bold(), " quit ".into()]);
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
            render_transcript_message(msg, inner_width, &mut lines);
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
    let inner_width = area.width.saturating_sub(4).max(1) as usize;
    let text = chat.editor.text();

    // Render the text into lines that preserve hard newlines AND show a
    // placeholder when empty so the input box is never just a blank slab.
    let lines: Vec<Line> = if text.is_empty() {
        vec![Line::from(Span::styled(
            "(type a message — Enter to send, Alt+Enter for newline, ↑ for history)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        text.split('\n')
            .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::White))))
            .collect()
    };

    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" message ")
                .border_style(Style::default().fg(Color::Cyan))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        );
    frame.render_widget(p, area);

    // Keep the placeholder unused to avoid warnings if we re-add `_` style.
    let _ = inner_width;
}

fn render_transcript_message(
    msg: &crate::session::transcript::TranscriptMessage,
    inner_width: usize,
    out: &mut Vec<Line<'static>>,
) {
    let prefix = match msg.role.as_str() {
        "user" => Span::styled(
            "[you] ",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        "assistant" => Span::styled(
            "[claude] ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        other => Span::styled(
            format!("[{other}] "),
            Style::default().fg(Color::Gray),
        ),
    };
    let prefix_len = prefix.content.chars().count();
    let indent: String = std::iter::repeat(' ').take(prefix_len).collect();
    let wrap_width = inner_width.saturating_sub(prefix_len).max(20);

    let blocks = transcript::blocks(&msg.content);
    let mut emitted_for_msg = false;

    for block in blocks {
        match block {
            MsgBlock::Text { text } => {
                let trimmed = text.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                let mut first = !emitted_for_msg;
                for raw_line in trimmed.lines() {
                    for chunk in wrap_str(raw_line, wrap_width) {
                        if first {
                            out.push(Line::from(vec![prefix.clone(), chunk.into()]));
                            first = false;
                        } else {
                            out.push(Line::from(vec![indent.clone().into(), chunk.into()]));
                        }
                    }
                }
                emitted_for_msg = true;
            }
            MsgBlock::ToolUse { name, input } => {
                let summary = transcript::summarize_tool_input(name, input);
                let head_prefix = if emitted_for_msg { indent.clone().into() } else { prefix.clone() };
                let arrow = Span::styled(
                    "⏺ ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                );
                let tool_name = Span::styled(
                    name.to_string(),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                );
                let head_extra: Vec<Span> = if summary.is_empty() {
                    vec![arrow, tool_name]
                } else {
                    vec![arrow, tool_name, "  ".into(), Span::styled(
                        first_line_truncated(&summary, wrap_width.saturating_sub(8)),
                        Style::default().fg(Color::White),
                    )]
                };
                let mut head_line = vec![head_prefix];
                head_line.extend(head_extra);
                out.push(Line::from(head_line));
                emitted_for_msg = true;
            }
            MsgBlock::ToolResult { content, is_error } => {
                let flat = transcript::flatten_tool_result(content);
                let color = if is_error { Color::Red } else { Color::DarkGray };
                let tag = if is_error { "  ↳ error: " } else { "  ↳ " };
                let head_prefix = if emitted_for_msg { indent.clone().into() } else { prefix.clone() };
                let body_width = wrap_width.saturating_sub(4).max(20);

                // Show up to 4 lines; collapse the rest into a "+N more" hint.
                let mut shown = 0usize;
                let mut total_lines = 0usize;
                let mut first_block = true;
                for raw_line in flat.lines() {
                    total_lines += 1;
                    if shown >= 4 {
                        continue;
                    }
                    for chunk in wrap_str(raw_line, body_width) {
                        if first_block {
                            out.push(Line::from(vec![
                                head_prefix.clone(),
                                Span::styled(tag.to_string(), Style::default().fg(color)),
                                Span::styled(chunk, Style::default().fg(color)),
                            ]));
                            first_block = false;
                        } else {
                            out.push(Line::from(vec![
                                indent.clone().into(),
                                Span::styled(
                                    "    ".to_string(),
                                    Style::default().fg(color),
                                ),
                                Span::styled(chunk, Style::default().fg(color)),
                            ]));
                        }
                    }
                    shown += 1;
                }
                if total_lines == 0 {
                    // No content — emit just the tag so the user can see something happened.
                    out.push(Line::from(vec![
                        head_prefix,
                        Span::styled(format!("{tag}(empty)"), Style::default().fg(color)),
                    ]));
                } else if total_lines > shown {
                    out.push(Line::from(vec![
                        indent.clone().into(),
                        Span::styled(
                            format!("    +{} more line{}", total_lines - shown,
                                if total_lines - shown == 1 { "" } else { "s" }),
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                emitted_for_msg = true;
            }
            MsgBlock::Thinking { .. } => {
                // Skipped by default — too noisy. Could add a toggle later.
            }
        }
    }

    if emitted_for_msg {
        out.push(Line::from(""));
    }
}

fn first_line_truncated(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() <= max {
        first.to_string()
    } else {
        let mut out: String = first.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transcript::{Transcript, TranscriptMessage};
    use crate::tui::app::{App, ChatState, View};
    use crate::tui::editor::Editor;
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;

    fn snapshot(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn build_chat_app(messages: Vec<TranscriptMessage>) -> App {
        let mut app = App::new("http://test".into());
        let session = crate::session::SessionState {
            session_id: "test-session-id".into(),
            cwd: Some("/tmp/x".into()),
            mode: SessionMode::Input,
            pending: None,
            started_at: time::OffsetDateTime::now_utc(),
            updated_at: time::OffsetDateTime::now_utc(),
            tool_calls: 0,
            last_event: Some("SessionStart".into()),
        };
        app.sessions.insert(session.session_id.clone(), session);
        app.order.push("test-session-id".into());
        app.view = View::Chat(ChatState {
            session_id: "test-session-id".into(),
            transcript: Transcript {
                path: Some("/x.jsonl".into()),
                messages,
            },
            editor: Editor::new(),
            scroll_offset: 0,
            last_seen_mode: SessionMode::Input,
        });
        app
    }

    #[test]
    fn empty_chat_shows_placeholder_and_renders_borders() {
        let app = build_chat_app(vec![]);
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("transcript"), "transcript title missing\n{s}");
        assert!(s.contains("message"), "message title missing\n{s}");
        assert!(
            s.contains("type a message"),
            "placeholder hint missing\n{s}"
        );
        assert!(s.contains("Enter") && s.contains("send"), "hints missing\n{s}");
        assert!(s.contains("Alt+Enter"), "newline hint missing\n{s}");
        assert!(s.contains("↑↓") && s.contains("history"), "history hint missing\n{s}");
    }

    #[test]
    fn chat_renders_text_messages_with_role_prefixes() {
        let messages = vec![
            TranscriptMessage {
                role: "user".into(),
                content: json!("hello there"),
                raw: json!({}),
            },
            TranscriptMessage {
                role: "assistant".into(),
                content: json!([{ "type": "text", "text": "hi back" }]),
                raw: json!({}),
            },
        ];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("[you]"), "user prefix missing\n{s}");
        assert!(s.contains("hello there"), "user msg missing\n{s}");
        assert!(s.contains("[claude]"), "assistant prefix missing\n{s}");
        assert!(s.contains("hi back"), "assistant msg missing\n{s}");
    }

    #[test]
    fn chat_renders_tool_use_with_bash_summary() {
        let messages = vec![TranscriptMessage {
            role: "assistant".into(),
            content: json!([
                { "type": "text", "text": "Running:" },
                { "type": "tool_use", "name": "Bash", "id": "x",
                  "input": { "command": "ls -la /tmp" } }
            ]),
            raw: json!({}),
        }];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("Running:"), "text block missing\n{s}");
        assert!(s.contains("Bash"), "tool name missing\n{s}");
        assert!(s.contains("ls -la /tmp"), "tool summary missing\n{s}");
    }

    #[test]
    fn chat_renders_tool_result_with_arrow_indent() {
        let messages = vec![TranscriptMessage {
            role: "user".into(),
            content: json!([
                { "type": "tool_result", "tool_use_id": "x",
                  "content": "main.rs\nCargo.toml" }
            ]),
            raw: json!({}),
        }];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("↳"), "tool result arrow missing\n{s}");
        assert!(s.contains("main.rs"), "first result line missing\n{s}");
    }

    #[test]
    fn pending_approval_banner_appears_when_mode_is_approval() {
        let mut app = build_chat_app(vec![]);
        if let Some(s) = app.sessions.get_mut("test-session-id") {
            s.mode = SessionMode::Approval;
            s.pending = Some(Pending::Approval {
                tool: Some("Bash".into()),
                summary: Some("rm -rf /tmp/x".into()),
                raw: json!({}),
            });
        }
        let s = snapshot(&app, 80, 22);
        assert!(s.contains("pending approval"), "banner title missing\n{s}");
        assert!(s.contains("Bash"), "tool not shown\n{s}");
        assert!(s.contains("rm -rf /tmp/x"), "summary not shown\n{s}");
        assert!(s.contains("[a]") && s.contains("[d]"), "action keys missing\n{s}");
    }

    #[test]
    fn pending_question_banner_lists_options() {
        let mut app = build_chat_app(vec![]);
        if let Some(state) = app.sessions.get_mut("test-session-id") {
            state.mode = SessionMode::Question;
            state.pending = Some(Pending::Question {
                questions: vec![crate::session::state::PendingQuestion {
                    question: "Pick one?".into(),
                    header: Some("Pick".into()),
                    multi_select: false,
                    options: vec![
                        crate::session::state::PendingOption {
                            label: "alpha".into(),
                            description: None,
                        },
                        crate::session::state::PendingOption {
                            label: "beta".into(),
                            description: None,
                        },
                    ],
                }],
                raw: json!({}),
            });
        }
        let s = snapshot(&app, 80, 22);
        assert!(s.contains("Pick one?"), "question text missing\n{s}");
        assert!(s.contains("[1]") && s.contains("alpha"), "option 1 missing\n{s}");
        assert!(s.contains("[2]") && s.contains("beta"), "option 2 missing\n{s}");
    }

    #[test]
    fn editor_text_appears_in_input_box() {
        let mut app = build_chat_app(vec![]);
        if let View::Chat(chat) = &mut app.view {
            for ch in "hello world".chars() {
                chat.editor.insert(ch);
            }
        }
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("hello world"), "typed text missing\n{s}");
        // Placeholder should be gone once there's content.
        assert!(!s.contains("type a message"), "placeholder still shown\n{s}");
    }

    #[test]
    fn dashboard_shows_session_with_mode_badge() {
        let mut app = build_chat_app(vec![]);
        app.view = View::Dashboard;
        let s = snapshot(&app, 100, 20);
        assert!(s.contains("test-ses"), "session id short form missing\n{s}");
        assert!(s.contains("input"), "mode badge missing\n{s}");
        assert!(s.contains("/tmp/x"), "cwd missing\n{s}");
    }
}

