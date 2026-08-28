//! The conversation: the transcript (committed + live tail), the tool-run rows
//! and edit diffs folded into it, and the composer beneath.

use super::*;

pub(super) fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
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
    let commits = app.commits();
    let session_id = app.chat_session_id();
    let stale = app
        .transcript_cache
        .as_ref()
        .is_none_or(|c| c.width != inner_w || c.commits != commits || c.session_id != session_id);
    if stale {
        let lines = transcript_committed_lines(app, inner_w);
        app.transcript_cache = Some(crate::app::TranscriptCache {
            width: inner_w,
            session_id,
            commits,
            lines,
        });
    }
    let cache = app.transcript_cache.as_ref().expect("cache just ensured");
    // The live tail is rebuilt every frame by design — it's the part that
    // actually changes per token, and it is a handful of lines, not the whole
    // conversation.
    let tail = transcript_tail_lines(app, inner_w);
    let head_len = cache.lines.len();
    let total = head_len + tail.len();
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
    let mut visible: Vec<Line> = Vec::with_capacity(viewport);
    if scroll < head_len {
        visible.extend_from_slice(&cache.lines[scroll..(scroll + viewport).min(head_len)]);
    }
    if visible.len() < viewport {
        let start = scroll.saturating_sub(head_len);
        if start < tail.len() {
            let end = (start + viewport - visible.len()).min(tail.len());
            visible.extend_from_slice(&tail[start..end]);
        }
    }
    if total == 0 {
        visible.push(Line::from(Span::styled(
            "no messages yet",
            Style::default().fg(app.theme.dim),
        )));
    }
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

