//! The agent list down the left, plus the per-agent status vocabulary — badge
//! text, state colour, rate-limit colour — that the rest of the UI borrows.

use super::*;

pub(super) fn render_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let title = if app.hidden_count > 0 {
        format!(
            " agents ({} · +{} stopped) ",
            app.agents.len(),
            app.hidden_count
        )
    } else {
        format!(" agents ({}) ", app.agents.len())
    };
    let editing = app.filter_editing;
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if editing { t.accent } else { t.dim }));
    // Show the active `/` filter along the bottom edge (with a cursor while
    // it's being typed).
    if let Some(q) = &app.filter {
        let txt = if editing {
            format!(" /{q}▏ ")
        } else {
            format!(" /{q} ")
        };
        block = block.title_bottom(Line::from(Span::styled(
            txt,
            Style::default().fg(if editing { t.accent } else { t.dim }),
        )));
    }

    // Pinned Dashboard row, then one row per agent.
    let mut items: Vec<ListItem> = Vec::with_capacity(app.agents.len() + 1);
    items.push(ListItem::new(vec![
        Line::from(vec![
            Span::styled("▣ ", Style::default().fg(t.accent)),
            Span::styled("Dashboard", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("overview", Style::default().fg(t.dim))),
    ]));
    items.extend(app.agents.iter().map(|a| {
        let marker = if a.is_waiting() {
            Span::styled("● ", Style::default().fg(t.warn))
        } else if a.is_busy() {
            Span::styled("● ", Style::default().fg(t.accent))
        } else {
            Span::styled("· ", Style::default().fg(t.dim))
        };
        let mut name_spans = vec![
            marker,
            Span::styled(
                app.agent_name(a),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ];
        // Harpoon pin badge: the 1-based slot, so `<leader>N` is discoverable.
        if let Some(slot) = app.harpoon.iter().position(|s| s == &a.session_id) {
            name_spans.push(Span::styled(
                format!(" ⚓{}", slot + 1),
                Style::default().fg(t.accent),
            ));
        }
        let name = Line::from(name_spans);
        let stats = derive_stats(a, app.status_lines.get(&a.session_id));
        let meta = Line::from(Span::styled(
            meta_line(a, &stats),
            Style::default().fg(t.dim),
        ));
        ListItem::new(vec![name, meta])
    }));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(t.selection_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

pub(super) fn meta_line(a: &Agent, stats: &DerivedStats) -> String {
    let mut s = badge(a.state());
    if let Some(m) = &stats.model {
        s.push_str(&format!("  {}", short_model(m)));
    }
    if let Some(p) = stats.context_pct {
        s.push_str(&format!("  {p:.0}% ctx"));
    }
    if let Some(c) = stats.cost {
        s.push_str(&format!("  ${c:.2}"));
    }
    // No usage/statusLine yet — fall back to a raw tool-call count.
    if stats.model.is_none()
        && stats.context_pct.is_none()
        && stats.cost.is_none()
        && a.tool_calls > 0
    {
        s.push_str(&format!("  {} tools", a.tool_calls));
    }
    s
}

/// Trim the `claude-` prefix for a compact model label (e.g. `opus-4-8`).
pub(super) fn short_model(model: &str) -> &str {
    model.strip_prefix("claude-").unwrap_or(model)
}

pub(super) fn badge(state: &str) -> String {
    let s = if state.is_empty() { "idle" } else { state };
    s.to_lowercase()
}

/// Map a session state to a theme role color: waiting/input → warn, error → bad,
/// everything else → ok.
pub(super) fn state_color(t: &Theme, state: &str) -> Color {
    match state.to_lowercase().as_str() {
        "input" | "waiting" => t.warn,
        "error" => t.bad,
        _ => t.ok,
    }
}

/// Colour a rate-limit percentage: ok < 75% < warn < 90% < bad.
pub(super) fn rate_color(t: &Theme, pct: f64) -> Color {
    if pct >= 90.0 {
        t.bad
    } else if pct >= 75.0 {
        t.warn
    } else {
        t.ok
    }
}

// ── detail (list view right pane) ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(s: &str) -> Color {
        state_color(&Theme::default(), s)
    }

    fn ok() -> Color {
        Theme::default().ok
    }

    fn warn() -> Color {
        Theme::default().warn
    }

    fn bad() -> Color {
        Theme::default().bad
    }

    // ── modes that claudemon actually emits ──────────────────────────────────

    /// "input" — user's turn; renders as warn (amber), same as "waiting".
    #[test]
    fn state_color_input_is_warn() {
        assert_eq!(sc("input"), warn());
    }

    /// "approval" — tool approval pending; not in an explicit arm, falls to ok.
    #[test]
    fn state_color_approval_is_ok() {
        assert_eq!(sc("approval"), ok());
    }

    /// "question" — structured question; falls to ok.
    #[test]
    fn state_color_question_is_ok() {
        assert_eq!(sc("question"), ok());
    }

    /// "responding" — generating a turn; falls to ok.
    #[test]
    fn state_color_responding_is_ok() {
        assert_eq!(sc("responding"), ok());
    }

    /// "stopped" — session ended; falls to ok.
    #[test]
    fn state_color_stopped_is_ok() {
        assert_eq!(sc("stopped"), ok());
    }

    /// "unknown" — emitted by Agent::state() when mode is absent; falls to ok.
    #[test]
    fn state_color_unknown_is_ok() {
        assert_eq!(sc("unknown"), ok());
    }

    /// "other" — emitted by Agent::state() for any unrecognised AgentMode;
    /// falls to ok (catch-all).
    #[test]
    fn state_color_other_is_ok() {
        assert_eq!(sc("other"), ok());
    }

    // ── aliased and legacy strings ───────────────────────────────────────────

    /// "waiting" — alias for "input" in state_color(); also yields warn.
    #[test]
    fn state_color_waiting_alias_is_warn() {
        assert_eq!(sc("waiting"), warn());
    }

    /// "thinking", "running", "streaming" — daemon never emits these; they fall
    /// to the ok catch-all.
    #[test]
    fn state_color_removed_dead_branches_fall_to_ok() {
        assert_eq!(sc("thinking"), ok());
        assert_eq!(sc("running"), ok());
        assert_eq!(sc("streaming"), ok());
    }

    /// "error" — explicit bad arm is still present.
    #[test]
    fn state_color_error_is_bad() {
        assert_eq!(sc("error"), bad());
    }

    /// state_color() normalises to lowercase before matching.
    #[test]
    fn state_color_case_insensitive() {
        assert_eq!(sc("INPUT"), warn(), "uppercase INPUT must also be warn");
        assert_eq!(sc("Error"), bad(), "mixed-case Error must also be bad");
        assert_eq!(sc("STOPPED"), ok(), "uppercase STOPPED must also be ok");
    }

    /// Exhaustive table for all known inputs.
    #[test]
    fn state_color_table() {
        let cases: &[(&str, Color)] = &[
            ("input", warn()),
            ("approval", ok()),
            ("question", ok()),
            ("responding", ok()),
            ("stopped", ok()),
            ("unknown", ok()),
            ("other", ok()),
            ("waiting", warn()),
            ("thinking", ok()),
            ("running", ok()),
            ("streaming", ok()),
            ("error", bad()),
            ("", ok()),
            ("anything", ok()),
        ];
        for (state, want) in cases {
            assert_eq!(sc(state), *want, "state_color({state:?}) expected {want:?}");
        }
    }

    // ── pure label/format helpers ────────────────────────────────────────────
    //
    // These are the small pure functions the render fns lean on. They carry no
    // Frame and no App, so they are the part of this module that can be moved
    // wholesale during decomposition — which is exactly why their output is
    // pinned here first.

    fn agent_with(json: serde_json::Value) -> Agent {
        serde_json::from_value(json).expect("agent fixture")
    }

    fn stats(model: Option<&str>, ctx: Option<f64>, cost: Option<f64>) -> DerivedStats {
        DerivedStats {
            model: model.map(str::to_string),
            context_pct: ctx,
            cost,
        }
    }

    /// The sidebar meta line is a space-joined digest, and each part is
    /// independently optional — a session with only a model must not render
    /// stray separators for the context and cost it doesn't have.
    #[test]
    fn meta_line_joins_only_the_parts_it_has() {
        let a = agent_with(serde_json::json!({ "session_id": "s1", "mode": "responding" }));

        assert_eq!(
            meta_line(&a, &stats(Some("claude-opus-4-8"), Some(42.0), Some(1.5))),
            "responding  opus-4-8  42% ctx  $1.50"
        );
        assert_eq!(
            meta_line(&a, &stats(Some("claude-opus-4-8"), None, None)),
            "responding  opus-4-8"
        );
        assert_eq!(meta_line(&a, &stats(None, None, None)), "responding");
    }

    /// Context is a whole number of percent and cost is always two decimals —
    /// the column is narrow and a drifting width would ragged the sidebar.
    #[test]
    fn meta_line_rounds_context_and_pads_cost() {
        let a = agent_with(serde_json::json!({ "session_id": "s1", "mode": "responding" }));
        let line = meta_line(&a, &stats(None, Some(7.62), Some(2.0)));
        assert!(line.contains("8% ctx"), "rounded, not truncated: {line}");
        assert!(line.contains("$2.00"), "two decimals: {line}");
    }

    /// Before any usage or status line arrives there is still something worth
    /// showing: how many tools the session has run. It is a *fallback* — one
    /// real stat must suppress it, or the line says two things at once.
    #[test]
    fn meta_line_falls_back_to_a_tool_count_only_when_it_has_nothing_else() {
        let busy = agent_with(serde_json::json!({
            "session_id": "s1", "mode": "responding", "tool_calls": 7
        }));
        assert_eq!(
            meta_line(&busy, &stats(None, None, None)),
            "responding  7 tools"
        );

        // Any one real stat wins and the fallback stays quiet.
        let with_model = meta_line(&busy, &stats(Some("opus"), None, None));
        assert!(!with_model.contains("tools"), "got {with_model}");
        let with_cost = meta_line(&busy, &stats(None, None, Some(0.01)));
        assert!(!with_cost.contains("tools"), "got {with_cost}");

        // A session that has run nothing says nothing.
        let idle = agent_with(serde_json::json!({ "session_id": "s2", "mode": "input" }));
        assert_eq!(meta_line(&idle, &stats(None, None, None)), "input");
    }

    /// Only the `claude-` vendor prefix is noise; anything else is the label.
    #[test]
    fn short_model_strips_only_the_claude_prefix() {
        assert_eq!(short_model("claude-opus-4-8"), "opus-4-8");
        assert_eq!(short_model("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(short_model("opus-4-8"), "opus-4-8");
        assert_eq!(short_model("claude-"), "");
        assert_eq!(short_model(""), "");
        // Not a substring match — the prefix has to lead.
        assert_eq!(
            short_model("anthropic-claude-opus"),
            "anthropic-claude-opus"
        );
    }

    /// An empty state is "idle", not an empty badge, and the badge is always
    /// lowercase so a daemon that starts shouting can't restyle the sidebar.
    #[test]
    fn badge_defaults_to_idle_and_lowercases() {
        assert_eq!(badge(""), "idle");
        assert_eq!(badge("Responding"), "responding");
        assert_eq!(badge("ERROR"), "error");
        assert_eq!(badge("input"), "input");
    }

    /// Rate-limit colour thresholds, checked on the boundaries themselves —
    /// they are `>=`, so 75 and 90 are already the worse colour.
    #[test]
    fn rate_color_steps_at_seventy_five_and_ninety() {
        let t = Theme::default();
        let cases: &[(f64, Color)] = &[
            (0.0, t.ok),
            (74.9, t.ok),
            (75.0, t.warn),
            (89.9, t.warn),
            (90.0, t.bad),
            (100.0, t.bad),
            (250.0, t.bad),
        ];
        for (pct, want) in cases {
            assert_eq!(rate_color(&t, *pct), *want, "rate_color({pct})");
        }
    }
}
