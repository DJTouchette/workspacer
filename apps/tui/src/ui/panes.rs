//! Pane layout inside an agent workspace — terminals, splits, and the read-only
//! watch pane onto another agent's session.

use super::*;

pub(super) fn render_terminal(f: &mut Frame, area: Rect, app: &mut App) {
    let (accent, dim) = (app.theme.accent, app.theme.dim);
    let title = app
        .chat_agent()
        .map(|a| format!(" {} ", app.agent_name(a)))
        .unwrap_or_else(|| " session ended ".into());
    let border = if app.term_attached() { accent } else { dim };
    let bottom = if app.term_attached() {
        Line::from(Span::styled(
            " ● attached — Ctrl-] to detach ",
            Style::default().fg(accent),
        ))
    } else {
        Line::from(Span::styled(
            " i/enter to attach · t transcript ",
            Style::default().fg(dim),
        ))
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
    if let Some(s) = sid.as_ref() {
        if let Some(term) = app.terms.get_mut(s) {
            if term.resize(inner.height, inner.width) {
                app.term_resizes
                    .insert(s.clone(), (inner.width, inner.height)); // (cols, rows)
            }
        }
    }

    match sid.as_ref().and_then(|s| app.terms.get(s)) {
        Some(term) => {
            let pty = PseudoTerminal::new(term.screen()).block(block);
            f.render_widget(pty, area);
        }
        None => {
            let p = Paragraph::new(Line::from(Span::styled(
                "starting terminal…",
                Style::default().fg(dim),
            )))
            .block(block);
            f.render_widget(p, area);
        }
    }
}

// ── window splits (tiled panes) ────────────────────────────────────────────────

/// The content area when an agent is open. With a single tile it's just the
/// agent view; with more, it tiles each agent — the focused one fully
/// interactive, the rest a live read-only terminal.
pub(super) fn render_panes(f: &mut Frame, area: Rect, app: &mut App) {
    if app.tiles.len() <= 1 {
        render_agent(f, area, app);
        return;
    }
    let dir = match app.split_dir {
        SplitDir::Columns => Direction::Horizontal,
        SplitDir::Rows => Direction::Vertical,
    };
    let n = app.tiles.len() as u32;
    let cells = Layout::default()
        .direction(dir)
        .constraints((0..n).map(|_| Constraint::Ratio(1, n)).collect::<Vec<_>>())
        .split(area);
    // Clone the tile list so we can hand `render_agent` a `&mut App`.
    let tiles = app.tiles.clone();
    let focus = app.tile_focus;
    for (i, sid) in tiles.iter().enumerate() {
        if i == focus {
            render_agent(f, cells[i], app);
        } else {
            render_watch_pane(f, cells[i], app, sid);
        }
    }
}

/// A non-focused tile: the agent's live terminal, read-only, dim-bordered.
pub(super) fn render_watch_pane(f: &mut Frame, area: Rect, app: &mut App, sid: &str) {
    let (dim, warn) = (app.theme.dim, app.theme.warn);
    let agent = app.all_agents.iter().find(|a| a.session_id == sid);
    let name = agent
        .map(|a| app.agent_name(a))
        .unwrap_or_else(|| "session ended".into());
    let waiting = agent.is_some_and(|a| a.is_waiting());
    // A waiting agent gets an amber marker so it still draws the eye when it's
    // not the focused pane.
    let title = Line::from(vec![
        Span::styled(if waiting { " ● " } else { " " }, Style::default().fg(warn)),
        Span::styled(format!("{name} "), Style::default().fg(dim)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " Ctrl-w w to focus ",
            Style::default().fg(dim),
        )))
        .border_style(Style::default().fg(if waiting { warn } else { dim }));
    let inner = block.inner(area);

    if let Some(term) = app.terms.get_mut(sid) {
        if term.resize(inner.height, inner.width) {
            app.term_resizes
                .insert(sid.to_string(), (inner.width, inner.height));
        }
    }
    match app.terms.get(sid) {
        Some(term) => {
            let pty = PseudoTerminal::new(term.screen()).block(block);
            f.render_widget(pty, area);
        }
        None => {
            let msg = if app.no_terminal.contains(sid) {
                "transcript only — Ctrl-w w to read"
            } else {
                "starting terminal…"
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(dim))))
                    .block(block),
                area,
            );
        }
    }
}

// ── agent view: tab bar + active pane ─────────────────────────────────────────

pub(super) fn render_agent(f: &mut Frame, area: Rect, app: &mut App) {
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
