//! The runs overlay — workflows and their subagents, with the duration and count
//! formatting that only this view needs.

use super::*;

/// The work an agent spawned: workflow runs (grouped by phase, when known) and
/// plain Agent-tool subagents.
///
/// Read from Claude Code's on-disk artifacts (see [`crate::runs`]) on a tick, so
/// a live run advances while you watch it. Live work sorts first, because the
/// question this answers is "what is happening right now" — a finished run is
/// history and can wait at the bottom.
pub(super) fn render_runs(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" runs ")
        .title_bottom(Line::from(Span::styled(
            " r refresh · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));

    let Some((sid, runs)) = app.open_runs_view() else {
        // The overlay opens before the first read lands, so this is the normal
        // first frame, not an error.
        let msg = if app.runs_open.is_some() {
            "reading…"
        } else {
            "no agent"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(t.dim)))).block(block),
            area,
        );
        return;
    };

    let w = area.width.saturating_sub(2);
    let mut lines: Vec<Line> = Vec::new();

    if runs.is_empty() {
        lines.push(Line::from(Span::styled(
            "no subagents or workflow runs for this session",
            Style::default().fg(t.dim),
        )));
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "the Agent and Workflow tools write their progress here as they run",
            Style::default().fg(t.dim),
        )));
    }

    for run in &runs.workflows {
        lines.extend(workflow_lines(t, run, w));
        lines.push(Line::raw(""));
    }

    if !runs.subagents.is_empty() {
        let live = runs
            .subagents
            .iter()
            .filter(|s| s.state == crate::runs::RunState::Running)
            .count();
        lines.push(Line::from(vec![
            Span::styled(
                "subagents ",
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}  ", runs.subagents.len()),
                Style::default().fg(t.dim),
            ),
            Span::styled(
                if live > 0 {
                    format!("{live} running")
                } else {
                    "all finished".to_string()
                },
                Style::default().fg(if live > 0 { t.accent } else { t.dim }),
            ),
        ]));
        for sub in &runs.subagents {
            lines.push(agent_row(
                t,
                sub.state,
                sub.agent_type.as_deref().unwrap_or("agent"),
                sub.description.as_deref(),
                sub.last_tool.as_deref(),
                w,
            ));
        }
    }

    // The plan belongs here too: it is the parent agent's own checklist, next to
    // the work it farmed out.
    if let Some(plan) = app.plan_for(sid) {
        lines.push(Line::raw(""));
        lines.extend(plan_lines(t, plan, w, false));
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

pub(super) fn workflow_lines(
    t: &Theme,
    run: &crate::runs::WorkflowRun,
    width: u16,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let total = run.agents.len();
    let done = run.done();
    let running = run.running();

    let mut header = vec![
        Span::styled(
            format!("{} ", run.state.glyph()),
            Style::default().fg(state_color_for(t, run.state)),
        ),
        Span::styled(
            run.name.clone().unwrap_or_else(|| run.run_id.clone()),
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
    ];
    if total > 0 {
        header.push(Span::styled(
            format!("  {done}/{total} "),
            Style::default().fg(t.dim),
        ));
        header.push(Span::styled(
            progress_bar(done, total),
            Style::default().fg(if done == total { t.ok } else { t.accent }),
        ));
    }
    if running > 0 {
        header.push(Span::styled(
            format!("  {running} running"),
            Style::default().fg(t.accent),
        ));
    }
    // A failure has to be loud: a dim × glyph alone is too easy to skim past.
    if run.state == crate::runs::RunState::Failed {
        header.push(Span::styled(
            format!("  {}", run.state.label()),
            Style::default().fg(t.bad).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(ms) = run.duration_ms {
        header.push(Span::styled(
            format!("  {}", human_duration(ms)),
            Style::default().fg(t.dim),
        ));
    }
    if let Some(tok) = run.total_tokens {
        header.push(Span::styled(
            format!("  {}", human_count(tok)),
            Style::default().fg(t.dim),
        ));
    }
    out.push(Line::from(header));

    // Group by phase when the run has told us its phases; a live run hasn't, so
    // its agents list flat rather than under invented headings.
    if run.phases.is_empty() {
        for a in &run.agents {
            out.push(agent_row(
                t,
                a.state,
                a.label.as_deref().unwrap_or(&a.id),
                a.model.as_deref(),
                a.last_tool.as_deref(),
                width,
            ));
        }
        return out;
    }

    for phase in &run.phases {
        let in_phase: Vec<&crate::runs::WorkflowAgent> = run
            .agents
            .iter()
            .filter(|a| a.phase.as_deref() == Some(phase.title.as_str()))
            .collect();
        let phase_done = in_phase
            .iter()
            .filter(|a| a.state == crate::runs::RunState::Done)
            .count();
        out.push(Line::from(vec![
            Span::styled("  ▸ ", Style::default().fg(t.dim)),
            Span::styled(
                phase.title.clone(),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {phase_done}/{}", in_phase.len()),
                Style::default().fg(t.dim),
            ),
        ]));
        if let Some(detail) = &phase.detail {
            out.push(Line::from(Span::styled(
                format!(
                    "      {}",
                    crate::render::truncate_width(detail, width as usize)
                ),
                Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
            )));
        }
        for a in in_phase {
            out.push(agent_row(
                t,
                a.state,
                a.label.as_deref().unwrap_or(&a.id),
                a.model.as_deref(),
                a.last_tool.as_deref(),
                width,
            ));
        }
    }
    // Agents the final file assigned to no phase still have to show up.
    let orphans: Vec<&crate::runs::WorkflowAgent> = run
        .agents
        .iter()
        .filter(|a| {
            a.phase
                .as_deref()
                .is_none_or(|p| !run.phases.iter().any(|ph| ph.title == p))
        })
        .collect();
    for a in orphans {
        out.push(agent_row(
            t,
            a.state,
            a.label.as_deref().unwrap_or(&a.id),
            a.model.as_deref(),
            a.last_tool.as_deref(),
            width,
        ));
    }
    out
}

/// One agent row: state glyph, name, and what it's doing (or was doing last).
pub(super) fn agent_row(
    t: &Theme,
    state: crate::runs::RunState,
    name: &str,
    detail: Option<&str>,
    last_tool: Option<&str>,
    width: u16,
) -> Line<'static> {
    let color = state_color_for(t, state);
    let running = state == crate::runs::RunState::Running;
    let mut spans = vec![
        Span::styled(
            format!("    {} ", state.glyph()),
            Style::default().fg(color),
        ),
        Span::styled(
            crate::render::truncate_width(name, (width as usize).saturating_sub(28).max(12)),
            Style::default()
                .fg(if running { t.fg } else { t.dim })
                .add_modifier(if running {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ];
    // The live tool is the interesting detail while running; the description is
    // what identifies a finished one.
    let tail = if running {
        last_tool.map(|tool| format!("  {tool}…"))
    } else {
        detail.map(|d| format!("  {d}"))
    };
    if let Some(tail) = tail {
        spans.push(Span::styled(
            crate::render::truncate_width(&tail, 26),
            Style::default().fg(t.dim),
        ));
    }
    Line::from(spans)
}

pub(super) fn state_color_for(t: &Theme, state: crate::runs::RunState) -> ratatui::style::Color {
    match state {
        crate::runs::RunState::Running => t.accent,
        crate::runs::RunState::Done => t.ok,
        crate::runs::RunState::Failed => t.bad,
        crate::runs::RunState::Queued => t.dim,
    }
}

/// `412108` → `6m52s`; short enough for a status row.
pub(super) fn human_duration(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return format!("{m}m{s:02}s");
    }
    format!("{}h{:02}m", m / 60, m % 60)
}

/// `1263590` → `1.3M`.
pub(super) fn human_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

// ── git review pane ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Durations are read at a glance in a status row: seconds under a minute,
    /// then zero-padded so the field doesn't jitter as the number ticks.
    #[test]
    fn human_duration_scales_and_zero_pads() {
        assert_eq!(human_duration(0), "0s");
        assert_eq!(human_duration(999), "0s", "sub-second rounds down");
        assert_eq!(human_duration(59_000), "59s");
        assert_eq!(human_duration(60_000), "1m00s", "padded, not 1m0s");
        assert_eq!(human_duration(412_108), "6m52s");
        assert_eq!(human_duration(3_599_000), "59m59s");
        assert_eq!(human_duration(3_600_000), "1h00m", "padded, not 1h0m");
        assert_eq!(human_duration(7_830_000), "2h10m");
    }

    /// Counts get a unit suffix past a thousand, one decimal place.
    #[test]
    fn human_count_switches_units_at_each_thousand() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(999), "999", "plain up to 999");
        assert_eq!(human_count(1_000), "1.0k");
        assert_eq!(human_count(1_263), "1.3k");
        assert_eq!(human_count(999_999), "1000.0k", "k runs to the very top");
        assert_eq!(human_count(1_000_000), "1.0M");
        assert_eq!(human_count(1_263_590), "1.3M");
    }

    /// Run states carry their own palette, distinct from session states:
    /// a *running* workflow is accent, not ok.
    #[test]
    fn run_state_colors_are_distinct_from_session_states() {
        let t = Theme::default();
        use crate::runs::RunState;
        assert_eq!(state_color_for(&t, RunState::Running), t.accent);
        assert_eq!(state_color_for(&t, RunState::Done), t.ok);
        assert_eq!(state_color_for(&t, RunState::Failed), t.bad);
        assert_eq!(state_color_for(&t, RunState::Queued), t.dim);
    }
}
