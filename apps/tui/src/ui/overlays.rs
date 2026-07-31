//! Everything that floats over the frame: the spawn, rename and notes dialogs,
//! the palette / picker / search overlays, help, and which-key.
//!
//! Every one of these sizes itself through [`super::modal_rect`] — see the note
//! there about why clamping to the frame is not optional.

use super::*;

pub(super) fn render_rename(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.rename.as_ref() else {
        return;
    };
    let w = area.width.saturating_sub(8).clamp(20, 60).min(area.width);
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  name  ", Style::default().fg(t.dim)),
            Span::raw(form.input.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(Span::styled(
            format!(
                "  {}",
                crate::types::truncate(&form.cwd, w.saturating_sub(4) as usize)
            ),
            Style::default().fg(t.dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  enter save · empty clears · esc cancel",
            Style::default().fg(t.dim),
        )),
    ];
    let h = lines.len() as u16 + 2;
    let rect = modal_rect(area, w, h, ModalY::Centered);
    f.render_widget(ratatui::widgets::Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" rename agent ")
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── header ──────────────────────────────────────────────────────────────────

pub(super) fn render_spawn_modal(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(form) = app.spawn_form.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(8).clamp(20, 72).min(area.width);
    let inner_w = w.saturating_sub(2) as usize;

    let profile = app.profiles.get(form.profile_idx);
    let profile_name = profile.map(|p| p.name.as_str()).unwrap_or("Default");
    let n = app.profiles.len().max(1);
    let extra = profile
        .filter(|p| !p.extra_args.is_empty())
        .map(|p| format!("  ({})", p.extra_args.join(" ")))
        .unwrap_or_default();
    let providers = crate::app::SPAWN_PROVIDERS;
    let provider = providers
        .get(form.provider_idx)
        .copied()
        .unwrap_or("claude");
    let is_claude = provider == "claude";

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  cwd      ", Style::default().fg(t.dim)),
            Span::raw(form.cwd.clone()),
            Span::styled("▏", Style::default().fg(t.accent)),
        ]),
        Line::from(vec![
            Span::styled("  provider ", Style::default().fg(t.dim)),
            Span::styled("‹ ", Style::default().fg(t.accent)),
            Span::styled(
                provider.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ›", Style::default().fg(t.accent)),
            Span::styled(
                format!("  {}/{}", form.provider_idx + 1, providers.len()),
                Style::default().fg(t.dim),
            ),
        ]),
    ];
    // The profile picker only applies to claude (managed providers ignore it).
    if is_claude {
        lines.push(Line::from(vec![
            Span::styled("  profile  ", Style::default().fg(t.dim)),
            Span::styled("‹ ", Style::default().fg(t.accent)),
            Span::styled(
                profile_name.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ›", Style::default().fg(t.accent)),
            Span::styled(
                format!("  {}/{}", form.profile_idx + 1, n),
                Style::default().fg(t.dim),
            ),
            Span::styled(extra, Style::default().fg(t.dim)),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  managed session — profile not used",
            Style::default().fg(t.dim),
        )));
    }

    // Tab-completion candidates, when the path is ambiguous.
    if !form.completions.is_empty() {
        let joined = form.completions.join("  ");
        let shown = crate::types::truncate(&joined, inner_w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {} {}", form.completions.len(), "matches:"),
            Style::default().fg(t.dim),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {shown}"),
            Style::default().fg(t.accent),
        )));
    }

    // When seeding a library prompt, show what will be inserted.
    if let Some(prompt) = form.initial_prompt.as_ref() {
        let first = prompt.lines().next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled("  prompt   ", Style::default().fg(t.dim)),
            Span::styled(
                crate::types::truncate(first, inner_w.saturating_sub(12)),
                Style::default().fg(t.ok),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  type a path · tab complete · ←→ provider · ↑↓ profile · enter spawn · esc",
        Style::default().fg(t.dim),
    )));

    let h = lines.len() as u16 + 2;
    let rect = modal_rect(area, w, h, ModalY::Centered);
    // Clear underneath so the list doesn't bleed through.
    f.render_widget(ratatui::widgets::Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" new agent ")
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── command palette ─────────────────────────────────────────────────────────

