//! The remote-node surface: which machines the hub knows about, what state
//! each is in, and the two-step wake.
//!
//! A node is a machine that can be off ON PURPOSE. Before this, a machine that
//! was asleep and one that had died looked identical from the TUI: nothing.
//!
//! Two rules govern everything drawn here, both from
//! `.workspacer/reports/2026-08-24-fly-wake-contract.md`:
//!
//!  - **`waking` is not `unreachable`, and neither is `stopping`.** A machine
//!    takes real seconds to boot and real seconds to drain; a state that reads
//!    the same as a hang is what makes someone give up. The five states get
//!    five marks and five words, and the two transitional ones wear the ACCENT
//!    (working) tone a thinking agent uses.
//!  - **The cost goes on the screen.** There is no hover in a terminal, so
//!    every reason — why a wake is offered, why it is refused, what it will
//!    spend — is a visible line, never a tooltip.
//!
//! The dashboard line ([`nodes_dashboard_line`]) stays SILENT when every node
//! is quietly fine, mirroring the desktop strip: a permanent "all good" row is
//! chrome nobody asked for. The overlay, which someone explicitly asked for,
//! always answers — including "this hub has no remote nodes".

use super::*;
use crate::nodes::{crash_notice, detail_line, wake_affordance, wake_failure_notice, NodeState};

/// The colour a state paints with. `waking` and `stopping` deliberately share
/// the accent (working) tone rather than the warning one — a booting machine
/// that paints like a failure is the whole bug, and a shutdown somebody just
/// asked for painted as a fault is the same bug in the other direction.
fn state_style(t: &Theme, state: NodeState) -> Style {
    Style::default().fg(match state {
        NodeState::Available => t.ok,
        NodeState::Waking | NodeState::Stopping => t.accent,
        NodeState::Stopped => t.dim,
        NodeState::Unreachable => t.warn,
    })
}

/// One line for the dashboard when — and only when — some node is not quietly
/// fine. An unreachable or sleeping machine is WHY dispatches are missing from
/// the fleet below it, which is the whole reason this belongs on the overview.
pub(super) fn nodes_dashboard_line(app: &App) -> Option<Line<'static>> {
    let reg = app.nodes.as_ref()?;
    let attention = reg.needing_attention();
    if attention.is_empty() {
        return None;
    }
    let t = &app.theme;
    let mut spans = vec![Span::styled("nodes ", Style::default().fg(t.dim))];
    for (i, n) in attention.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(t.dim)));
        }
        spans.push(Span::styled(
            format!("{} {} {}", n.state.marker(), n.label, n.state.label()),
            state_style(t, n.state),
        ));
    }
    spans.push(Span::styled(
        format!("   {} nodes", app.keymap.leader().display()),
        Style::default().fg(t.dim),
    ));
    Some(Line::from(spans))
}

