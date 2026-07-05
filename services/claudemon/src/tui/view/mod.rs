//! ratatui rendering for the watch TUI. Pure function of `&App`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use time::OffsetDateTime;

use crate::session::SessionMode;

use super::app::{App, View};
use super::syntax;

mod chat;
mod dashboard;

/// Width allocated to mode badges so columns line up across rows.
/// Widest token is "responding" (10 chars).
const BADGE_WIDTH: usize = 10;
/// Color used for the active/focused border (input box, selected items).
const FOCUS: Color = Color::Cyan;
/// Footer hint labels need to remain readable on dark terminal themes.
const HINT_LABEL: Color = Color::Gray;

pub fn render(frame: &mut Frame, app: &App) {
    match &app.view {
        View::Dashboard => dashboard::render_dashboard(frame, app),
        View::Chat(chat) => chat::render_chat(frame, app, chat),
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────────

fn draw_toast(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(t) = app.current_toast() {
        frame.render_widget(
            Paragraph::new(format!(" {t}")).style(Style::default().fg(Color::Yellow)),
            area,
        );
    }
}

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

        if let Some(after_fence) = trimmed.strip_prefix("```") {
            if in_code {
                in_code = false;
                code_lang.clear();
                code_highlighter = None;
                continue;
            }
            in_code = true;
            code_lang = after_fence.trim().to_string();
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
            let opens_ok = prev.is_none_or(|c| !c.is_alphanumeric())
                && next.is_some_and(|c| !c.is_whitespace());
            let closes_ok = prev.is_some_and(|c| !c.is_whitespace())
                && next.is_none_or(|c| !c.is_alphanumeric());
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
    id.char_indices()
        .nth(8)
        .map_or_else(|| id.to_string(), |(i, _)| format!("{}…", &id[..i]))
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
            transcript_path: None,
            status_line: None,
            provider: "claude".into(),
            plan: None,
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
            s.pending = Some(crate::session::state::Pending::Approval {
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
            state.pending = Some(crate::session::state::Pending::Question {
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
        let spans =
            inline_markdown_spans("call foo_bar_baz here", Style::default().fg(Color::White));
        let flat: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(flat, "call foo_bar_baz here");
        assert!(spans
            .iter()
            .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)));
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
