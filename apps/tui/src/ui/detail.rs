//! The list view's right-hand pane: an agent's stats and plan, and the pending
//! approval / question block it shares with the chat view.

use super::*;

pub(super) fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" details ")
        .border_style(Style::default().fg(t.dim));

    let Some(a) = app.selected_agent() else {
        let p = Paragraph::new(Line::from(Span::styled(
            "select an agent — enter to open",
            Style::default().fg(t.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    };

    let mut lines: Vec<Line> = vec![
        kv(t, "cwd", a.cwd_str()),
        Line::from(vec![
            Span::styled("state  ", Style::default().fg(t.dim)),
            Span::styled(
                badge(a.state()),
                Style::default()
                    .fg(state_color(t, a.state()))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    if let Some(h) = &a.hub {
        if a.hub_offline {
            lines.insert(
                1,
                kv(t, "hub", &format!("{h} — offline (last known state)")),
            );
            lines.insert(
                2,
                Line::from(Span::styled(
                    "hub unreachable — actions disabled until it reconnects",
                    Style::default().fg(t.warn),
                )),
            );
        } else {
            lines.insert(
                1,
                kv(t, "hub", &format!("{h} (remote — transcript view only)")),
            );
        }
    }
    let stats = derive_stats(a, app.status_lines.get(&a.session_id));
    if let Some(m) = &stats.model {
        lines.push(kv(t, "model", m));
    }
    if let Some(p) = stats.context_pct {
        lines.push(kv(t, "context", &format!("{p:.0}%")));
    }
    if let Some(c) = stats.cost {
        lines.push(kv(t, "cost", &format!("${c:.2}")));
    }
    // Account-wide rate-limit windows, when Claude reports them (Pro/Max).
    if let Some(sl) = app.status_lines.get(&a.session_id) {
        if let Some(p) = sl.five_hour_pct {
            lines.push(kv(t, "5h", &format!("{p:.0}% used")));
        }
        if let Some(p) = sl.seven_day_pct {
            lines.push(kv(t, "7d", &format!("{p:.0}% used")));
        }
        if let Some(p) = sl.monthly_pct {
            lines.push(kv(t, "Mo", &format!("{p:.0}% used")));
        }
        if sl.overage_out_of_credits == Some(true) {
            lines.push(kv(t, "overage", "out of credits"));
        }
        if let Some(w) = &sl.rate_limit_warning {
            lines.push(Line::from(Span::styled(
                format!("⚠ {w}"),
                Style::default().fg(t.warn),
            )));
        }
    }
    if a.tool_calls > 0 {
        lines.push(kv(t, "tools", &a.tool_calls.to_string()));
    }
    if let Some(ev) = a.last_event.as_deref().filter(|e| !e.is_empty()) {
        lines.push(kv(t, "event", ev));
    }
    // The agent's own checklist, when it has published one.
    if let Some(plan) = app.plan_for(&a.session_id) {
        lines.push(Line::raw(""));
        lines.extend(plan_lines(t, plan, area.width.saturating_sub(2), false));
    }
    lines.push(Line::raw(""));
    lines.extend(ask_lines(
        t,
        a,
        app.question_flow.as_ref(),
        area.width.saturating_sub(2),
    ));

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, area);
}

/// The agent's plan as a checklist.
///
/// The step in flight is what a glance should land on, so it carries the accent
/// and its present-tense `activeForm` ("Wiring the delta feed") rather than the
/// imperative title — that phrasing exists precisely to be read while it's
/// happening. Done steps stay visible but recede; a long plan is truncated
/// around the current step rather than from the top, so the interesting part
/// never scrolls out of a short pane.
///
/// `compact` drops the header and the done/pending steps, leaving the one line
/// that answers "what is it doing" — for the composer, where vertical space is
/// the scarce thing.
pub(super) fn plan_lines(
    t: &Theme,
    plan: &crate::types::Plan,
    width: u16,
    compact: bool,
) -> Vec<Line<'static>> {
    use crate::types::PlanStatus;
    let w = (width.max(10) as usize).saturating_sub(4);
    let mut out = Vec::new();
    let done = plan.done();
    let total = plan.steps.len();

    if compact {
        let Some(step) = plan.current() else {
            return out;
        };
        let text = step.active_form.as_deref().unwrap_or(&step.content);
        out.push(Line::from(vec![
            Span::styled("◐ ", Style::default().fg(t.accent)),
            Span::styled(
                crate::render::truncate_width(text, w.saturating_sub(10)),
                Style::default().fg(t.fg),
            ),
            Span::styled(
                format!("  {done}/{total}"),
                Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
            ),
        ]));
        return out;
    }

    out.push(Line::from(vec![
        Span::styled("plan   ", Style::default().fg(t.dim)),
        Span::styled(
            format!("{done}/{total}"),
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", progress_bar(done, total)),
            Style::default().fg(if done == total { t.ok } else { t.accent }),
        ),
    ]));

    // Keep the window around the step in flight (or the end, once everything is
    // done) — the top of a long plan is the least interesting part of it.
    const MAX_STEPS: usize = 8;
    let anchor = plan
        .steps
        .iter()
        .position(|s| s.status == PlanStatus::InProgress)
        .unwrap_or(total.saturating_sub(1));
    let start = if total <= MAX_STEPS {
        0
    } else {
        anchor.saturating_sub(MAX_STEPS / 2).min(total - MAX_STEPS)
    };
    if start > 0 {
        out.push(Line::from(Span::styled(
            format!("  … {start} earlier"),
            Style::default().fg(t.dim),
        )));
    }
    for step in plan.steps.iter().skip(start).take(MAX_STEPS) {
        let (glyph, color, modifier) = match step.status {
            PlanStatus::Completed => ("✓", t.ok, Modifier::DIM),
            PlanStatus::InProgress => ("◐", t.accent, Modifier::BOLD),
            PlanStatus::Pending => ("○", t.dim, Modifier::empty()),
        };
        // The in-flight step speaks in the present tense; the others don't.
        let text = match step.status {
            PlanStatus::InProgress => step.active_form.as_deref().unwrap_or(&step.content),
            _ => step.content.as_str(),
        };
        out.push(Line::from(vec![
            Span::styled(format!("  {glyph} "), Style::default().fg(color)),
            Span::styled(
                crate::render::truncate_width(text, w),
                Style::default().fg(color).add_modifier(modifier),
            ),
        ]));
    }
    let shown = start + MAX_STEPS.min(total - start);
    if shown < total {
        out.push(Line::from(Span::styled(
            format!("  … {} more", total - shown),
            Style::default().fg(t.dim),
        )));
    }
    out
}

/// A tiny unicode meter — `▰▰▰▱▱`, eight cells wide.
pub(super) fn progress_bar(done: usize, total: usize) -> String {
    const CELLS: usize = 8;
    if total == 0 {
        return String::new();
    }
    let filled = (done * CELLS).div_ceil(total).min(CELLS);
    format!("{}{}", "▰".repeat(filled), "▱".repeat(CELLS - filled))
}

pub(super) fn kv<'a>(t: &Theme, k: &'a str, v: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{k:<7}"), Style::default().fg(t.dim)),
        Span::raw(v.to_string()),
    ])
}

/// Pretty-print the most relevant slice of a permission-request hook payload —
/// the tool input if we can find it, else the whole thing.
pub(super) fn approval_input(raw: &Value) -> String {
    let target = raw
        .get("tool_input")
        .or_else(|| raw.get("input"))
        .unwrap_or(raw);
    serde_json::to_string_pretty(target).unwrap_or_default()
}

/// The pending approval / question block, shared by the detail and chat panes.
///
/// Multi-question sets render ONE question at a time with a `Q n of m`
/// progress marker, stepping via `flow` (see [`crate::app::QuestionFlow`]);
/// multi-select questions render ☐/☑ checkboxes that digits toggle and Enter
/// confirms.
pub(super) fn ask_lines(
    t: &Theme,
    a: &Agent,
    flow: Option<&crate::app::QuestionFlow>,
    width: u16,
) -> Vec<Line<'static>> {
    let w = width.max(10) as usize;
    let mut out = Vec::new();
    if let Some((tool, raw)) = a.approval() {
        out.push(Line::from(Span::styled(
            format!("⚠ wants to run {tool}"),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )));
        let pretty = approval_input(raw);
        for line in pretty.lines().take(12) {
            for piece in wrap(line, w) {
                out.push(Line::from(Span::styled(piece, Style::default().fg(t.dim))));
            }
        }
        out.push(Line::raw(""));
        out.push(Line::from(Span::styled(
            "[y]es  [n]o  [a]lways",
            Style::default().fg(t.ok),
        )));
    } else if let Some(qs) = a.questions().filter(|q| !q.is_empty()) {
        let n = qs.len();
        // The stepper's flow only applies when it tracks this exact set
        // (same session, length, AND content — not a superseded look-alike).
        let flow = flow.filter(|f| f.tracks(&a.session_id, qs));
        let idx = flow.map(|f| f.idx.min(n - 1)).unwrap_or(0);
        let q = &qs[idx];

        let mut head = vec![Span::styled(
            q.header.clone().unwrap_or_else(|| "Question".into()),
            Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
        )];
        if n > 1 {
            head.push(Span::styled(
                format!("  · Q {} of {n}", idx + 1),
                Style::default().fg(t.dim),
            ));
        }
        out.push(Line::from(head));
        for piece in wrap(&q.question, w) {
            out.push(Line::raw(piece));
        }

        // The recorded pick for a revisited question renders highlighted.
        let prev_pick = flow.and_then(|f| f.answers[idx].as_deref());
        let picks = flow.map(|f| &f.picks[idx]);
        if q.multi_select && !q.options.is_empty() {
            for (i, o) in q.options.iter().enumerate().take(9) {
                let checked = picks.is_some_and(|p| p.contains(&i));
                let (bx, style) = if checked {
                    ("☑", Style::default().fg(t.ok))
                } else {
                    ("☐", Style::default().fg(t.dim))
                };
                out.push(Line::from(vec![
                    Span::styled(format!(" {}. ", i + 1), Style::default().fg(t.accent)),
                    Span::styled(format!("{bx} "), style),
                    Span::raw(o.label.clone()),
                ]));
                push_option_desc(&mut out, t, w, o);
            }
            out.push(Line::raw(""));
            out.push(Line::from(Span::styled(
                back_hint("1-9 toggle · enter confirm", idx),
                Style::default().fg(t.dim),
            )));
        } else if !q.options.is_empty() {
            for (i, o) in q.options.iter().enumerate().take(9) {
                let chosen = prev_pick == Some((i + 1).to_string().as_str());
                let label_style = if chosen {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let marker = if chosen { "❯" } else { " " };
                out.push(Line::from(vec![
                    Span::styled(marker.to_string(), Style::default().fg(t.accent)),
                    Span::styled(format!("{}. ", i + 1), Style::default().fg(t.accent)),
                    Span::styled(o.label.clone(), label_style),
                ]));
                push_option_desc(&mut out, t, w, o);
            }
            out.push(Line::raw(""));
            out.push(Line::from(Span::styled(
                back_hint("press 1-9 to answer, or i to type", idx),
                Style::default().fg(t.dim),
            )));
        } else {
            out.push(Line::from(Span::styled(
                back_hint("press i to type your answer", idx),
                Style::default().fg(t.dim),
            )));
        }
    }
    out
}

/// A question option's dim description lines, wrapped and indented.
pub(super) fn push_option_desc(
    out: &mut Vec<Line<'static>>,
    t: &Theme,
    w: usize,
    o: &crate::types::QuestionOption,
) {
    if let Some(desc) = o.description.as_ref().filter(|d| !d.is_empty()) {
        for piece in wrap(desc, w.saturating_sub(4)) {
            out.push(Line::from(Span::styled(
                format!("    {piece}"),
                Style::default().fg(t.dim),
            )));
        }
    }
}

/// Append the mid-set `esc back` hint to a question footer.
pub(super) fn back_hint(base: &str, idx: usize) -> String {
    if idx > 0 {
        format!("{base} · esc back")
    } else {
        base.to_string()
    }
}

// ── chat view ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::testutil::line_texts;
    use super::*;

    #[test]
    fn ask_lines_renders_the_stepper_and_multiselect_checkboxes() {
        let t = Theme::default();
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s1", "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Pick one", "options": [{"label": "A"}, {"label": "B"}]},
                {"question": "Choose", "multi_select": true,
                 "options": [{"label": "X"}, {"label": "Y"}]}
            ]}
        }))
        .unwrap();

        // Before any interaction: Q1 renders with its progress marker.
        let texts = line_texts(&ask_lines(&t, &a, None, 60));
        assert!(texts[0].contains("Q 1 of 2"), "got {:?}", texts[0]);
        assert!(texts.iter().any(|l| l.contains("1. A")));
        assert!(
            !texts.iter().any(|l| l.contains("esc back")),
            "no back hint on the first question"
        );

        // Mid-set on the multi-select: checkboxes reflect the toggles.
        let mut flow = crate::app::QuestionFlow::new("s1".into(), a.questions().unwrap());
        flow.idx = 1;
        flow.answers[0] = Some("2".into());
        flow.picks[1].insert(0);
        let texts = line_texts(&ask_lines(&t, &a, Some(&flow), 60));
        assert!(texts[0].contains("Q 2 of 2"), "got {:?}", texts[0]);
        assert!(texts.iter().any(|l| l.contains("☑ X")), "{texts:?}");
        assert!(texts.iter().any(|l| l.contains("☐ Y")), "{texts:?}");
        assert!(texts
            .iter()
            .any(|l| l.contains("enter confirm") && l.contains("esc back")));

        // Revisiting Q1: the recorded pick renders highlighted.
        flow.idx = 0;
        let lines = ask_lines(&t, &a, Some(&flow), 60);
        let texts = line_texts(&lines);
        let b_row = texts.iter().position(|l| l.contains("2. B")).unwrap();
        assert!(texts[b_row].starts_with('❯'), "got {:?}", texts[b_row]);
        let b_label = lines[b_row]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "B")
            .expect("label span");
        assert!(b_label.style.add_modifier.contains(Modifier::BOLD));
    }

    fn plan(steps: &[(&str, &str, Option<&str>)]) -> crate::types::Plan {
        crate::types::Plan {
            steps: steps
                .iter()
                .map(|(content, status, active)| crate::types::PlanStep {
                    content: (*content).into(),
                    status: match *status {
                        "done" => crate::types::PlanStatus::Completed,
                        "now" => crate::types::PlanStatus::InProgress,
                        _ => crate::types::PlanStatus::Pending,
                    },
                    active_form: active.map(|a| a.to_string()),
                })
                .collect(),
            updated_at: None,
        }
    }

    /// The step in flight is what the eye should land on, and it speaks in the
    /// present tense — that is what `activeForm` is for.
    #[test]
    fn plan_shows_progress_and_the_active_step_in_present_tense() {
        let t = Theme::default();
        let p = plan(&[
            ("Write the fold", "done", None),
            ("Wire the feed", "now", Some("Wiring the feed")),
            ("Render it", "pending", None),
        ]);
        let texts = line_texts(&plan_lines(&t, &p, 60, false));

        assert!(texts.iter().any(|l| l.contains("1/3")), "{texts:?}");
        assert!(texts.iter().any(|l| l.contains('▰')), "a meter: {texts:?}");
        assert!(
            texts.iter().any(|l| l.contains("Wiring the feed")),
            "present tense for the live step: {texts:?}"
        );
        assert!(
            !texts.iter().any(|l| l.contains("Wire the feed")),
            "not the imperative title too: {texts:?}"
        );
        assert!(texts.iter().any(|l| l.contains("✓")), "{texts:?}");
    }

    /// Compact form is for the composer border: the one line that answers "what
    /// is it doing", and nothing when nothing is in flight.
    #[test]
    fn compact_plan_is_one_line_and_empty_when_idle() {
        let t = Theme::default();
        let busy = plan(&[("a", "done", None), ("b", "now", Some("Doing b"))]);
        let lines = plan_lines(&t, &busy, 60, true);
        assert_eq!(lines.len(), 1);
        assert!(line_texts(&lines)[0].contains("Doing b"));
        assert!(line_texts(&lines)[0].contains("1/2"));

        let finished = plan(&[("a", "done", None)]);
        assert!(
            plan_lines(&t, &finished, 60, true).is_empty(),
            "no step in flight → nothing to say"
        );
    }

    /// A long plan keeps the window around the live step — truncating from the
    /// top would scroll the interesting part out of a short pane.
    #[test]
    fn a_long_plan_windows_around_the_live_step() {
        let t = Theme::default();
        let mut steps: Vec<(&str, &str, Option<&str>)> =
            (0..20).map(|_| ("filler", "done", None)).collect();
        steps[15] = ("the live one", "now", None);
        let texts = line_texts(&plan_lines(&t, &plan(&steps), 60, false));

        assert!(
            texts.iter().any(|l| l.contains("the live one")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|l| l.contains("earlier")),
            "says what it hid above: {texts:?}"
        );
    }

    #[test]
    fn progress_bar_fills_and_completes() {
        assert_eq!(progress_bar(0, 4), "▱▱▱▱▱▱▱▱");
        assert_eq!(progress_bar(4, 4), "▰▰▰▰▰▰▰▰");
        assert_eq!(progress_bar(1, 8).chars().filter(|c| *c == '▰').count(), 1);
        assert_eq!(progress_bar(0, 0), "", "no plan, no meter");
    }

    /// The approval panel wants the tool's arguments, not the envelope the hook
    /// wrapped them in. `tool_input` wins, `input` is the older spelling, and a
    /// payload with neither is shown whole rather than blanked.
    #[test]
    fn approval_input_unwraps_the_tool_payload() {
        let both = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "ls" },
            "input": { "command": "rm -rf /" },
        });
        let out = approval_input(&both);
        assert!(
            out.contains("\"command\": \"ls\""),
            "tool_input wins: {out}"
        );
        assert!(!out.contains("rm -rf"), "not the legacy key too: {out}");

        let legacy = serde_json::json!({ "input": { "command": "ls" } });
        assert!(approval_input(&legacy).contains("\"command\": \"ls\""));

        // Neither key: show the whole envelope rather than nothing.
        let bare = serde_json::json!({ "anything": 1 });
        assert!(approval_input(&bare).contains("\"anything\": 1"));

        // Pretty-printed, so a multi-key payload is readable in the panel.
        assert!(
            approval_input(&serde_json::json!({ "a": 1, "b": 2 })).contains('\n'),
            "pretty, not compact"
        );
    }

    /// Esc only means "back" once there is somewhere to go back to.
    #[test]
    fn back_hint_appears_only_past_the_first_step() {
        assert_eq!(back_hint("enter confirm", 0), "enter confirm");
        assert_eq!(back_hint("enter confirm", 1), "enter confirm · esc back");
        assert_eq!(back_hint("enter confirm", 9), "enter confirm · esc back");
    }

    /// The detail pane's key column is a fixed 7 columns so the values line up.
    #[test]
    fn kv_pads_the_key_column_and_dims_it() {
        let t = Theme::default();
        let line = kv(&t, "cwd", "/tmp");
        assert_eq!(line.spans[0].content.as_ref(), "cwd    ", "padded to 7");
        assert_eq!(line.spans[0].style.fg, Some(t.dim));
        assert_eq!(line.spans[1].content.as_ref(), "/tmp");

        // An over-long key is not truncated — it pushes the value instead.
        assert_eq!(
            kv(&t, "transcript", "x").spans[0].content.as_ref(),
            "transcript"
        );
    }
}
