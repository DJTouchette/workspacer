//! ratatui rendering for the watch TUI. Pure function of `&App`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use time::OffsetDateTime;

use crate::session::{
    state::Pending,
    transcript::{self, Block as MsgBlock},
    SessionMode,
};

use super::app::{App, ChatState, View};
use super::syntax;

/// Width allocated to mode badges so columns line up across rows.
/// Widest token is "responding" (10 chars).
const BADGE_WIDTH: usize = 10;
/// Color used for the active/focused border (input box, selected items).
const FOCUS: Color = Color::Cyan;
/// Footer hint labels need to remain readable on dark terminal themes.
const HINT_LABEL: Color = Color::Gray;

pub fn render(frame: &mut Frame, app: &App) {
    match &app.view {
        View::Dashboard => render_dashboard(frame, app),
        View::Chat(chat) => render_chat(frame, app, chat),
    }
}

// ─── Dashboard ──────────────────────────────────────────────────────────

fn render_dashboard(frame: &mut Frame, app: &App) {
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

fn draw_toast(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(t) = app.current_toast() {
        frame.render_widget(
            Paragraph::new(format!(" {t}")).style(Style::default().fg(Color::Yellow)),
            area,
        );
    }
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

// ─── Chat ──────────────────────────────────────────────────────────────

fn render_chat(frame: &mut Frame, app: &App, chat: &ChatState) {
    let area = frame.area();
    let inner_input_width = area.width.saturating_sub(4).max(1);
    let input_rows = chat.editor.visual_rows(inner_input_width).clamp(1, 10);
    let input_box_height = input_rows + 2;

    let pending_height = if pending_banner_height(app, chat) > 0 {
        4
    } else {
        0
    };

    let mut constraints = vec![Constraint::Length(1), Constraint::Min(4)];
    if pending_height > 0 {
        constraints.push(Constraint::Length(pending_height));
    }
    constraints.push(Constraint::Length(input_box_height));
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut i = 0;
    draw_chat_header(frame, chunks[i], app, chat);
    i += 1;
    draw_transcript(frame, chunks[i], app, chat);
    i += 1;
    if pending_height > 0 {
        draw_pending_banner(frame, chunks[i], app, chat);
        i += 1;
    }
    let input_area = chunks[i];
    draw_input(frame, input_area, chat);
    i += 1;
    draw_toast(frame, chunks[i], app);
    i += 1;
    draw_chat_hints(frame, chunks[i], app, chat);

    let (cx, cy) = chat.editor.visual_cursor(inner_input_width);
    let cur_x = input_area.x.saturating_add(2 + cx);
    let cur_y = input_area.y.saturating_add(1 + cy);
    let cx_clamped = cur_x.min(input_area.x + input_area.width.saturating_sub(2));
    let cy_clamped = cur_y.min(input_area.y + input_area.height.saturating_sub(2));
    frame.set_cursor_position((cx_clamped, cy_clamped));
}

fn pending_banner_height(app: &App, chat: &ChatState) -> u16 {
    let Some(state) = app.sessions.get(&chat.session_id) else {
        return 0;
    };
    if state.pending.is_some() {
        4
    } else {
        0
    }
}

fn draw_chat_header(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let state = app.sessions.get(&chat.session_id);
    let mode = state.map(|s| s.mode).unwrap_or(SessionMode::Unknown);
    let cwd = state.and_then(|s| s.cwd.as_deref()).unwrap_or("—");
    let dot = if app.connected {
        "●".green()
    } else {
        "●".red()
    };
    let line = Line::from(vec![
        " ".into(),
        Span::styled("← Esc", Style::default().fg(Color::Cyan)),
        "  ".into(),
        Span::styled(
            short_id(&chat.session_id),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}  ", cwd), Style::default().fg(Color::Gray)),
        mode_badge(mode),
        "   ".into(),
        dot,
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_transcript(frame: &mut Frame, area: Rect, _app: &App, chat: &ChatState) {
    let inner_width = area.width.saturating_sub(2) as usize;

    let key = super::app::TranscriptRenderKey {
        msg_count: chat.transcript.messages.len(),
        last_msg_signature: chat
            .transcript
            .messages
            .last()
            .map(|m| m.content.to_string())
            .unwrap_or_default(),
        inner_width,
        expand_tool_results: chat.expand_tool_results,
    };

    {
        // Rebuild only when the (transcript, width, expand) tuple changes.
        // Scrolling keeps the cache hot — it only changes scroll_offset,
        // which isn't part of the key.
        let mut cache = chat.render_cache.borrow_mut();
        let needs_rebuild = cache.as_ref().map_or(true, |c| c.key != key);
        if needs_rebuild {
            let lines = build_transcript_lines(chat, inner_width);
            *cache = Some(super::app::TranscriptRenderCache { key, lines });
        }
    }

    let cache = chat.render_cache.borrow();
    let lines: &[Line<'static>] = cache
        .as_ref()
        .map(|c| c.lines.as_slice())
        .unwrap_or(&[]);

    let viewport = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let end = total.saturating_sub(chat.scroll_offset as usize);
    let start = end.saturating_sub(viewport);
    let visible: Vec<Line> = lines
        .iter()
        .skip(start)
        .take(end - start)
        .cloned()
        .collect();

    let title = if let Some(path) = chat.transcript.path.as_deref() {
        format!(" transcript · {} ", path.split('/').last().unwrap_or(path))
    } else {
        " transcript ".to_string()
    };

    let border_style = if chat.transcript_focus {
        Style::default().fg(FOCUS)
    } else {
        Style::default()
    };
    let p = Paragraph::new(visible).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title),
    );
    frame.render_widget(p, area);
}

fn draw_pending_banner(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let Some(state) = app.sessions.get(&chat.session_id) else {
        frame.render_widget(Paragraph::new(""), area);
        return;
    };
    match &state.pending {
        Some(Pending::Approval { tool, summary, .. }) => {
            let mut body: Vec<Line> = Vec::new();
            body.push(Line::from(vec![
                "  ".into(),
                Span::styled(
                    tool.clone().unwrap_or_else(|| "tool".into()),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    summary
                        .as_deref()
                        .map(|s| format!("  {s}"))
                        .unwrap_or_default(),
                    Style::default().fg(Color::White),
                ),
            ]));
            body.push(Line::from(vec![
                "  ".into(),
                Span::styled(
                    "[a]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                " allow   ".into(),
                Span::styled(
                    "[d]",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                " deny".into(),
            ]));
            let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_set(symbols::border::DOUBLE)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" approval needed "),
            );
            frame.render_widget(p, area);
        }
        Some(Pending::Question { questions, .. }) => {
            let mut body: Vec<Line> = Vec::new();
            if let Some(q) = questions.first() {
                body.push(Line::from(vec![
                    "  ".into(),
                    Span::styled(
                        q.question.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let mut opt_spans: Vec<Span> = vec!["  ".into()];
                for (i, opt) in q.options.iter().enumerate() {
                    let key = i + 1;
                    if i > 0 {
                        opt_spans.push("   ".into());
                    }
                    opt_spans.push(Span::styled(
                        format!("[{key}]"),
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ));
                    opt_spans.push(format!(" {}", opt.label).into());
                }
                body.push(Line::from(opt_spans));
                if questions.len() > 1 {
                    body.push(Line::from(Span::styled(
                        format!(
                            "  (+{} more question{} after this)",
                            questions.len() - 1,
                            if questions.len() == 2 { "" } else { "s" }
                        ),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
            }
            let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_set(symbols::border::DOUBLE)
                    .border_style(Style::default().fg(Color::Magenta))
                    .title(" question "),
            );
            frame.render_widget(p, area);
        }
        None => {
            frame.render_widget(Paragraph::new(""), area);
        }
    }
}

fn draw_input(frame: &mut Frame, area: Rect, chat: &ChatState) {
    let text = chat.editor.text();
    let lines: Vec<Line> = if text.is_empty() {
        vec![Line::from(Span::styled(
            "type a message…",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ))]
    } else {
        text.split('\n')
            .map(|l| {
                Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::White),
                ))
            })
            .collect()
    };
    let p = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if chat.transcript_focus {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(FOCUS)
            })
            .title(Span::styled(
                " message ",
                if chat.transcript_focus {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(FOCUS)
                },
            ))
            .padding(ratatui::widgets::Padding::horizontal(1)),
    );
    frame.render_widget(p, area);
}

fn draw_chat_hints(frame: &mut Frame, area: Rect, app: &App, chat: &ChatState) {
    let mut spans: Vec<Span> = vec![
        " ".into(),
        "Tab".bold().cyan(),
        hint(if chat.transcript_focus {
            " message  "
        } else {
            " transcript  "
        }),
        "Enter".bold().cyan(),
        hint(" send  "),
        "Alt+Enter".bold(),
        hint(" newline  "),
        "↑↓".bold(),
        hint(" history  "),
        "PgUp/Dn".bold(),
        hint(" scroll  "),
        "Esc".bold(),
        hint(" back  "),
    ];
    if chat.transcript_focus {
        spans.extend(vec![
            "j/k".bold().cyan(),
            hint(" scroll  "),
            "h/l".bold().cyan(),
            hint(if chat.expand_tool_results {
                " collapse  "
            } else {
                " expand  "
            }),
        ]);
    }
    if let Some(s) = app.sessions.get(&chat.session_id) {
        match s.mode {
            SessionMode::Approval => spans.extend(vec![
                "a".bold().green(),
                hint(" allow  "),
                "d".bold().red(),
                hint(" deny  "),
            ]),
            SessionMode::Question => spans.extend(vec!["1-9".bold(), hint(" answer  ")]),
            _ => {}
        }
    }
    spans.extend(vec!["q".bold(), hint(" quit ")]);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ─── Transcript message blocks ─────────────────────────────────────────

/// Build the full styled transcript as a Vec<Line>. Pulled out so the
/// render path can run it once and stash the result in ChatState's
/// cache — scrolling then just re-slices.
fn build_transcript_lines(chat: &ChatState, inner_width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if chat.transcript.messages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  no transcript yet",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  type below to send your first message — the assistant's",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  reply will land here once Claude Code has written it out",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  to ~/.claude/projects/.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let tool_calls = collect_tool_calls(&chat.transcript.messages);
        for msg in &chat.transcript.messages {
            render_transcript_message(
                msg,
                inner_width,
                chat.expand_tool_results,
                &tool_calls,
                &mut lines,
            );
        }
    }
    lines
}

/// Pre-scan all messages to build a tool_use_id → (tool_name, file_path)
/// map. The tool_result block lands in a later (user-role) message and
/// carries only the tool_use_id, so without this index we can't tell
/// whether a given result is from `Read` or `Bash`.
fn collect_tool_calls(
    messages: &[crate::session::transcript::TranscriptMessage],
) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut out = std::collections::HashMap::new();
    for msg in messages {
        for block in transcript::blocks(&msg.content) {
            if let MsgBlock::ToolUse {
                name,
                input,
                id: Some(id),
            } = block
            {
                let path = input
                    .get("file_path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                out.insert(id.to_string(), (name.to_string(), path));
            }
        }
    }
    out
}

fn render_transcript_message(
    msg: &crate::session::transcript::TranscriptMessage,
    inner_width: usize,
    expand_tool_results: bool,
    tool_calls: &std::collections::HashMap<String, (String, Option<String>)>,
    out: &mut Vec<Line<'static>>,
) {
    // Tool results are tagged onto the *previous* turn visually — they
    // aren't the user's voice. If a "user" message contains only
    // tool_result blocks, skip the [you] prefix entirely.
    let raw_blocks = transcript::blocks(&msg.content);
    let only_tool_results = !raw_blocks.is_empty()
        && raw_blocks
            .iter()
            .all(|b| matches!(b, MsgBlock::ToolResult { .. }));

    let prefix = if only_tool_results {
        None
    } else {
        Some(role_prefix(&msg.role))
    };

    // A tool result is the consequence of the call that came before it.
    // Collapse the blank line we emitted at the end of the previous
    // message so the ↳ sits flush under the ● — they read as one beat.
    // We also indent it as if it were still inside the assistant turn
    // (5-char indent matches `›    `), so ↳ and ● align in the same
    // column.
    if only_tool_results {
        while out
            .last()
            .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .unwrap_or(false)
        {
            out.pop();
        }
    }
    let prefix_len = if only_tool_results {
        5
    } else {
        prefix
            .as_ref()
            .map(|p| p.content.chars().count())
            .unwrap_or(2)
    };
    let indent: String = std::iter::repeat(' ').take(prefix_len).collect();
    let wrap_width = inner_width.saturating_sub(prefix_len).max(20);

    let mut emitted_for_msg = false;

    for block in raw_blocks {
        match block {
            MsgBlock::Text { text } => {
                let trimmed = text.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                render_markdown_text(
                    trimmed,
                    prefix.as_ref(),
                    &indent,
                    wrap_width,
                    !emitted_for_msg,
                    out,
                );
                emitted_for_msg = true;
            }
            MsgBlock::ToolUse { name, input, .. } => {
                let summary = transcript::summarize_tool_input(name, input);
                let head_prefix: Span = if emitted_for_msg {
                    indent.clone().into()
                } else if let Some(p) = prefix.clone() {
                    p
                } else {
                    indent.clone().into()
                };
                let color = tool_color(name);
                let dot = Span::styled(
                    "● ",
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                );
                let tool_name = Span::styled(
                    name.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                );
                let mut head_line = vec![head_prefix, dot, tool_name];
                if !summary.is_empty() {
                    head_line.push("  ".into());
                    head_line.push(Span::styled(
                        first_line_truncated(&summary, wrap_width.saturating_sub(8)),
                        Style::default().fg(Color::Gray),
                    ));
                }
                out.push(Line::from(head_line));
                emitted_for_msg = true;
            }
            MsgBlock::ToolResult {
                content,
                is_error,
                tool_use_id,
            } => {
                let flat = transcript::flatten_tool_result(content);
                let compact_success = compact_tool_success(&flat);
                let color = if is_error {
                    Color::Red
                } else {
                    Color::DarkGray
                };
                let tag = if is_error { "↳ error: " } else { "↳ " };
                let head_prefix: Span = if emitted_for_msg {
                    indent.clone().into()
                } else if let Some(p) = prefix.clone() {
                    p
                } else {
                    indent.clone().into()
                };
                let body_width = wrap_width.saturating_sub(tag.chars().count()).max(20);

                if let Some(summary) = compact_success {
                    out.push(Line::from(vec![
                        head_prefix,
                        Span::styled(
                            "✓ ",
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(summary, Style::default().fg(Color::Green)),
                    ]));
                    emitted_for_msg = true;
                    continue;
                }

                // If this result is from `Read`, look up the file path so
                // we can syntax-highlight the body. Skipped on errors so
                // tracebacks stay red.
                let read_lang: &'static str = if is_error {
                    ""
                } else {
                    tool_use_id
                        .and_then(|id| tool_calls.get(id))
                        .filter(|(name, _)| name == "Read")
                        .and_then(|(_, path)| path.as_deref())
                        .map(syntax::language_from_path)
                        .unwrap_or("")
                };

                // One stateful highlighter for the whole result so
                // multi-line strings / block comments stay correctly
                // tokenized across line breaks.
                let mut highlighter = syntax::Highlighter::for_language(read_lang);

                let mut shown = 0usize;
                let mut total_lines = 0usize;
                let mut first_block = true;
                for raw_line in flat.lines() {
                    total_lines += 1;
                    if !expand_tool_results && shown >= 4 {
                        // Drive the highlighter on hidden lines so its
                        // state stays in sync with the visible portion.
                        if highlighter.is_active() {
                            let (_, body) = split_cat_n_line(raw_line);
                            let _ = highlighter.highlight(&body, Style::default());
                        }
                        continue;
                    }
                    let (gutter, body) = if highlighter.is_active() {
                        split_cat_n_line(raw_line)
                    } else {
                        (String::new(), raw_line.to_string())
                    };
                    let highlighted = if highlighter.is_active() {
                        highlighter.highlight(&body, Style::default().fg(Color::Gray))
                    } else {
                        vec![Span::styled(body.clone(), Style::default().fg(color))]
                    };
                    let chunk_width = body_width.saturating_sub(gutter.chars().count()).max(1);
                    let wrapped = wrap_spans(&highlighted, chunk_width);
                    for (i, chunk_spans) in wrapped.into_iter().enumerate() {
                        let prefix_span = if first_block {
                            head_prefix.clone()
                        } else {
                            indent.clone().into()
                        };
                        let tag_span: Span<'static> = if first_block {
                            Span::styled(tag.to_string(), Style::default().fg(color))
                        } else {
                            let pad: String =
                                std::iter::repeat(' ').take(tag.chars().count()).collect();
                            Span::styled(pad, Style::default().fg(color))
                        };
                        let mut spans: Vec<Span<'static>> = vec![prefix_span, tag_span];
                        if i == 0 && !gutter.is_empty() {
                            spans.push(Span::styled(
                                gutter.clone(),
                                Style::default().fg(Color::DarkGray),
                            ));
                        } else if !gutter.is_empty() {
                            spans.push(" ".repeat(gutter.chars().count()).into());
                        }
                        spans.extend(chunk_spans);
                        out.push(Line::from(spans));
                        first_block = false;
                    }
                    shown += 1;
                }
                if total_lines == 0 {
                    if is_error {
                        out.push(Line::from(vec![
                            head_prefix,
                            Span::styled(format!("{tag}(empty)"), Style::default().fg(color)),
                        ]));
                    }
                } else if total_lines > shown {
                    let pad: String = std::iter::repeat(' ').take(tag.chars().count()).collect();
                    out.push(Line::from(vec![
                        indent.clone().into(),
                        Span::styled(pad, Style::default().fg(color)),
                        Span::styled(
                            format!(
                                "+{} more line{}",
                                total_lines - shown,
                                if total_lines - shown == 1 { "" } else { "s" }
                            ),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                emitted_for_msg = true;
            }
            MsgBlock::Thinking { .. } => {}
        }
    }

    if emitted_for_msg {
        out.push(Line::from(""));
    }
}

fn role_prefix(role: &str) -> Span<'static> {
    match role {
        "user" => Span::styled(
            "you  ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        "assistant" => Span::styled(
            "›    ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        other => Span::styled(format!("{other}  "), Style::default().fg(Color::Gray)),
    }
}

// ─── Small helpers ─────────────────────────────────────────────────────

/// Read tool output is formatted like `cat -n`: each line begins with a
/// right-aligned line number, a tab, then the file content. We split
/// those apart so we can render the line number in a dim gutter and
/// syntax-highlight the body separately. Lines that don't match the
/// pattern (e.g. truncation banners) are returned with an empty gutter.
fn split_cat_n_line(line: &str) -> (String, String) {
    let Some(tab_idx) = line.find('\t') else {
        return (String::new(), line.to_string());
    };
    let prefix = &line[..tab_idx];
    let trimmed = prefix.trim_start();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return (String::new(), line.to_string());
    }
    // Keep one trailing space so the body doesn't butt up against the digits.
    let gutter = format!("{prefix} ");
    let body = line[tab_idx + 1..].to_string();
    (gutter, body)
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

fn render_markdown_text(
    text: &str,
    prefix: Option<&Span<'static>>,
    indent: &str,
    wrap_width: usize,
    first_line_gets_prefix: bool,
    out: &mut Vec<Line<'static>>,
) {
    let mut first = first_line_gets_prefix;
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_highlighter: Option<syntax::Highlighter> = None;

    // Closure-ish helper: emit `spans` on a new line, taking care of
    // whether this is the first line of the message (which needs the
    // role prefix) vs a continuation (which needs the indent pad).
    let push_with_prefix =
        |first: &mut bool, mut leading: Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if *first {
                if let Some(p) = prefix {
                    spans.push(p.clone());
                }
                *first = false;
            } else {
                spans.push(indent.to_string().into());
            }
            spans.append(&mut leading);
            out.push(Line::from(spans));
        };

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            if in_code {
                in_code = false;
                code_lang.clear();
                code_highlighter = None;
                continue;
            }
            in_code = true;
            code_lang = trimmed[3..].trim().to_string();
            code_highlighter = Some(syntax::Highlighter::for_language(&code_lang));
            let label = if code_lang.is_empty() {
                "code".to_string()
            } else {
                code_lang.clone()
            };
            let header = vec![
                Span::styled("┌ ".to_string(), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    label,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ];
            push_with_prefix(&mut first, header, out);
            continue;
        }

        if line.trim().is_empty() {
            out.push(Line::from(""));
            first = false;
            continue;
        }

        if in_code {
            let body_width = wrap_width.saturating_sub(2).max(20);
            let base = Style::default().fg(Color::Gray);
            // Drive the highlighter once per source line so multi-line
            // strings and block comments stay correctly tokenized.
            let highlighted = match code_highlighter.as_mut() {
                Some(h) if h.is_active() => h.highlight(line, base),
                _ => vec![Span::styled(line.to_string(), base)],
            };
            for chunk_spans in wrap_spans(&highlighted, body_width) {
                let mut leading: Vec<Span<'static>> = vec![Span::styled(
                    "│ ".to_string(),
                    Style::default().fg(Color::DarkGray),
                )];
                leading.extend(chunk_spans);
                push_with_prefix(&mut first, leading, out);
            }
            continue;
        }

        let (marker, body, style) = markdown_line_parts(trimmed);
        let marker_width = marker.chars().count();
        let body_width = wrap_width.saturating_sub(marker_width).max(20);
        let chunks = wrap_str(&body, body_width);

        for (i, chunk) in chunks.into_iter().enumerate() {
            let mut leading: Vec<Span<'static>> = Vec::new();
            if i == 0 {
                if !marker.is_empty() {
                    leading.push(Span::styled(marker.clone(), style));
                }
            } else if marker_width > 0 {
                leading.push(" ".repeat(marker_width).into());
            }

            leading.extend(inline_markdown_spans(&chunk, style));
            push_with_prefix(&mut first, leading, out);
        }
    }
}

fn markdown_line_parts(line: &str) -> (String, String, Style) {
    let heading_level = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&heading_level)
        && line
            .chars()
            .nth(heading_level)
            .is_some_and(char::is_whitespace)
    {
        return (
            String::new(),
            line[heading_level..].trim_start().to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }

    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return (
                "• ".to_string(),
                rest.to_string(),
                Style::default().fg(Color::White),
            );
        }
    }

    // Numbered list "1. ", "12. ", etc.
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after = &line[digits.len()..];
        if let Some(rest) = after.strip_prefix(". ") {
            return (
                format!("{digits}. "),
                rest.to_string(),
                Style::default().fg(Color::White),
            );
        }
    }

    if let Some(rest) = line.strip_prefix("> ") {
        return (
            "│ ".to_string(),
            rest.to_string(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        );
    }

    (
        String::new(),
        line.to_string(),
        Style::default().fg(Color::White),
    )
}

fn inline_markdown_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut i = 0;

    while i < chars.len() {
        // Inline code: `...`
        if chars[i] == '`' {
            if let Some(rel_end) = chars[i + 1..].iter().position(|c| *c == '`') {
                flush_inline(&mut buf, &mut spans, base, bold, italic);
                let code: String = chars[i + 1..i + 1 + rel_end].iter().collect();
                spans.push(Span::styled(code, Style::default().fg(Color::Green)));
                i = i + rel_end + 2;
                continue;
            }
        }

        // **bold**
        if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
            flush_inline(&mut buf, &mut spans, base, bold, italic);
            bold = !bold;
            i += 2;
            continue;
        }

        // [text](url) — render text only, dim url
        if chars[i] == '[' {
            if let Some(text_rel) = chars[i + 1..].iter().position(|c| *c == ']') {
                let text_end = i + 1 + text_rel;
                if text_end + 1 < chars.len() && chars[text_end + 1] == '(' {
                    if let Some(url_rel) = chars[text_end + 2..].iter().position(|c| *c == ')') {
                        let link_text: String = chars[i + 1..text_end].iter().collect();
                        flush_inline(&mut buf, &mut spans, base, bold, italic);
                        spans.push(Span::styled(
                            link_text,
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::UNDERLINED),
                        ));
                        i = text_end + 2 + url_rel + 1;
                        continue;
                    }
                }
            }
        }

        // *italic* / _italic_ — guarded so we don't toggle on snake_case
        // identifiers or asterisks inside punctuation. Open requires
        // whitespace/start-of-line before and non-space after; close
        // requires non-space before and whitespace/EOL/punctuation after.
        if (chars[i] == '*' || chars[i] == '_') && !is_double_marker(&chars, i) {
            let prev = if i == 0 { None } else { Some(chars[i - 1]) };
            let next = chars.get(i + 1).copied();
            let opens_ok = prev.map_or(true, |c| !c.is_alphanumeric())
                && next.is_some_and(|c| !c.is_whitespace());
            let closes_ok = prev.is_some_and(|c| !c.is_whitespace())
                && next.map_or(true, |c| !c.is_alphanumeric());
            if (!italic && opens_ok) || (italic && closes_ok) {
                flush_inline(&mut buf, &mut spans, base, bold, italic);
                italic = !italic;
                i += 1;
                continue;
            }
        }

        buf.push(chars[i]);
        i += 1;
    }

    flush_inline(&mut buf, &mut spans, base, bold, italic);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn flush_inline(
    buf: &mut String,
    out: &mut Vec<Span<'static>>,
    base: Style,
    bold: bool,
    italic: bool,
) {
    if buf.is_empty() {
        return;
    }
    let mut style = base;
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    out.push(Span::styled(std::mem::take(buf), style));
}

fn is_double_marker(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    (i + 1 < chars.len() && chars[i + 1] == c) || (i > 0 && chars[i - 1] == c)
}

fn tool_color(name: &str) -> Color {
    match name {
        "Bash" => Color::Cyan,
        "Read" | "Glob" | "Grep" => Color::Blue,
        "Write" | "Edit" | "MultiEdit" => Color::Green,
        "WebFetch" | "WebSearch" => Color::Magenta,
        _ => Color::Yellow,
    }
}

fn compact_tool_success(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let prefix = "The file ";
    let suffix = " has been updated successfully.";
    let path = trimmed.strip_prefix(prefix)?.split(suffix).next()?;
    if path == trimmed {
        return None;
    }
    let file = path
        .rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path);
    Some(format!("updated {file}"))
}

/// Wrap a styled run of spans to `width` characters, preserving each
/// character's style. Word-aware (breaks at spaces), falls back to
/// char-splitting for words longer than `width`.
fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans.to_vec()];
    }
    // Flatten to (style, char) pairs so we can walk word-by-word
    // regardless of where span boundaries fall.
    let chars: Vec<(Style, char)> = spans
        .iter()
        .flat_map(|s| {
            let style = s.style;
            s.content.chars().map(move |c| (style, c))
        })
        .collect();

    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<(Style, char)> = Vec::new();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        // Read one "word" via split_inclusive(' ') semantics: chars up
        // to and including the next space.
        let mut j = i;
        while j < n && chars[j].1 != ' ' {
            j += 1;
        }
        if j < n {
            j += 1;
        }
        let word_len = j - i;
        if current.len() + word_len > width && !current.is_empty() {
            out.push(emit_run(&current));
            current.clear();
        }
        if word_len > width {
            // A single token is wider than the line — break char-wise.
            for &p in &chars[i..j] {
                if current.len() >= width {
                    out.push(emit_run(&current));
                    current.clear();
                }
                current.push(p);
            }
        } else {
            current.extend_from_slice(&chars[i..j]);
        }
        i = j;
    }

    if !current.is_empty() {
        out.push(emit_run(&current));
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

/// Coalesce a run of (style, char) pairs into a minimal Vec<Span>.
fn emit_run(run: &[(Style, char)]) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut i = 0;
    while i < run.len() {
        let style = run[i].0;
        let mut j = i;
        while j < run.len() && run[j].0 == style {
            j += 1;
        }
        let s: String = run[i..j].iter().map(|p| p.1).collect();
        out.push(Span::styled(s, style));
        i = j;
    }
    out
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
    let (text, style) = badge_token(mode);
    Span::styled(text.to_string(), style)
}

