//! The fleet dashboard: every agent at a glance, shown when the sidebar's row 0
//! is selected.

use super::*;

pub(super) fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" dashboard ")
        .border_style(Style::default().fg(t.accent));

    // Fleet totals reflect the whole live set (not the `/`-filtered sidebar view),
    // but exclude TUI-spawned shells — they aren't agents.
    let fleet = || {
        app.all_agents
            .iter()
            .filter(|a| !app.is_shell_session(&a.session_id))
    };
    let total = fleet().count();
    let waiting = fleet().filter(|a| a.is_waiting()).count();
    let busy = fleet().filter(|a| a.is_busy()).count();
    let idle = total.saturating_sub(waiting + busy);
    let cost: f64 = fleet()
        .filter_map(|a| derive_stats(a, app.status_lines.get(&a.session_id)).cost)
        .sum();
    // Rate limits are account-wide (identical across sessions) — show the first
    // session that reports them.
    let rate = app.status_lines.values().find(|s| {
        s.five_hour_pct.is_some() || s.seven_day_pct.is_some() || s.monthly_pct.is_some()
    });

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "workspacer · {total} agent{}",
                if total == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("needs you ", Style::default().fg(t.dim)),
            Span::styled(
                format!("{waiting}"),
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    working ", Style::default().fg(t.dim)),
            Span::styled(
                format!("{busy}"),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("    idle ", Style::default().fg(t.dim)),
            Span::styled(format!("{idle}"), Style::default().fg(t.ok)),
        ]),
        Line::from(vec![
            Span::styled("total cost ", Style::default().fg(t.dim)),
            Span::styled(format!("${cost:.2}"), Style::default().fg(t.ok)),
        ]),
    ];
    if let Some(s) = rate {
        let mut spans = vec![Span::styled("rate limit ", Style::default().fg(t.dim))];
        if let Some(p) = s.five_hour_pct {
            spans.push(Span::styled(
                format!("5h {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        if let Some(p) = s.seven_day_pct {
            spans.push(Span::styled(
                format!("   7d {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        if let Some(p) = s.monthly_pct {
            spans.push(Span::styled(
                format!("   Mo {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));

    // Compact roster over the whole fleet (ignores the sidebar filter, skips shells).
    for a in app
        .all_agents
        .iter()
        .filter(|a| !app.is_shell_session(&a.session_id))
    {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        let mut row = vec![
            marker,
            project_mark(app, a),
            Span::styled(
                format!("{:<28}", crate::types::truncate(&app.agent_name(a), 28)),
                Style::default(),
            ),
            Span::styled(
                format!("{:<10}", a.state()),
                Style::default().fg(state_color(t, a.state())),
            ),
        ];
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        if let Some(p) = stats.context_pct {
            row.push(Span::styled(
                format!(" {p:.0}%"),
                Style::default().fg(t.dim),
            ));
        }
        if let Some(c) = stats.cost {
            row.push(Span::styled(
                format!("  ${c:.2}"),
                Style::default().fg(t.dim),
            ));
        }
        lines.push(Line::from(row));
    }
    if total == 0 {
        lines.push(Line::from(Span::styled(
            "no sessions yet — press c to spawn an agent",
            Style::default().fg(t.dim),
        )));
    }

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

// ── spawn modal ───────────────────────────────────────────────────────────────
