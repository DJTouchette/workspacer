//! The git panes: the docked side pane, the transcript's changed-file list, and
//! the full review view with its file list and unified diff.

use super::*;

/// The pane docked beside the agent: the git review, or the agent's own changes.
/// The border says which one has the keys, so `Ctrl-w` is never a guess.
pub(super) fn render_side_pane(f: &mut Frame, area: Rect, app: &App) {
    match app.side.as_ref().map(|p| p.kind) {
        Some(crate::app::SideKind::Review) => render_review(f, area, app),
        Some(crate::app::SideKind::Changes) => render_changes(f, area, app),
        None => {}
    }
}

/// Files the agent changed, newest turn first.
///
/// This is the agent's account of its own work, taken from the transcript — a
/// different question from what the work tree looks like now, which is the review
/// pane beside it. A file the agent edited and then reverted appears here and not
/// there; a file you changed by hand appears there and not here. Both are true.
pub(super) fn render_changes(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let focused = app.side_focused();
    let Some(pane) = app.side_of(crate::app::SideKind::Changes) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" changes ")
        .title_bottom(Line::from(Span::styled(
            if focused {
                " j/k scroll · ctrl-w back · esc close "
            } else {
                " ctrl-w to focus "
            },
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(if focused { t.accent } else { t.dim }));

    let Some(fold) = app.folds.get(&pane.session_id) else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "open the agent's chat to load its transcript",
                Style::default().fg(t.dim),
            )))
            .block(block),
            area,
        );
        return;
    };

    let w = area.width.saturating_sub(2);
    let mut lines: Vec<Line> = Vec::new();

    let session = fold.session_changes();
    if session.is_empty() {
        lines.push(Line::from(Span::styled(
            "this agent hasn't changed any files yet",
            Style::default().fg(t.dim),
        )));
    } else {
        let (added, removed): (usize, usize) = session
            .iter()
            .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} files", session.len()),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  +{added}"), Style::default().fg(t.ok)),
            Span::styled(format!(" −{removed}"), Style::default().fg(t.bad)),
            Span::styled("  this session", Style::default().fg(t.dim)),
        ]));
        lines.push(Line::raw(""));

        // Grouped by turn, newest first: "what did that last turn touch" is the
        // question, and a flat file list can't answer it.
        for (i, (_, changes)) in fold.changed_turns().iter().enumerate() {
            let (a, r): (usize, usize) = changes
                .iter()
                .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
            lines.push(Line::from(vec![
                Span::styled(
                    if i == 0 {
                        "latest turn".to_string()
                    } else {
                        format!("{i} turns back")
                    },
                    Style::default()
                        .fg(if i == 0 { t.accent } else { t.dim })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  +{a}"), Style::default().fg(t.ok)),
                Span::styled(format!(" −{r}"), Style::default().fg(t.bad)),
            ]));
            for c in changes {
                lines.push(change_row(t, c, w));
            }
            lines.push(Line::raw(""));
        }
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .scroll((pane.scroll, 0))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// One changed-file row: path on the left, its +/− on the right.
pub(super) fn change_row(t: &Theme, c: &crate::types::ChangedFile, width: u16) -> Line<'static> {
    let counts = format!("+{} −{}", c.added, c.removed);
    let room = (width as usize).saturating_sub(counts.len() + 5).max(8);
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            crate::render::truncate_width(&c.short_path(), room),
            Style::default().fg(t.fg),
        ),
        Span::raw(" "),
        Span::styled(format!("+{}", c.added), Style::default().fg(t.ok)),
        Span::styled(format!(" −{}", c.removed), Style::default().fg(t.bad)),
    ])
}

// ── runs overlay: workflows + subagents ───────────────────────────────────────

pub(super) fn render_review(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(r) = app.review.as_ref() else { return };

    let branch = r.branch.as_deref().unwrap_or("(detached)");
    let view = if r.staged_view { "staged" } else { "unstaged" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " review · {branch} · {view} ({} files) ",
            r.files.len()
        ))
        .title_bottom(Line::from(Span::styled(
            if app.side_focused() {
                " ctrl-w back to chat "
            } else {
                " ctrl-w to focus "
            },
            Style::default().fg(t.dim),
        )))
        // Dim when the chat has the keys, so which pane a keystroke lands in is
        // never a guess.
        .border_style(Style::default().fg(if app.side_focused() { t.accent } else { t.dim }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let composing = r.commit_msg.is_some();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if composing { 3 } else { 0 }),
        ])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(20)])
        .split(rows[0]);

    render_review_files(f, cols[0], t, r);
    render_review_diff(f, cols[1], t, r);

    if composing {
        let msg = r.commit_msg.as_deref().unwrap_or("");
        let p = Paragraph::new(Line::from(vec![
            Span::styled(
                "commit ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{msg}▏")),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" message · enter commit · esc cancel ")
                .border_style(Style::default().fg(t.accent)),
        );
        f.render_widget(p, rows[1]);
    }
}

pub(super) fn render_review_files(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    r: &crate::app::ReviewState,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" files ")
        .border_style(Style::default().fg(t.dim));
    if let Some(err) = &r.error {
        let p = Paragraph::new(vec![
            Line::from(Span::styled(
                "git unavailable",
                Style::default().fg(t.bad).add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
            Line::from(Span::styled(err.clone(), Style::default().fg(t.dim))),
        ])
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(block);
        f.render_widget(p, area);
        return;
    }
    if r.files.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "working tree clean",
            Style::default().fg(t.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = r
        .files
        .iter()
        .map(|file| {
            let staged = file.staged.trim();
            let unstaged = file.unstaged.trim();
            let sc = if staged.is_empty() {
                '·'
            } else {
                staged.chars().next().unwrap()
            };
            let uc = if unstaged.is_empty() {
                '·'
            } else {
                unstaged.chars().next().unwrap()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{sc}"), Style::default().fg(t.ok)),
                Span::styled(format!("{uc} "), Style::default().fg(t.warn)),
                Span::styled(
                    crate::types::truncate(&file.display_path(), 30),
                    Style::default(),
                ),
            ]))
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(r.selected));
    f.render_stateful_widget(list, area, &mut state);
}

pub(super) fn render_review_diff(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    r: &crate::app::ReviewState,
) {
    let path = r
        .selected_file()
        .map(|file| file.path.as_str())
        .unwrap_or("");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", if path.is_empty() { "diff" } else { path }))
        .border_style(Style::default().fg(t.dim));

    if r.diff.trim().is_empty() {
        let msg = if r.files.is_empty() {
            "nothing to review"
        } else {
            "no changes in this view"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(t.dim)))).block(block),
            area,
        );
        return;
    }

    let lines: Vec<Line> = r
        .diff
        .lines()
        .map(|line| {
            let style = if line.starts_with("@@") {
                Style::default().fg(t.accent)
            } else if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("diff ")
                || line.starts_with("index ")
            {
                Style::default().fg(t.dim)
            } else if line.starts_with('+') {
                Style::default().fg(t.ok)
            } else if line.starts_with('-') {
                Style::default().fg(t.bad)
            } else {
                Style::default()
            };
            Line::from(Span::styled(line.to_string(), style))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .scroll((r.diff_scroll, 0)),
        area,
    );
}

// ── footer ────────────────────────────────────────────────────────────────────