pub(super) fn render_palette(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(p) = app.palette.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(8).clamp(24, 76).min(area.width);
    let max_rows = area.height.saturating_sub(6).clamp(3, 14);
    let visible: Vec<_> = p.visible().collect();
    let shown = (visible.len() as u16).min(max_rows);
    let h = shown + 4; // search line + borders + padding
    let rect = modal_rect(area, w, h, ModalY::Top(2));
    f.render_widget(ratatui::widgets::Clear, rect);

    let inner_w = w.saturating_sub(2) as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("› ", Style::default().fg(t.accent)),
        Span::raw(p.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    // Scroll the list so the selection stays visible.
    let start = p.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, item) in visible.iter().skip(start).take(shown as usize).enumerate() {
        let i = start + offset;
        let selected = i == p.selected;
        let marker = if selected { "❯ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(item.label.clone(), label_style),
        ];
        if !item.hint.is_empty() {
            let room = inner_w.saturating_sub(item.label.len() + 6);
            if room > 4 {
                spans.push(Span::styled(
                    format!("  {}", crate::types::truncate(&item.hint, room)),
                    Style::default().fg(t.dim),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches",
            Style::default().fg(t.dim),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" command palette ")
        .title_bottom(Line::from(Span::styled(
            " ↑↓ move · enter run · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The model / handoff-provider picker: a query line over a small filtered
/// list. The model picker also accepts free text (a model id not in the list).
pub(super) fn render_picker(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(p) = app.picker.as_ref() else { return };

    let w = area.width.saturating_sub(8).clamp(24, 64).min(area.width);
    let max_rows = area.height.saturating_sub(6).clamp(3, 12);
    let shown = (p.matched.len() as u16).min(max_rows);
    let h = shown + 5;
    let rect = modal_rect(area, w, h, ModalY::Top(2));
    f.render_widget(ratatui::widgets::Clear, rect);
    let inner_w = w.saturating_sub(2) as usize;

    let mut lines = vec![Line::from(vec![
        Span::styled("› ", Style::default().fg(t.accent)),
        Span::raw(p.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    let start = p.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, &mi) in p
        .matched
        .iter()
        .skip(start)
        .take(shown as usize)
        .enumerate()
    {
        let i = start + offset;
        let selected = i == p.selected;
        let marker = if selected { "❯ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(
                crate::types::truncate(&p.items[mi].label, inner_w.saturating_sub(4)),
                label_style,
            ),
        ]));
    }
    if p.pending {
        lines.push(Line::from(Span::styled(
            "  loading models…",
            Style::default().fg(t.dim),
        )));
    } else if p.matched.is_empty() {
        let hint = if p.allow_free_text {
            "type a model id and press enter"
        } else {
            "no matches"
        };
        lines.push(Line::from(Span::styled(
            format!("  {hint}"),
            Style::default().fg(t.dim),
        )));
    }

    let foot = if p.allow_free_text {
        " ↑↓ move · enter apply (or typed id) · esc close "
    } else {
        " ↑↓ move · enter apply · esc close "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", p.title))
        .title_bottom(Line::from(Span::styled(foot, Style::default().fg(t.dim))))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The cross-agent content-search modal: a query line plus matching transcript
/// lines (agent name + snippet), with an indexing-progress note in the title.
pub(super) fn render_search(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(s) = app.search.as_ref() else { return };

    let w = area.width.saturating_sub(8).clamp(36, 100).min(area.width);
    let max_rows = area.height.saturating_sub(6).clamp(3, 16);
    let shown = (s.matched.len() as u16).min(max_rows);
    let body_rows = if s.matched.is_empty() { 1 } else { shown };
    let h = body_rows + 4;
    let rect = modal_rect(area, w, h, ModalY::Top(2));
    f.render_widget(ratatui::widgets::Clear, rect);

    const NAME_COL: usize = 16;
    let inner_w = w.saturating_sub(2) as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled("/ ", Style::default().fg(t.accent)),
        Span::raw(s.query.clone()),
        Span::styled("▏", Style::default().fg(t.accent)),
    ])];

    let start = s.selected.saturating_sub(shown.saturating_sub(1) as usize);
    for (offset, &idx) in s
        .matched
        .iter()
        .skip(start)
        .take(shown as usize)
        .enumerate()
    {
        let i = start + offset;
        let hit = &s.entries[idx];
        let selected = i == s.selected;
        let marker = if selected { "❯ " } else { "  " };
        let room = inner_w.saturating_sub(NAME_COL + 4);
        let snippet = crate::types::truncate(hit.line.trim(), room.max(8));
        let snippet_style = if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(
                format!(
                    "{:<width$}",
                    crate::types::truncate(&hit.name, NAME_COL),
                    width = NAME_COL
                ),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("  {snippet}"), snippet_style),
        ]));
    }
    if s.query.is_empty() {
        lines.push(Line::from(Span::styled(
            "type to search every agent's transcript",
            Style::default().fg(t.dim),
        )));
    } else if s.matched.is_empty() {
        let msg = if s.pending > 0 {
            "indexing…"
        } else {
            "no matches"
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(t.dim))));
    }

    let title = if s.pending > 0 {
        format!(" search · indexing {} more… ", s.pending)
    } else {
        format!(" search · {} matches ", s.matched.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " ↑↓ move · enter open transcript · esc close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── help / keybindings overlay ────────────────────────────────────────────────

/// Friendly label for an action (snake_case → spaced).
pub(super) fn action_label(a: Action) -> String {
    a.name().replace('_', " ")
}

/// A two-column block of `chord  action` rows for one context, generated from
/// the live keymap so it can never drift from what the keys actually do.
pub(super) fn binding_lines(t: &Theme, app: &App, title: &str, ctx: Context) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ))];
    for (chord, action) in app.keymap.bindings(ctx) {
        out.push(Line::from(vec![
            Span::styled(format!("  {chord:<10}"), Style::default().fg(t.ok)),
            Span::styled(action_label(action), Style::default().fg(t.dim)),
        ]));
    }
    out.push(Line::raw(""));
    out
}

pub(super) fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let w = area.width.saturating_sub(6).clamp(24, 64).min(area.width);

    let mut lines = vec![Line::from(Span::styled(
        "Keybindings — edit ~/.config/workspacer/tui.json to remap",
        Style::default().fg(t.dim),
    ))];
    lines.push(Line::from(vec![
        Span::styled("press ", Style::default().fg(t.dim)),
        Span::styled(app.keymap.leader().display(), Style::default().fg(t.ok)),
        Span::styled(
            " for the leader menu (which-key)",
            Style::default().fg(t.dim),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.extend(binding_lines(t, app, "global", Context::Global));
    lines.extend(binding_lines(t, app, "sidebar / dashboard", Context::List));
    lines.extend(binding_lines(
        t,
        app,
        "agent · terminal",
        Context::AgentTerminal,
    ));
    lines.extend(binding_lines(
        t,
        app,
        "agent · transcript",
        Context::AgentTranscript,
    ));
    lines.push(Line::from(vec![
        Span::styled("answer keys ", Style::default().fg(t.dim)),
        Span::styled("1-9", Style::default().fg(t.ok)),
        Span::styled("  (positional, not remappable)", Style::default().fg(t.dim)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("themes: ", Style::default().fg(t.dim)),
        Span::styled(
            crate::theme::BUILTINS.join(", "),
            Style::default().fg(t.accent),
        ),
    ]));

    // Cap height to the viewport; the box scrolls via Paragraph if it overflows.
    // The -2 keeps a row of frame visible above and below when there is room.
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = modal_rect(area, w, h, ModalY::Centered);
    f.render_widget(ratatui::widgets::Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" help ")
        .title_bottom(Line::from(Span::styled(
            " any key to close ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The which-key popup: when a multi-key sequence is in flight (e.g. after the
/// leader), float a box listing the chords that can come next and what they do.
/// Renders nothing when no sequence is pending or the prefix is a dead end.
pub(super) fn render_whichkey(f: &mut Frame, area: Rect, app: &App) {
    if app.pending_keys.is_empty() {
        return;
    }
    let t = &app.theme;
    let ctxs = [Context::Global, app.key_context()];
    let conts = app.keymap.continuations(&ctxs, &app.pending_keys);
    if conts.is_empty() {
        return;
    }

    let mut rows: Vec<Line> = conts
        .iter()
        .map(|c| {
            let key = c.chord.display();
            let label = match c.action {
                Some(a) => action_label(a),
                None => "▸ …".to_string(),
            };
            Line::from(vec![
                Span::styled(
                    format!(" {key:<7}"),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(label, Style::default().fg(t.fg)),
            ])
        })
        .collect();
    // The positional harpoon jumps (`<leader>1..9`) aren't in the keymap, so
    // surface them as a hint when the leader prefix is up and pins exist.
    if app.pending_keys == [app.keymap.leader()] && !app.harpoon.is_empty() {
        rows.push(Line::from(vec![
            Span::styled(
                format!(" {:<7}", "1-9"),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("jump to pinned agent (⚓1-{})", app.harpoon.len()),
                Style::default().fg(t.fg),
            ),
        ]));
    }

    let prefix = crate::keys::display_seq(&app.pending_keys);
    let title = format!(" {prefix}… ");
    let inner_w = rows
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(title.chars().count()) as u16;
    // `.max` then clamp, never `clamp(16, area.width - 2)`: on a terminal
    // narrower than 18 columns that lower bound exceeds the upper one and
    // `clamp` itself panics.
    let w = (inner_w + 2).max(16).min(area.width.saturating_sub(2));
    let h = (rows.len() as u16 + 2).min(area.height.saturating_sub(1));
    // Bottom-anchored, like which-key.nvim — out of the way of the content.
    let rect = modal_rect(area, w, h, ModalY::Bottom);
    f.render_widget(ratatui::widgets::Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            " esc cancel ",
            Style::default().fg(t.dim),
        )))
        .border_style(Style::default().fg(t.accent));
    f.render_widget(Paragraph::new(rows).block(block), rect);
}

// ── notes scratchpad ──────────────────────────────────────────────────────────

pub(super) fn render_notes(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(n) = app.notes_view.as_ref() else {
        return;
    };

    let w = area.width.saturating_sub(6).clamp(24, 76).min(area.width);
    let h = area.height.saturating_sub(4).clamp(6, 24);
    let rect = modal_rect(area, w, h, ModalY::Centered);
    f.render_widget(ratatui::widgets::Clear, rect);

    let mode = if n.editing { "editing" } else { "notes" };
    let bottom = if n.editing {
        " esc save · enter newline "
    } else {
        " i edit · j/k scroll · esc close "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {mode} · {} ",
            crate::types::truncate(&n.cwd, w.saturating_sub(12) as usize)
        ))
        .title_bottom(Line::from(Span::styled(bottom, Style::default().fg(t.dim))))
        .border_style(Style::default().fg(t.accent));

    let body = if n.text.is_empty() && !n.editing {
        Paragraph::new(Line::from(Span::styled(
            "empty — press i to write",
            Style::default().fg(t.dim),
        )))
    } else {
        // Show a trailing cursor while editing.
        let text = if n.editing {
            format!("{}▏", n.text)
        } else {
            n.text.clone()
        };
        Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((n.scroll, 0))
    };
    f.render_widget(body.block(block), rect);
}

// ── docked side pane ──────────────────────────────────────────────────────────
