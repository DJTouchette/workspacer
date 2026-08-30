//! The persistent frame: the one-row header, the tab bar, and the footer with
//! its mode chip. Drawn every frame regardless of which view is up.

use super::*;

pub(super) fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let (dot, dot_color, status) = if app.connected {
        ("●", t.ok, "connected")
    } else {
        ("●", t.bad, "reconnecting…")
    };
    let mut spans = vec![
        Span::styled(
            " workspacer ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
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

pub(super) fn render_tab_bar(f: &mut Frame, area: Rect, app: &App) {
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
    spans.push(Span::styled(
        "  T:term  [ ]:tabs  w:close",
        Style::default().fg(t.dim),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── dashboard ──────────────────────────────────────────────────────────────────

pub(super) fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    // The `:` command line takes over the footer while it's open.
    if let Some(cmd) = &app.cmdline {
        let chip = Span::styled(
            " CMD ",
            Style::default()
                .bg(app.theme.accent)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                chip,
                Span::styled(format!(" :{cmd}▏"), Style::default().fg(app.theme.fg)),
            ])),
            area,
        );
        return;
    }
    let in_agent = matches!(app.view, View::Agent { .. });
    let on_shell = matches!(app.active_tab().map(|t| t.kind), Some(TabKind::Shell));
    let hint = if app.notes_view.as_ref().is_some_and(|n| n.editing) {
        "type notes · enter newline · esc save"
    } else if app.notes_view.is_some() {
        "i edit · j/k scroll · esc close"
    } else if app.rename.is_some() {
        "type a name · enter save · esc cancel"
    } else if app.spawn_form.is_some() {
        "type path · tab complete · ↑↓ profile · enter spawn · esc cancel"
    } else if app.term_attached() {
        "● attached — keys go to Claude · Ctrl-] to detach"
    } else if app.filter_editing {
        "type to filter · enter keep · esc clear"
    } else if !in_agent {
        "j/k move · / filter · enter open · ^K palette · c new · m attention · ? help · q quit"
    } else if app.insert_mode {
        "enter send · esc normal"
    } else if on_shell {
        "i attach · [ ] tabs · T term · ^w split · w close · x/X stop · esc back"
    } else if app.chat_mode == ChatMode::Terminal {
        "i attach · t transcript · [ ] tabs · ^w split · w close · ? help · esc back"
    } else {
        "i type · j/k scroll · t terminal · ^w split · y/n/a · 1-9 · ? help · esc back"
    };
    // In any normal/navigation mode (not a text field or raw terminal), point at
    // the leader menu so it's discoverable.
    let in_text = app.notes_view.is_some()
        || app.rename.is_some()
        || app.spawn_form.is_some()
        || app.term_attached()
        || app.filter_editing
        || (in_agent && app.insert_mode);
    let body = if in_text {
        format!(" {hint} ")
    } else {
        format!(" {hint} · {} menu", app.keymap.leader().display())
    };
    // lualine-style mode chip on the left, then the contextual hint.
    let (label, color) = mode_chip(app, in_agent, on_shell);
    let mut spans = vec![Span::styled(
        format!(" {label} "),
        Style::default()
            .bg(color)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    )];
    // Pending vim count (e.g. while typing `12` before `G`).
    if let Some(n) = app.count {
        spans.push(Span::styled(
            format!(" {n}"),
            Style::default()
                .fg(app.theme.warn)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(body, Style::default().fg(app.theme.dim)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The current editing/navigation mode, as a (label, colour) chip for the
/// footer — so the modal state is never ambiguous.
pub(super) fn mode_chip(app: &App, in_agent: bool, on_shell: bool) -> (&'static str, Color) {
    let t = &app.theme;
    if app.notes_view.as_ref().is_some_and(|n| n.editing) {
        ("NOTES", t.ok)
    } else if app.rename.is_some() {
        ("RENAME", t.accent)
    } else if app.spawn_form.is_some() {
        ("SPAWN", t.accent)
    } else if app.term_attached() {
        ("TERM", t.bad)
    } else if app.filter_editing {
        ("FILTER", t.accent)
    } else if in_agent && app.insert_mode {
        ("INSERT", t.ok)
    } else if in_agent && on_shell {
        ("SHELL", t.warn)
    } else {
        ("NORMAL", t.accent)
    }
}

// ── text wrapping ───────────────────────────────────────────────────────────
