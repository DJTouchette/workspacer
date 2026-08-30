//! The docked side pane and the transcript's changed-file list.

use super::*;

/// The pane docked beside the agent: the agent's own changes.
/// The border says when it has the keys, so `Ctrl-w` is never a guess.
pub(super) fn render_side_pane(f: &mut Frame, area: Rect, app: &App) {
    match app.side.as_ref().map(|p| p.kind) {
        Some(crate::app::SideKind::Changes) => render_changes(f, area, app),
        None => {}
    }
}

/// Files the agent changed, newest turn first.
///
/// This is the agent's account of its own work, taken from the transcript — a
/// different question from what the work tree looks like now. A file the agent
/// edited and then reverted appears here; a file you changed by hand does not.
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

// ── footer ────────────────────────────────────────────────────────────────────