/// Mode badge padded to BADGE_WIDTH so dashboard columns line up.
fn mode_badge_padded(mode: SessionMode) -> Span<'static> {
    let (text, style) = badge_token(mode);
    let pad = BADGE_WIDTH.saturating_sub(text.chars().count());
    let padded: String = format!("{}{}", text, " ".repeat(pad));
    Span::styled(padded, style)
}

fn badge_token(mode: SessionMode) -> (&'static str, Style) {
    match mode {
        SessionMode::Unknown => ("unknown", Style::default().fg(Color::DarkGray)),
        SessionMode::Input => ("input", Style::default().fg(Color::Cyan)),
        SessionMode::Responding => (
            "responding",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        SessionMode::Approval => (
            "APPROVAL",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        SessionMode::Question => (
            "QUESTION",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        SessionMode::Stopped => ("stopped", Style::default().fg(Color::DarkGray)),
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
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn label(text: &str) -> Span<'static> {
    // Pad to 9 chars so all "key:" labels line up vertically.
    let padded = format!("{:<9} ", text);
    Span::styled(padded, Style::default().fg(Color::DarkGray))
}

fn hint(text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(HINT_LABEL))
}

fn kv(k: &str, v: &str) -> Line<'static> {
    Line::from(vec![label(k), v.to_string().into()])
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
            transcript_focus: false,
            expand_tool_results: false,
            scroll_offset: 0,
            last_seen_mode: SessionMode::Input,
            render_cache: std::cell::RefCell::new(None),
        });
        app
    }

    #[test]
    fn empty_chat_shows_friendly_empty_state() {
        let app = build_chat_app(vec![]);
        let s = snapshot(&app, 80, 24);
        assert!(s.contains("transcript"), "transcript title missing\n{s}");
        assert!(s.contains("message"), "message title missing\n{s}");
        assert!(
            s.contains("no transcript yet"),
            "friendly empty state missing\n{s}"
        );
        assert!(
            s.contains("type a message"),
            "input placeholder missing\n{s}"
        );
        assert!(
            s.contains("Enter") && s.contains("send"),
            "hints missing\n{s}"
        );
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
        assert!(s.contains("you"), "user prefix missing\n{s}");
        assert!(s.contains("hello there"), "user msg missing\n{s}");
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
    fn chat_renders_tool_result_without_user_prefix() {
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
        // The user-prefix should NOT appear next to the tool result. We
        // assert the arrow comes before any "you" prefix on its line.
        let result_line = s.lines().find(|l| l.contains("↳")).unwrap_or("");
        assert!(
            !result_line.trim_start().starts_with("you"),
            "tool result should not be labeled [you]: {result_line}",
        );
    }

    #[test]
    fn pending_approval_banner_appears_with_yellow_treatment() {
        let mut app = build_chat_app(vec![]);
        if let Some(s) = app.sessions.get_mut("test-session-id") {
            s.mode = SessionMode::Approval;
            s.pending = Some(Pending::Approval {
                tool: Some("Bash".into()),
                summary: Some("rm -rf /tmp/x".into()),
                raw: json!({}),
            });
        }
        let s = snapshot(&app, 80, 24);
        assert!(s.contains("approval needed"), "banner title missing\n{s}");
        assert!(s.contains("Bash"), "tool not shown\n{s}");
        assert!(s.contains("rm -rf /tmp/x"), "summary not shown\n{s}");
        assert!(
            s.contains("[a]") && s.contains("[d]"),
            "action keys missing\n{s}"
        );
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
        let s = snapshot(&app, 80, 24);
        assert!(s.contains("Pick one?"), "question text missing\n{s}");
        assert!(
            s.contains("[1]") && s.contains("alpha"),
            "option 1 missing\n{s}"
        );
        assert!(
            s.contains("[2]") && s.contains("beta"),
            "option 2 missing\n{s}"
        );
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
        assert!(
            !s.contains("type a message…"),
            "placeholder still shown\n{s}"
        );
    }

    #[test]
    fn dashboard_shows_session_with_unbracketed_mode_badge() {
        let mut app = build_chat_app(vec![]);
        app.view = View::Dashboard;
        let s = snapshot(&app, 100, 20);
        assert!(s.contains("test-ses"), "session id short form missing\n{s}");
        assert!(s.contains("input"), "mode badge missing\n{s}");
        assert!(
            !s.contains("[ input"),
            "old bracketed badge still rendered\n{s}"
        );
        assert!(s.contains("/tmp/x"), "cwd missing\n{s}");
    }

    #[test]
    fn empty_dashboard_shows_onboarding_hint() {
        let app = App::new("http://test".into());
        let s = snapshot(&app, 100, 24);
        assert!(
            s.contains("no sessions yet"),
            "onboarding header missing\n{s}"
        );
        assert!(s.contains("claudemon serve"), "onboarding cmd missing\n{s}");
        assert!(s.contains("claudemon wrap"), "wrap cmd missing\n{s}");
    }

    #[test]
    fn dashboard_hints_hide_approve_when_no_pending() {
        let mut app = build_chat_app(vec![]);
        app.view = View::Dashboard;
        let s = snapshot(&app, 100, 20);
        // No session is in Approval mode → "allow" / "deny" hints suppressed.
        assert!(
            !s.contains(" allow"),
            "approve hint shown when nothing pending\n{s}"
        );
        assert!(
            !s.contains(" deny"),
            "deny hint shown when nothing pending\n{s}"
        );
    }

    #[test]
    fn dashboard_hints_show_approve_when_approval_pending() {
        let mut app = build_chat_app(vec![]);
        app.view = View::Dashboard;
        if let Some(state) = app.sessions.get_mut("test-session-id") {
            state.mode = SessionMode::Approval;
        }
        let s = snapshot(&app, 100, 20);
        assert!(
            s.contains(" allow"),
            "approve hint missing when pending\n{s}"
        );
        assert!(s.contains(" deny"), "deny hint missing when pending\n{s}");
    }

    #[test]
    fn inline_markdown_strips_bold_and_code_markers() {
        let spans = inline_markdown_spans(
            "use **bold** and `code` here",
            Style::default().fg(Color::White),
        );
        let flat: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "use bold and code here");

        let bold = spans
            .iter()
            .find(|s| s.content == "bold")
            .expect("bold span");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));

        let code = spans
            .iter()
            .find(|s| s.content == "code")
            .expect("code span");
        assert_eq!(code.style.fg, Some(Color::Green));
    }

    #[test]
    fn inline_markdown_renders_link_text_and_drops_url() {
        let spans = inline_markdown_spans(
            "see [the docs](https://example.com/x) please",
            Style::default().fg(Color::White),
        );
        let flat: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "see the docs please");
        let link = spans
            .iter()
            .find(|s| s.content == "the docs")
            .expect("link span");
        assert_eq!(link.style.fg, Some(Color::Cyan));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn inline_markdown_italic_strips_marker() {
        let spans = inline_markdown_spans(
            "this is *important* really",
            Style::default().fg(Color::White),
        );
        let flat: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "this is important really");
        let it = spans
            .iter()
            .find(|s| s.content == "important")
            .expect("italic span");
        assert!(it.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_markdown_leaves_snake_case_alone() {
        // _ inside identifiers must not be treated as italic markers.
        let spans = inline_markdown_spans(
            "call foo_bar_baz here",
            Style::default().fg(Color::White),
        );
        let flat: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "call foo_bar_baz here");
        assert!(spans.iter().all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)));
    }

    #[test]
    fn numbered_list_marker_recognized() {
        let (marker, body, _) = markdown_line_parts("1. first item");
        assert_eq!(marker, "1. ");
        assert_eq!(body, "first item");

        let (marker, body, _) = markdown_line_parts("12. twelfth");
        assert_eq!(marker, "12. ");
        assert_eq!(body, "twelfth");
    }

    #[test]
    fn chat_renders_fenced_rust_code_with_language_header() {
        let body = "Here:\n```rust\nlet x = 1;\n```\nDone.";
        let messages = vec![TranscriptMessage {
            role: "assistant".into(),
            content: json!([{ "type": "text", "text": body }]),
            raw: json!({}),
        }];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 80, 24);
        // Fence char ``` should not appear raw.
        assert!(!s.contains("```"), "raw fence visible:\n{s}");
        // Language tag bubbles up as the header label.
        assert!(s.contains("rust"), "language label missing:\n{s}");
        // Code body is rendered with the gutter prefix and keywords.
        assert!(s.contains("│"), "code gutter missing:\n{s}");
        assert!(s.contains("let x = 1;"), "code content missing:\n{s}");
    }

    #[test]
    fn split_cat_n_line_peels_line_number() {
        let (gutter, body) = split_cat_n_line("   42\t    let x = 1;");
        assert_eq!(gutter, "   42 ");
        assert_eq!(body, "    let x = 1;");
    }

    #[test]
    fn split_cat_n_line_leaves_unnumbered_lines_alone() {
        let (gutter, body) = split_cat_n_line("+12 more lines");
        assert!(gutter.is_empty());
        assert_eq!(body, "+12 more lines");
    }

    #[test]
    fn read_tool_result_highlights_body_by_extension() {
        let messages = vec![
            TranscriptMessage {
                role: "assistant".into(),
                content: json!([
                    { "type": "tool_use", "id": "r1", "name": "Read",
                      "input": { "file_path": "/proj/src/main.rs" } }
                ]),
                raw: json!({}),
            },
            TranscriptMessage {
                role: "user".into(),
                content: json!([
                    { "type": "tool_result", "tool_use_id": "r1",
                      "content": "   1\tfn main() {\n   2\t    let x = 1;\n   3\t}" }
                ]),
                raw: json!({}),
            },
        ];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 100, 24);
        // The line-number gutter should remain visible.
        assert!(s.contains("1 fn main"), "gutter+body not rendered:\n{s}");
        // The raw tab should not be in the buffer.
        assert!(!s.contains("\t"), "raw tab leaked:\n{s}");
    }

    #[test]
    fn chat_strips_bold_markers_from_assistant_text() {
        let messages = vec![TranscriptMessage {
            role: "assistant".into(),
            content: json!([{ "type": "text", "text": "say **hi** to the world" }]),
            raw: json!({}),
        }];
        let app = build_chat_app(messages);
        let s = snapshot(&app, 80, 20);
        assert!(s.contains("hi"), "bold body missing:\n{s}");
        assert!(!s.contains("**"), "raw ** markers leaked:\n{s}");
    }
}
