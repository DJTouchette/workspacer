//! ratatui rendering. One entry point, [`render`], takes `&mut App` because it
//! resolves the transcript's follow-to-bottom flag into a concrete scroll
//! offset clamped to the content height — the only state the renderer writes.
//!
//! Colors come from `app.theme` (see `theme.rs`); no widget references a literal
//! color. Leaf helpers that don't get an `&App` take an explicit `&Theme`.
//!
//! Each screen lives in its own child module; they pick up this module's
//! imports via `use super::*`, since a private `use` is visible to descendants
//! (the same way a `mod tests` block sees them). This file keeps the entry
//! point, the shared modal geometry, and the text wrapper.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, ChatMode, SplitDir, TabKind, View};
use crate::keys::{Action, Context};
use crate::theme::Theme;
use crate::types::{derive_stats, window_short, Agent, DerivedStats, Part, Role};
use serde_json::Value;
use tui_term::widget::PseudoTerminal;

mod chat;
mod chrome;
mod dashboard;
mod detail;
mod overlays;
mod panes;
mod review;
mod runs;
mod sidebar;

use chat::*;
use chrome::*;
use dashboard::*;
use detail::*;
use overlays::*;
use panes::*;
use review::*;
use runs::*;
use sidebar::*;

/// Render-level characterization tests, kept in their own file so this module
/// stays navigable; a child module so they still reach the private `render_*`
/// fns and helpers via `super::*`.
#[cfg(test)]
#[path = "../ui_render_tests.rs"]
mod render_tests;

pub fn render(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_header(f, root[0], app);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(20)])
        .split(root[1]);

    render_sidebar(f, body[0], app);
    // A docked pane splits the content column: the agent keeps the larger share,
    // because the conversation is still the thing you are reading.
    let (content, side) = match app.side.as_ref() {
        Some(_) if app.runs_open.is_none() => {
            let cells = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(body[1]);
            (cells[0], Some(cells[1]))
        }
        _ => (body[1], None),
    };
    if app.runs_open.is_some() {
        // The runs overlay takes the whole content column.
        render_runs(f, content, app);
    } else {
        match &app.view {
            View::List if app.dashboard_selected() => render_dashboard(f, content, app),
            View::List => render_detail(f, content, app),
            View::Agent { .. } => render_panes(f, content, app),
        }
    }
    if let Some(area) = side {
        render_side_pane(f, area, app);
    }

    render_footer(f, root[2], app);

    // Modals float over everything when open.
    if app.spawn_form.is_some() {
        render_spawn_modal(f, f.area(), app);
    }
    if app.palette.is_some() {
        render_palette(f, f.area(), app);
    }
    if app.picker.is_some() {
        render_picker(f, f.area(), app);
    }
    if app.search.is_some() {
        render_search(f, f.area(), app);
    }
    if app.rename.is_some() {
        render_rename(f, f.area(), app);
    }
    if app.notes_view.is_some() {
        render_notes(f, f.area(), app);
    }
    if app.help {
        render_help(f, f.area(), app);
    }
    // The which-key popup floats whenever a multi-key sequence is mid-flight.
    render_whichkey(f, root[1], app);
}

/// Where a modal sits vertically in the frame.
enum ModalY {
    /// Centred — most dialogs.
    Centered,
    /// A fixed inset from the top. The query-style overlays sit high so their
    /// result list has room beneath it.
    Top(u16),
    /// Flush to the bottom, out of the way of the content (which-key).
    Bottom,
}

/// A modal box of at most `want_w` × `want_h`, placed in `area` and guaranteed
/// to fit inside it.
///
/// The clamping is load-bearing, not cosmetic: ratatui panics when a widget's
/// rect leaves the buffer, so an overlay that insists on its preferred size
/// takes the whole TUI down on a short terminal. Every overlay in this module
/// goes through here — `ui_render_tests` draws each one at sizes down to 1×1
/// to keep it that way.
fn modal_rect(area: Rect, want_w: u16, want_h: u16, y: ModalY) -> Rect {
    let width = want_w.min(area.width);
    let height = want_h.min(area.height);
    let slack = area.height.saturating_sub(height);
    let dy = match y {
        ModalY::Centered => slack / 2,
        ModalY::Top(inset) => inset.min(slack),
        ModalY::Bottom => slack,
    };
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + dy,
        width,
        height,
    }
}

/// Greedy word-wrap to `width` display columns, hard-splitting tokens longer
/// than the line. Display-width-aware (wide glyphs count 2) — see
/// [`crate::render::wrap`].
fn wrap(s: &str, width: usize) -> Vec<String> {
    crate::render::wrap_plain(s, width)
}

// ── state_color characterization tests ──────────────────────────────────────

/// Shared by the per-screen test modules: the plain text of each rendered
/// line, with styling dropped.
#[cfg(test)]
mod testutil {
    use ratatui::text::Line;

    pub(in crate::ui) fn line_texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `wrap` is this module's seam onto the shared renderer. Pin the delegation
    /// so a decomposition that re-homes it can't quietly swap the algorithm.
    #[test]
    fn wrap_delegates_to_the_shared_display_width_wrapper() {
        assert_eq!(
            wrap("hello world", 5),
            crate::render::wrap_plain("hello world", 5)
        );
        assert_eq!(wrap("alpha beta", 20), vec!["alpha beta".to_string()]);
        // Wide glyphs count two columns.
        assert_eq!(wrap("日本語", 4), crate::render::wrap_plain("日本語", 4));
    }
}