pub(super) fn render_composer(f: &mut Frame, area: Rect, app: &App, agent: &Option<Agent>) {
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
    // The plan's in-flight step rides the composer's bottom border: it is the one
    // line always on screen in a chat, and "what is it doing right now" is the
    // question you ask while waiting for a reply.
    let block = match agent
        .as_ref()
        .and_then(|a| app.plan_for(&a.session_id))
        .and_then(|p| {
            plan_lines(t, p, area.width, true)
                .into_iter()
                .next()
                .map(|l| l.spans)
        }) {
        Some(spans) => block.title_bottom(Line::from(spans)),
        None => block,
    };
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
pub(super) struct ToolRow {
    pub(super) name: String,
    pub(super) summary: String,
    pub(super) result: Option<String>,
    /// Edit/MultiEdit `(old, new)` pairs — rendered as a compact diff and kept
    /// visible even when the run collapses to a summary line.
    pub(super) edits: Vec<(String, String)>,
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
pub(super) fn transcript_committed_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = &app.theme;
    let w = width.max(10);
    let mut out: Vec<Line> = Vec::new();
    // Assistant turn headers name the actual backend (claude / codex / …).
    let agent_label = agent_label(app);
    let mut run: Vec<ToolRow> = Vec::new();
    for turn in app.turns() {
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
                    ..
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

        push_role_label(&mut out, t, turn.role, &agent_label);
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
                    ..
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
    out
}

/// Assistant turn headers name the actual backend (claude / codex / …).
pub(super) fn agent_label(app: &App) -> String {
    app.chat_session_id()
        .map(|sid| app.provider_for(&sid))
        .unwrap_or_else(|| "claude".to_string())
}

/// The uncommitted tail of the transcript, rebuilt on every draw.
///
/// Split out from the committed turns because this is the only part that moves
/// per streamed token. Folding it into the memo meant each token invalidated
/// the wrapped render of the entire conversation, so the redraw cost scaled
/// with transcript length at the delta rate — at a few hundred KB the loop
/// spends all its time rebuilding and the backlog grows.
pub(super) fn transcript_tail_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let t = &app.theme;
    let w = width.max(10);
    let mut out: Vec<Line> = Vec::new();
    let agent_label = agent_label(app);

    // Assistant text still arriving, not yet a committed turn. Rendered through
    // the same markdown pass as a finished message, so a half-written list or
    // code fence looks like itself while it streams rather than snapping into
    // shape at the end.
    if let Some(partial) = app.pending_text() {
        if !matches!(app.turns().last().map(|t| t.role), Some(Role::Assistant)) {
            push_role_label(&mut out, t, Role::Assistant, &agent_label);
        }
        out.extend(crate::render::markdown_lines(partial, t, w));
        out.push(Line::raw(""));
    }

    // Optimistic echo: the just-sent message, until a refold carries it.
    if let Some(echo) = app.pending_echo.as_deref() {
        push_role_label(&mut out, t, Role::User, &agent_label);
        out.extend(crate::render::markdown_lines(echo, t, w));
        out.push(Line::from(Span::styled(
            "…sending",
            Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
        )));
        out.push(Line::raw(""));
    }
    out
}

/// The `▍ you` / `▍ <agent>` turn header, where `<agent>` is the session's
/// actual provider (claude / codex / copilot / opencode / pi).
pub(super) fn push_role_label(out: &mut Vec<Line<'static>>, t: &Theme, role: Role, agent: &str) {
    let (label, color) = match role {
        Role::User => ("▍ you".to_string(), t.accent),
        Role::Assistant => (format!("▍ {agent}"), t.ok),
    };
    out.push(Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
}

/// The dim `⚙ name · summary` line for one tool call.
pub(super) fn push_tool_row(
    out: &mut Vec<Line<'static>>,
    t: &Theme,
    w: usize,
    name: &str,
    summary: &str,
) {
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
pub(super) fn push_tool_result(out: &mut Vec<Line<'static>>, res: &str, t: &Theme, w: usize) {
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
pub(super) fn push_edit_diff(
    out: &mut Vec<Line<'static>>,
    t: &Theme,
    w: usize,
    edits: &[(String, String)],
) {
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
        // Display-width truncation: render_chat's Paragraph has no .wrap(),
        // so a row measured in chars (wide glyphs = 2 columns) would clip at
        // the pane border and hide the `…` marker entirely.
        let line = crate::render::truncate_width(&format!("  {sign} {text}"), w);
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
pub(super) fn flush_tool_run(
    out: &mut Vec<Line<'static>>,
    run: &mut Vec<ToolRow>,
    t: &Theme,
    w: usize,
) {
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
pub(super) fn summarize_tool_names(names: &[&str]) -> String {
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

#[cfg(test)]
mod tests {
    use super::super::testutil::line_texts;
    use super::*;

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
    fn edit_diff_truncates_by_display_width_not_char_count() {
        // The chat Paragraph has no .wrap(): a row wider than the pane clips
        // at the border. Wide glyphs count 2 columns, so char-count truncation
        // would leave this CJK row ~2x the pane width with the '…' invisible.
        let t = Theme::default();
        let old = "宽".repeat(30); // 60 columns of content
        let mut out = Vec::new();
        push_edit_diff(&mut out, &t, 20, &[(old, "x".to_string())]);
        let texts = line_texts(&out);
        assert!(
            crate::render::wrap::display_width(&texts[0]) <= 20,
            "the - row fits the pane: {:?}",
            texts[0]
        );
        assert!(texts[0].ends_with('…'), "the truncation marker is visible");
        assert_eq!(texts[1], "  + x", "short rows pass through untouched");
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
    fn role_label_names_provider_for_assistant_only() {
        // The assistant header names the session's provider; the user header
        // (also the optimistic-echo path) stays a fixed "▍ you" regardless.
        let t = Theme::default();
        let mut out = Vec::new();
        push_role_label(&mut out, &t, Role::Assistant, "codex");
        push_role_label(&mut out, &t, Role::User, "codex");
        let texts = line_texts(&out);
        assert_eq!(texts, vec!["▍ codex", "▍ you"]);
        assert_eq!(out[0].spans[0].style.fg, Some(t.ok), "assistant in ok");
        assert!(out[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(out[1].spans[0].style.fg, Some(t.accent), "user in accent");
    }

    /// The full transcript render (committed + live tail), as the draw path
    /// composes it. Test-only: production slices the viewport out of the two
    /// halves separately so the committed one stays memoized across tokens.
    fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
        let mut out = transcript_committed_lines(app, width);
        out.extend(transcript_tail_lines(app, width));
        out
    }

    #[test]
    fn transcript_agent_label_names_the_session_provider() {
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
        // With no chat session there is no fold to read a transcript from, so the
        // label fallback shows on the one thing that renders regardless: the
        // optimistic echo of a message the user just sent.
        app.pending_echo = Some("hi".into());
        let texts = line_texts(&transcript_lines(&app, 40));
        assert!(texts.iter().any(|l| l == "▍ you"), "{texts:?}");

        // A codex session open (wire provider + workspace tab) → "▍ codex".
        let codex: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "mode": "responding", "provider": "codex"
        }))
        .unwrap();
        app.all_agents = vec![codex];
        app.view = View::Agent { id: "s1".into() };
        app.workspaces.insert(
            "s1".into(),
            crate::app::Workspace {
                tabs: vec![crate::app::Tab {
                    title: "codex".into(),
                    session_id: "s1".into(),
                    kind: TabKind::Claude,
                }],
                active: 0,
            },
        );
        app.pending_echo = None;
        app.seed_fold(
            "s1",
            &serde_json::json!({
                "items": [{ "kind": "assistant_text", "text": "hi" }]
            }),
        );
        let texts = line_texts(&transcript_lines(&app, 40));
        assert!(texts.iter().any(|l| l == "▍ codex"), "{texts:?}");
        assert!(!texts.iter().any(|l| l == "▍ claude"), "{texts:?}");
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
}
