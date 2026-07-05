//! Chat view rendering — transcript, input box, pending banners, and hints.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::session::{
    state::Pending,
    transcript::{self, Block as MsgBlock},
    SessionMode,
};

use crate::tui::app::{App, ChatState};
use crate::tui::syntax;

use super::render_markdown_text;
use super::{
    compact_tool_success, draw_toast, first_line_truncated, hint, mode_badge, short_id,
    split_cat_n_line, tool_color, wrap_spans, FOCUS,
};

pub(super) fn render_chat(frame: &mut Frame, app: &App, chat: &ChatState) {
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

    let key = super::super::app::TranscriptRenderKey {
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
        let needs_rebuild = cache.as_ref().is_none_or(|c| c.key != key);
        if needs_rebuild {
            let lines = build_transcript_lines(chat, inner_width);
            *cache = Some(super::super::app::TranscriptRenderCache { key, lines });
        }
    }

    let cache = chat.render_cache.borrow();
    let lines: &[Line<'static>] = cache.as_ref().map(|c| c.lines.as_slice()).unwrap_or(&[]);

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
        format!(
            " transcript · {} ",
            path.split('/').next_back().unwrap_or(path)
        )
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
    let indent: String = std::iter::repeat_n(' ', prefix_len).collect();
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
                                std::iter::repeat_n(' ', tag.chars().count()).collect();
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
                    let pad: String = std::iter::repeat_n(' ', tag.chars().count()).collect();
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
