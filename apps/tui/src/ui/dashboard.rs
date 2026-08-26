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
    // Tombstones (hub offline) can't be acted on: they count as neither
    // "needs you" nor working — they get their own line below.
    let offline = fleet().filter(|a| a.hub_offline).count();
    let waiting = fleet().filter(|a| a.needs_you()).count();
    let busy = fleet().filter(|a| a.is_busy() && !a.hub_offline).count();
    let idle = total.saturating_sub(waiting + busy + offline);
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
    if offline > 0 {
        lines.push(Line::from(vec![
            Span::styled("hub offline ", Style::default().fg(t.dim)),
            Span::styled(
                format!("{offline}"),
                Style::default().fg(t.bad).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    // Peer hubs, when any are federated in: name, reachability, session count.
    let hubs = app.remote.summary();
    if !hubs.is_empty() {
        let mut spans = vec![Span::styled("hubs ", Style::default().fg(t.dim))];
        for (i, (name, online, count)) in hubs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(t.dim)));
            }
            if *online {
                spans.push(Span::styled(
                    format!("{name} ● {count}"),
                    Style::default().fg(t.accent),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{name} ○ offline"),
                    Style::default().fg(t.dim),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    // Remote worker nodes, when any is not quietly fine. A machine that is
    // asleep or unreachable is WHY dispatches are missing from the roster
    // below — and a healthy fleet of them says nothing at all.
    if let Some(line) = nodes_dashboard_line(app) {
        lines.push(line);
    }
    if let Some(s) = rate {
        let mut spans = vec![Span::styled("rate limit ", Style::default().fg(t.dim))];
        // Each window wears its own length where the provider reports one, so a
        // Codex primary that isn't five hours doesn't get mislabelled "5h".
        let five = window_short(s.five_hour_window_minutes).unwrap_or_else(|| "5h".into());
        let seven = window_short(s.seven_day_window_minutes).unwrap_or_else(|| "7d".into());
        let monthly = window_short(s.monthly_window_minutes).unwrap_or_else(|| "Mo".into());
        if let Some(p) = s.five_hour_pct {
            spans.push(Span::styled(
                format!("{five} {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        if let Some(p) = s.seven_day_pct {
            spans.push(Span::styled(
                format!("   {seven} {p:.0}%"),
                Style::default().fg(rate_color(t, p)),
            ));
        }
        if let Some(p) = s.monthly_pct {
            spans.push(Span::styled(
                format!("   {monthly} {p:.0}%"),
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
        let marker = if a.hub_offline {
            Span::styled("○ ", Style::default().fg(t.dim))
        } else if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        // The name column carries the hub tag for remote rows, so a mixed
        // fleet reads machine-by-machine at a glance.
        let name = match &a.hub {
            Some(h) => format!("{} [{h}]", app.agent_name(a)),
            None => app.agent_name(a),
        };
        let (state_txt, state_style) = if a.hub_offline {
            ("hub offline", Style::default().fg(t.dim))
        } else {
            (a.state(), Style::default().fg(state_color(t, a.state())))
        };
        let mut row = vec![
            marker,
            project_mark(app, a),
            Span::styled(
                format!("{:<28}", crate::types::truncate(&name, 28)),
                Style::default(),
            ),
            Span::styled(format!("{state_txt:<10}"), state_style),
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