pub(super) fn render_nodes(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(view) = app.nodes_view.as_ref() else {
        return;
    };
    let w = area.width.saturating_sub(6).clamp(24, 76).min(area.width);
    // The block's interior, and the two indents everything below the identity
    // row hangs off. Wrapping must subtract the INDENT, not just the border: a
    // line wrapped to the full interior and then indented overflows, and
    // ratatui silently clips the tail — which on this surface would mean the
    // cost sentence losing its last words.
    let inner_w = w.saturating_sub(2) as usize;
    let body_w = inner_w.saturating_sub(4);
    let note_w = inner_w.saturating_sub(6);
    let can_wake = app.can_wake_nodes();
    let rows = app.node_rows();

    let mut lines: Vec<Line<'static>> = Vec::new();
    if rows.is_empty() {
        // The honest answer for almost every install. `nodes.list` sits in the
        // VIEW tier but the hub only REGISTERS it when a nodes.json exists, so
        // "no registry" and "empty registry" both land here and both mean the
        // same thing to a person.
        for l in wrap(
            "this hub has no remote nodes. a node is a machine that can be off on              purpose — the hub keeps them in nodes.json.",
            inner_w.saturating_sub(2),
        ) {
            lines.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(t.dim),
            )));
        }
    }
    for (i, node) in rows.iter().enumerate() {
        let selected = i == view.selected;
        let pending = view.pending.contains(&node.id);
        let cursor = if selected { "▸ " } else { "  " };
        // Identity row: which machine, and what it is doing.
        lines.push(Line::from(vec![
            Span::styled(
                cursor.to_string(),
                Style::default().fg(if selected { t.accent } else { t.dim }),
            ),
            Span::styled(
                format!("{} ", node.state.marker()),
                state_style(t, node.state),
            ),
            Span::styled(
                crate::types::truncate(&node.label, body_w.saturating_sub(14)),
                Style::default().fg(t.fg).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            Span::styled(
                format!("  {}", node.state.label()),
                state_style(t, node.state),
            ),
        ]));
        // The hub's own sentence — it is written to be read by a person.
        for l in wrap(detail_line(node), body_w) {
            lines.push(Line::from(Span::styled(
                format!("    {l}"),
                Style::default().fg(t.dim),
            )));
        }
        // The node telling you its last run crashed. The ONLY notice anyone
        // gets, and it arrives one wake late by construction.
        if let Some(notice) = crash_notice(node) {
            for l in wrap(&notice, body_w) {
                lines.push(Line::from(Span::styled(
                    format!("    {l}"),
                    Style::default().fg(t.warn),
                )));
            }
        }
        // Failed wakes: a machine that started and never became usable.
        if let Some(notice) = wake_failure_notice(node) {
            for l in wrap(&notice, body_w) {
                lines.push(Line::from(Span::styled(
                    format!("    {l}"),
                    Style::default().fg(t.warn),
                )));
            }
        }
        if let Some(err) = view.errors.get(&node.id) {
            for l in wrap(err, body_w) {
                lines.push(Line::from(Span::styled(
                    format!("    {l}"),
                    Style::default().fg(t.bad),
                )));
            }
        }
        // The confirmation step, when this is the node it is armed for. It
        // names the CONSEQUENCE, not the action, and it REPLACES the offer
        // rather than sitting under it: the cost sentence printed twice reads
        // as two separate warnings and dilutes the one that matters.
        if view.confirm.as_deref() == Some(node.id.as_str()) {
            lines.push(Line::from(Span::styled(
                format!("    wake {}?", node.label),
                Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
            )));
            for l in wrap(crate::nodes::WAKE_COST_NOTE, note_w) {
                lines.push(Line::from(Span::styled(
                    format!("      {l}"),
                    Style::default().fg(t.warn),
                )));
            }
            lines.push(Line::from(vec![
                Span::styled(
                    "      y",
                    Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " confirm · any other key cancels",
                    Style::default().fg(t.dim),
                ),
            ]));
            lines.push(Line::raw(""));
            continue;
        }
        // The action and its cost, or the reason it isn't on offer — never a
        // control that would be refused, and never one that hides its price.
        // A REFUSED action prints its reason alone: the verb it cannot reach
        // is already implied, and repeating the state ("starting…") beside a
        // chip that just said it is noise on a screen that has to stay read.
        let affordance = wake_affordance(node, can_wake, pending);
        if affordance.visible {
            if affordance.enabled {
                let style = if selected {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.accent)
                };
                lines.push(Line::from(Span::styled(
                    format!("    {}", affordance.label),
                    style,
                )));
                for l in wrap(affordance.reason, note_w) {
                    lines.push(Line::from(Span::styled(
                        format!("      {l}"),
                        Style::default().fg(t.dim),
                    )));
                }
            } else {
                for l in wrap(affordance.reason, body_w) {
                    lines.push(Line::from(Span::styled(
                        format!("    {l}"),
                        Style::default().fg(t.dim),
                    )));
                }
            }
        }
        lines.push(Line::raw(""));
    }

    // Cap to the viewport; the -2 keeps a row of frame visible above and below
    // when there is room. Everything goes through `modal_rect` — see the note
    // there about why clamping to the frame is not optional.
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = modal_rect(area, w, h, ModalY::Centered);
    f.render_widget(ratatui::widgets::Clear, rect);
    // The title carries the roster at a glance, so a collapsed reading ("1
    // asleep · 2 connected") is available without walking the rows.
    let summary = app
        .nodes
        .as_ref()
        .map(crate::nodes::NodeRegistry::summary)
        .filter(|s| !s.is_empty())
        .map(|s| format!(" remote nodes · {s} "))
        .unwrap_or_else(|| " remote nodes ".to_string());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(summary)
        .title_bottom(Line::from(Span::styled(
            " j/k select · w wake · r refresh · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}
