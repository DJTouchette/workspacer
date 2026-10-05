//! The pop-up terminal: each agent's own interactive shell, opened in the
//! agent's folder on the hub's machine, docked under the conversation or
//! popped out into a window of its own. Hiding it keeps the shell running;
//! reopening the agent's terminal attaches to the same shell and replays its
//! screen. See `wks_native::terminal`.
use super::*;
use gpui::{
    AnyElement, FontStyle, FontWeight, Keystroke, Pixels, StyledText, TextRun, UnderlineStyle,
};
use std::collections::BTreeMap;
use wks_native::terminal::{self as term, Emulator, Status};

/// Panel height as a share of the window, normal and enlarged.
const PANEL_SHARE: f32 = 0.4;
const PANEL_SHARE_TALL: f32 = 0.72;
/// Wheel lines per notch through scrollback.
const WHEEL_LINES: f32 = 3.;

#[derive(Default)]
pub(super) struct TerminalUi {
    pub open: bool,
    pub tall: bool,
    /// The agent whose terminal the panel shows.
    pub agent: Option<String>,
    views: HashMap<String, Entity<TerminalView>>,
    /// agent → shell id for this hub, mirrored to disk.
    remembered: BTreeMap<String, String>,
    loaded: bool,
    adopted: bool,
    /// A terminal in its own window, and the agent it belongs to.
    popout: Option<(gpui::WindowHandle<gpui_component::Root>, String)>,
}

pub(super) struct TerminalView {
    agent: String,
    controller: Controller,
    emulator: Emulator,
    /// The attach whose replay this emulator holds.
    attach: u64,
    focus: FocusHandle,
    /// Cell width and line height at the current font size.
    cell: (Pixels, Pixels),
    font_size: f32,
    /// Size last sent to the hub.
    sent: (u16, u16),
    error: Option<String>,
}

impl TerminalView {
    fn new(agent: String, controller: Controller, cx: &mut Context<Self>) -> Self {
        Self {
            agent,
            controller,
            emulator: Emulator::new(term::DEFAULT_ROWS, term::DEFAULT_COLS),
            attach: 0,
            focus: cx.focus_handle(),
            cell: (px(8.), px(17.)),
            font_size: 13.,
            sent: (0, 0),
            error: None,
        }
    }

    pub(super) fn has_focus(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub(super) fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub(super) fn text(&self) -> String {
        self.emulator.text()
    }

    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        // Typing returns to the live screen.
        if self.emulator.scroll > 0 {
            self.emulator.scroll_by(-(self.emulator.scroll as isize));
        }
        let result = self
            .controller
            .command(Command::Terminal(term::Command::Input {
                agent: self.agent.clone(),
                bytes,
            }));
        self.error = result.err().map(|e| e.to_string());
        cx.notify();
    }

    /// A key while the terminal has focus. `false` leaves it to the app
    /// (its few reserved shortcuts and Quit).
    pub(super) fn keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let m = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        let paste = (m.control && m.shift && key == "v") || (m.platform && key == "v");
        let copy = (m.control && m.shift && key == "c") || (m.platform && key == "c");
        if paste {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                let bracketed = self.emulator.screen().bracketed_paste();
                self.send(term::paste_bytes(&text, bracketed), cx);
            }
            return true;
        }
        if copy {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.emulator.text()));
            return true;
        }
        // Reserved for the app: hide the terminal, Quit, open the editor.
        if (m.control || m.platform) && key == "`"
            || (m.platform && key == "q")
            || (m.control && m.shift && matches!(key, "q" | "e"))
        {
            return false;
        }
        let application_cursor = self.emulator.screen().application_cursor();
        match term::key_bytes(
            key,
            keystroke.key_char.as_deref(),
            m.control,
            m.alt,
            m.shift,
            m.platform,
            application_cursor,
        ) {
            Some(bytes) => {
                self.send(bytes, cx);
                true
            }
            None => !m.platform,
        }
    }

    /// Apply the controller's state and queued output.
    fn sync(&mut self, state: Option<&term::Terminal>, feed: &term::Feed, cx: &mut Context<Self>) {
        let Some(state) = state else {
            return;
        };
        let (cols, rows) = self.size();
        if state.attach != self.attach {
            self.attach = state.attach;
            self.emulator = Emulator::new(rows, cols);
        }
        if let Some(shell) = &state.shell
            && let Some(chunk) = feed.take(shell)
        {
            if chunk.reset {
                self.emulator = Emulator::new(rows, cols);
            }
            self.emulator.process(&chunk.bytes);
            cx.notify();
        }
    }

    fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.emulator.size();
        (cols, rows)
    }

    /// Fit the grid to the painted area and tell the hub.
    fn fit(
        &mut self,
        bounds: gpui::Bounds<Pixels>,
        cell: (Pixels, Pixels),
        cx: &mut Context<Self>,
    ) {
        self.cell = cell;
        let cols = (f32::from(bounds.size.width) / f32::from(cell.0))
            .floor()
            .max(2.) as u16;
        let rows = (f32::from(bounds.size.height) / f32::from(cell.1))
            .floor()
            .max(1.) as u16;
        if (cols, rows) == self.sent {
            return;
        }
        self.sent = (cols, rows);
        self.emulator.resize(rows, cols);
        let _ = self
            .controller
            .command(Command::Terminal(term::Command::Resize {
                agent: self.agent.clone(),
                cols,
                rows,
            }));
        cx.notify();
    }

    fn wheel(&mut self, event: &gpui::ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(self.cell.1);
        let lines =
            (f32::from(delta.y) / f32::from(self.cell.1) * WHEEL_LINES / 3.).round() as isize;
        if lines == 0 {
            return;
        }
        if self.emulator.screen().alternate_screen() {
            // Full-screen programs (less, vim) scroll with the arrow keys.
            let key = if lines > 0 { "up" } else { "down" };
            let app = self.emulator.screen().application_cursor();
            let mut bytes = vec![];
            for _ in 0..lines.unsigned_abs().min(10) {
                bytes.extend(term::key_bytes(key, None, false, false, false, false, app).unwrap());
            }
            self.send(bytes, cx);
        } else {
            self.emulator.scroll_by(lines);
            cx.notify();
        }
        let _ = window;
    }

    fn color(&self, color: vt100::Color, p: Palette, foreground: bool) -> Option<gpui::Hsla> {
        match color {
            vt100::Color::Default => None,
            vt100::Color::Idx(i) if i < 16 => Some(rgb(ansi(i, p)).into()),
            vt100::Color::Idx(i) => Some(rgb(term::xterm_color(i)).into()),
            vt100::Color::Rgb(r, g, b) => {
                Some(rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into())
            }
        }
        .or_else(|| foreground.then(|| rgb(p.text).into()))
    }
}

/// ANSI colours from the selected theme: red, green, yellow and blue are its
/// error/success/warning/accent tones and white is its text; the remaining
/// slots use VS Code's terminal palette for the theme's lightness.
fn ansi(index: u8, p: Palette) -> u32 {
    match index & 15 {
        1 | 9 => return p.error,
        2 | 10 => return p.success,
        3 | 11 => return p.warning,
        4 | 12 => return p.accent,
        7 => return p.prose,
        15 => return p.text,
        8 => return p.muted,
        _ => {}
    }
    let light = term::is_light(p.code_block);
    const DARK: [u32; 16] = [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    const LIGHT: [u32; 16] = [
        0x000000, 0xcd3131, 0x00bc00, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, 0x666666,
        0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
    ];
    (if light { LIGHT } else { DARK })[index as usize & 15]
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let family = gpui_component::Theme::global(cx).mono_font_family.clone();
        let p = cx.global::<TerminalPalette>().0;
        let font = gpui::font(family.clone());
        let measure = font.clone();
        let size = px(self.font_size);
        let line = px((self.font_size * 1.35).round());
        let focused = self.focus.is_focused(window);
        let screen_rows = self.emulator.rows();
        let (cursor_row, cursor_col) = self.emulator.screen().cursor_position();
        let show_cursor = !self.emulator.screen().hide_cursor() && self.emulator.scroll == 0;
        let rows = screen_rows.into_iter().map(|runs| {
            let mut text = String::new();
            let mut styled = Vec::with_capacity(runs.len());
            for run in runs {
                let (fg, bg) = if run.style.inverse {
                    (
                        self.color(run.style.bg, p, false)
                            .unwrap_or(rgb(p.code_block).into()),
                        Some(
                            self.color(run.style.fg, p, true)
                                .unwrap_or(rgb(p.text).into()),
                        ),
                    )
                } else {
                    (
                        self.color(run.style.fg, p, true)
                            .unwrap_or(rgb(p.text).into()),
                        self.color(run.style.bg, p, false),
                    )
                };
                let mut font = font.clone();
                if run.style.bold {
                    font.weight = FontWeight::BOLD;
                }
                if run.style.italic {
                    font.style = FontStyle::Italic;
                }
                text.push_str(&run.text);
                styled.push(TextRun {
                    len: run.text.len(),
                    font,
                    color: fg,
                    background_color: bg,
                    underline: run.style.underline.then(|| UnderlineStyle {
                        thickness: px(1.),
                        color: Some(fg),
                        wavy: false,
                    }),
                    strikethrough: None,
                });
            }
            div()
                .h(line)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(StyledText::new(text).with_runs(styled))
        });
        let view = cx.entity().downgrade();
        let cell = self.cell;
        let cursor = show_cursor.then(|| {
            div()
                .absolute()
                .left(cell.0 * cursor_col as f32)
                .top(cell.1 * cursor_row as f32)
                .w(cell.0)
                .h(cell.1)
                .when(focused, |d| {
                    d.bg(gpui::Hsla::from(rgb(p.accent)).opacity(0.55))
                })
                .when(!focused, |d| d.border_1().border_color(rgb(p.accent)))
        });
        div()
            .id("terminal-view")
            .debug_selector(|| "terminal-view".into())
            .key_context("Terminal")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family(family)
            .text_size(size)
            .line_height(line)
            .text_color(rgb(p.text))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                this.wheel(event, window, cx);
                cx.stop_propagation();
            }))
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        let font_id = window.text_system().resolve_font(&measure);
                        let width = window
                            .text_system()
                            .advance(font_id, size, 'M')
                            .map(|s| s.width)
                            .unwrap_or(size * 0.6);
                        let cell = (width, line);
                        cx.defer(move |cx| {
                            let _ = view.update(cx, |this, cx| this.fit(bounds, cell, cx));
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(rows)
            .children(cursor)
    }
}

/// The palette terminals draw with (set by the workspace on theme changes).
#[derive(Clone, Copy)]
pub(super) struct TerminalPalette(pub Palette);
impl gpui::Global for TerminalPalette {}

impl Workspace {
    /// The selected agent's terminal, focused, or hidden again.
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.open {
            self.hide_terminal(window, cx);
        } else {
            self.show_terminal(window, cx);
        }
    }

    pub(super) fn show_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session().cloned() else {
            self.extras.notice = "Select a session to open its terminal.".into();
            cx.notify();
            return;
        };
        if self.terminal_popped_out(&session.id)
            && let Some((handle, _)) = &self.terminal.popout
        {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        self.terminal.open = true;
        self.terminal.agent = Some(session.id.clone());
        self.screen = Screen::Conversation;
        self.start_terminal(&session.id, &session.cwd, cx);
        if let Some(view) = self.terminal.views.get(&session.id) {
            view.read(cx).focus(window);
        }
        cx.notify();
    }

    /// Ask the hub for the agent's shell (an existing one is re-attached).
    fn start_terminal(&mut self, agent: &str, cwd: &str, cx: &mut Context<Self>) {
        self.set_terminal_palette(cx);
        let controller = self.controller.clone();
        let view = self
            .terminal
            .views
            .entry(agent.to_owned())
            .or_insert_with(|| cx.new(|cx| TerminalView::new(agent.to_owned(), controller, cx)))
            .clone();
        let (cols, rows) = {
            let view = view.read(cx);
            if view.sent.0 > 0 {
                view.sent
            } else {
                (term::DEFAULT_COLS, term::DEFAULT_ROWS)
            }
        };
        self.command(
            Command::Terminal(term::Command::Open {
                agent: agent.to_owned(),
                cwd: cwd.to_owned(),
                cols,
                rows,
            }),
            cx,
        );
    }

    pub(super) fn hide_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.terminal_has_focus(window, cx);
        self.terminal.open = false;
        if let Some(agent) = self.terminal.agent.clone()
            && !self.terminal_popped_out(&agent)
        {
            self.command(Command::Terminal(term::Command::Hide { agent }), cx);
        }
        if focused {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    fn restart_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.terminal.agent.clone() else {
            return;
        };
        let cwd = self
            .view
            .sessions
            .iter()
            .find(|s| s.id == agent)
            .map(|s| s.cwd.clone())
            .or_else(|| self.view.terminals.get(&agent).map(|t| t.cwd.clone()))
            .unwrap_or_default();
        let (cols, rows) = self
            .terminal
            .views
            .get(&agent)
            .map(|v| v.read(cx).sent)
            .filter(|s| s.0 > 0)
            .unwrap_or((term::DEFAULT_COLS, term::DEFAULT_ROWS));
        self.command(
            Command::Terminal(term::Command::Restart {
                agent: agent.clone(),
                cwd,
                cols,
                rows,
            }),
            cx,
        );
        if let Some(view) = self.terminal.views.get(&agent) {
            view.read(cx).focus(window);
        }
    }

    pub(super) fn terminal_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.terminal
            .agent
            .as_ref()
            .and_then(|a| self.terminal.views.get(a))
            .is_some_and(|v| v.read(cx).has_focus(window))
    }

    /// Keys typed into a focused terminal go to its shell, ahead of every
    /// binding; `true` when the terminal took the key.
    pub(super) fn terminal_keystroke(
        &mut self,
        keystroke: &Keystroke,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.terminal.open || !self.terminal_has_focus(window, cx) {
            return false;
        }
        let Some(view) = self
            .terminal
            .agent
            .as_ref()
            .and_then(|a| self.terminal.views.get(a))
            .cloned()
        else {
            return false;
        };
        view.update(cx, |view, cx| view.keystroke(keystroke, cx))
    }

    fn set_terminal_palette(&self, cx: &mut App) {
        cx.set_global(TerminalPalette(self.appearance.palette()));
    }

    /// Hand new output to each terminal, follow the selected agent, and keep
    /// the remembered shells current.
    pub(super) fn sync_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.terminal.loaded {
            self.terminal.loaded = true;
            if let Some(path) = &self.settings_path {
                self.terminal.remembered =
                    term::load_remembered(&term::remembered_path(path, &self.project_scope));
            }
        }
        if self.view.connected && !self.terminal.adopted {
            self.terminal.adopted = true;
            if !self.terminal.remembered.is_empty() {
                let known = self.terminal.remembered.clone();
                self.command(Command::Terminal(term::Command::Adopt(known)), cx);
            }
        }
        let feed = self.view.terminal_feed.clone();
        for (agent, view) in &self.terminal.views {
            let state = self.view.terminals.get(agent);
            view.update(cx, |view, cx| view.sync(state, &feed, cx));
        }
        // The panel follows the selected agent: its own shell, if it has one.
        if self.terminal.open && self.view.selected != self.terminal.agent {
            let focused = self.terminal_has_focus(window, cx);
            if let Some(agent) = self.terminal.agent.take()
                && !self.terminal_popped_out(&agent)
            {
                self.command(Command::Terminal(term::Command::Hide { agent }), cx);
            }
            self.terminal.agent = self.view.selected.clone();
            if let Some(session) = self.selected_session().cloned()
                && self
                    .view
                    .terminals
                    .get(&session.id)
                    .is_some_and(term::Terminal::running)
            {
                self.start_terminal(&session.id, &session.cwd, cx);
                if focused && let Some(view) = self.terminal.views.get(&session.id) {
                    view.read(cx).focus(window);
                }
            }
        }
        // Persist agent → shell for this hub.
        let mut remembered = self.terminal.remembered.clone();
        for (agent, state) in &self.view.terminals {
            match (&state.shell, state.status) {
                (Some(shell), _) => {
                    remembered.insert(agent.clone(), shell.clone());
                }
                (None, Status::Exited | Status::Failed) => {
                    remembered.remove(agent);
                }
                _ => {}
            }
        }
        while remembered.len() > term::MAX_REMEMBERED {
            remembered.pop_first();
        }
        if remembered != self.terminal.remembered {
            self.terminal.remembered = remembered;
            if let Some(path) = &self.settings_path
                && let Err(error) = term::save_remembered(
                    &term::remembered_path(path, &self.project_scope),
                    &self.terminal.remembered,
                )
            {
                eprintln!("Could not save native terminal sessions: {error}");
            }
        }
    }

    /// The docked terminal under the conversation, when open.
    pub(super) fn render_terminal_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.terminal.open {
            return None;
        }
        let p = self.appearance.palette();
        self.set_terminal_palette(cx);
        let agent = self.terminal.agent.clone()?;
        let session = self.view.sessions.iter().find(|s| s.id == agent).cloned();
        let state = self.view.terminals.get(&agent).cloned();
        let view = self.terminal.views.get(&agent).cloned();
        let height = window.viewport_size().height
            * if self.terminal.tall {
                PANEL_SHARE_TALL
            } else {
                PANEL_SHARE
            };
        let cwd = state
            .as_ref()
            .map(|t| t.cwd.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| session.as_ref().map(|s| s.cwd.clone()))
            .unwrap_or_default();
        let (label, tone) = match state.as_ref().map(|t| t.status) {
            None => ("No terminal yet", p.muted),
            Some(Status::Starting) => ("Starting…", p.muted),
            Some(Status::Attaching) => ("Connecting…", p.muted),
            Some(Status::Live) => ("Running", p.success),
            Some(Status::Detached) => ("Running in the background", p.muted),
            Some(Status::Exited) => ("Shell exited", p.warning),
            Some(Status::Failed) => ("Unavailable", p.error),
        };
        let error = state
            .as_ref()
            .and_then(|t| t.error.clone())
            .or_else(|| view.as_ref().and_then(|v| v.read(cx).error.clone()));
        let has_shell = state.as_ref().is_some_and(term::Terminal::running);
        let can_start = self.view.connected && session.is_some();
        let title = session
            .as_ref()
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Session".into());
        let header = div()
            .flex_shrink_0()
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                Icon::new(IconName::SquareTerminal)
                    .size(px(14.))
                    .text_color(rgb(p.accent)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Terminal"),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(px(200.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono_font())
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(cwd.clone()),
            )
            .child(
                div()
                    .debug_selector(|| "terminal-status".into())
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(rgb(tone))
                    .child(status_dot(tone))
                    .child(label),
            )
            .child(
                self.icon_button(
                    "terminal-copy",
                    "Copy screen text",
                    IconName::Copy,
                    view.is_some(),
                )
                .when_some(view.clone(), |d, view| {
                    d.on_click(move |_, _, cx| {
                        let text = view.read(cx).text();
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    })
                }),
            )
            .child(
                self.icon_button(
                    "terminal-restart",
                    "End this shell and start a new one",
                    IconName::Redo,
                    can_start && state.is_some(),
                )
                .debug_selector(|| "terminal-restart".into())
                .when(can_start && state.is_some(), |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.restart_terminal(window, cx)))
                }),
            )
            .child(
                self.icon_button(
                    "terminal-popout",
                    "Open in a separate window",
                    IconName::ExternalLink,
                    has_shell,
                )
                .debug_selector(|| "terminal-popout".into())
                .when(has_shell, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.pop_out_terminal(window, cx)))
                }),
            )
            .child(
                self.icon_button(
                    "terminal-size",
                    if self.terminal.tall {
                        "Shrink"
                    } else {
                        "Enlarge"
                    },
                    if self.terminal.tall {
                        IconName::Minimize
                    } else {
                        IconName::Maximize
                    },
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.terminal.tall = !this.terminal.tall;
                    cx.notify();
                })),
            )
            .child(
                self.icon_button(
                    "terminal-close",
                    "Hide terminal (the shell keeps running)",
                    IconName::Close,
                    true,
                )
                .debug_selector(|| "terminal-close".into())
                .on_click(cx.listener(|this, _, window, cx| this.hide_terminal(window, cx))),
            );
        let body: AnyElement = match (view, has_shell || state.as_ref().is_some_and(|t| t.status == Status::Failed)) {
            (Some(view), true) => div()
                .flex_1()
                .min_h_0()
                .px_2()
                .py_1()
                .bg(rgb(p.code_block))
                .child(view)
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(rgb(p.code_block))
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(if state.as_ref().is_some_and(|t| t.status == Status::Exited) {
                    "The shell exited. Start a new one in this agent’s folder."
                } else {
                    "This agent has no terminal yet. Start a shell in its folder on the hub’s machine."
                })
                .child(
                    self.primary_button("terminal-start", "Start terminal", can_start)
                        .debug_selector(|| "terminal-start".into())
                        .when(can_start, |d| {
                            d.on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(session) = this.selected_session().cloned() {
                                    this.start_terminal(&session.id, &session.cwd, cx);
                                    if let Some(view) = this.terminal.views.get(&session.id) {
                                        view.read(cx).focus(window);
                                    }
                                }
                            }))
                        }),
                )
                .into_any_element(),
        };
        Some(
            div()
                .id("terminal-panel")
                .debug_selector(|| "terminal-panel".into())
                .flex_shrink_0()
                .h(height)
                .w_full()
                .px_2()
                .pb_2()
                .bg(rgb(p.chat))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .rounded(px(p.panel_radius))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .overflow_hidden()
                        .child(header)
                        .when_some(error, |d, error| {
                            d.child(
                                div()
                                    .debug_selector(|| "terminal-error".into())
                                    .px_3()
                                    .py_1()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.warning))
                                    .child(error),
                            )
                        })
                        .child(body),
                )
                .into_any_element(),
        )
    }
}

impl Workspace {
    fn terminal_popped_out(&self, agent: &str) -> bool {
        self.terminal
            .popout
            .as_ref()
            .is_some_and(|(_, popped)| popped == agent)
    }

    /// Move the panel's terminal into a window of its own. The shell keeps
    /// streaming; the panel closes. The window opens after this update (its
    /// first render reads the workspace).
    pub(super) fn pop_out_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.terminal.agent.clone() else {
            return;
        };
        let Some(view) = self.terminal.views.get(&agent).cloned() else {
            return;
        };
        self.close_terminal_popout(cx);
        let focused = self.terminal_has_focus(window, cx);
        self.terminal.open = false;
        if focused {
            self.focus_editor(window, cx);
        }
        let title = self
            .view
            .sessions
            .iter()
            .find(|s| s.id == agent)
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Terminal".into());
        let workspace = cx.entity();
        let main = window.window_handle();
        cx.defer(move |cx| {
            let opened =
                open_terminal_window(workspace.downgrade(), agent.clone(), view, title, cx);
            let _ = main.update(cx, |_, _, cx| {
                workspace.update(cx, |ws, cx| {
                    match opened {
                        Ok(handle) => ws.terminal.popout = Some((handle, agent)),
                        Err(error) => {
                            ws.extras.notice = format!("Couldn’t open a terminal window: {error}");
                            ws.terminal.open = true;
                        }
                    }
                    cx.notify();
                })
            });
        });
        cx.notify();
    }

    /// The terminal window closed: stop streaming unless the panel shows it.
    fn terminal_window_closed(&mut self, id: gpui::WindowId, cx: &mut Context<Self>) {
        let Some((handle, agent)) = self.terminal.popout.clone() else {
            return;
        };
        if handle.window_id() != id {
            return;
        }
        self.terminal.popout = None;
        if !(self.terminal.open && self.terminal.agent.as_ref() == Some(&agent)) {
            self.command(Command::Terminal(term::Command::Hide { agent }), cx);
        }
        cx.notify();
    }

    /// Bring a popped-out terminal back under the conversation.
    fn dock_terminal(&mut self, id: gpui::WindowId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((handle, agent)) = self.terminal.popout.clone() else {
            return;
        };
        if handle.window_id() != id {
            return;
        }
        self.terminal.popout = None;
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        if self.view.selected.as_ref() != Some(&agent) {
            self.command(Command::Select(agent.clone()), cx);
        }
        self.terminal.open = true;
        self.terminal.agent = Some(agent.clone());
        self.screen = Screen::Conversation;
        if let Some(view) = self.terminal.views.get(&agent) {
            view.read(cx).focus(window);
        }
        cx.notify();
    }

    /// Close a popped-out terminal with its owner (main window closing).
    pub(super) fn close_terminal_popout(&mut self, cx: &mut App) {
        if let Some((handle, _)) = self.terminal.popout.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// The root of a popped-out terminal's window.
struct TerminalWindow {
    workspace: gpui::WeakEntity<Workspace>,
    agent: String,
    title: String,
    view: Entity<TerminalView>,
    _keys: gpui::Subscription,
}

impl Render for TerminalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.global::<TerminalPalette>().0;
        let (status, cwd) = self
            .workspace
            .upgrade()
            .and_then(|ws| ws.read(cx).view.terminals.get(&self.agent).cloned())
            .map(|t| {
                (
                    match t.status {
                        Status::Live => "Running",
                        Status::Exited => "Shell exited — dock it to start a new one",
                        Status::Failed => "Unavailable",
                        Status::Starting | Status::Attaching => "Connecting…",
                        Status::Detached => "Detached",
                    },
                    t.cwd,
                )
            })
            .unwrap_or(("Connecting…", String::new()));
        let id = window.window_handle().window_id();
        div()
            .id("terminal-window")
            .debug_selector(|| "terminal-window".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .font_family(gpui_component::Theme::global(cx).font_family.clone())
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        Icon::new(IconName::SquareTerminal)
                            .size(px(14.))
                            .text_color(rgb(p.accent)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(cwd),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.muted))
                            .child(status),
                    )
                    .child(
                        div()
                            .id("terminal-window-dock")
                            .debug_selector(|| "terminal-window-dock".into())
                            .px_2()
                            .py_1()
                            .rounded(px(p.control_radius))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(p.selected)))
                            .child("Dock under the chat")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let workspace = this.workspace.clone();
                                cx.defer(move |cx| {
                                    let Some(workspace) = workspace.upgrade() else {
                                        return;
                                    };
                                    let Some(main) = workspace.read(cx).main_window else {
                                        return;
                                    };
                                    let _ = main.update(cx, |_, window, cx| {
                                        workspace
                                            .update(cx, |ws, cx| ws.dock_terminal(id, window, cx))
                                    });
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .py_1()
                    .bg(rgb(p.code_block))
                    .child(self.view.clone()),
            )
    }
}

/// Open the terminal's own window. Keys typed there go to its shell ahead of
/// every binding, as in the docked panel.
fn open_terminal_window(
    workspace: gpui::WeakEntity<Workspace>,
    agent: String,
    view: Entity<TerminalView>,
    title: String,
    cx: &mut App,
) -> anyhow::Result<gpui::WindowHandle<gpui_component::Root>> {
    let bounds = gpui::Bounds::centered(None, gpui::size(px(900.), px(560.)), cx);
    let window_title = format!("{title} — Terminal — Workspacer");
    let handle = cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            window_min_size: Some(gpui::size(px(360.), px(200.))),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(window_title.clone().into()),
                ..Default::default()
            }),
            app_id: Some("workspacer-native".into()),
            ..Default::default()
        },
        |window, cx| {
            window.set_window_title(&window_title);
            window.set_app_id("workspacer-native");
            let own = window.window_handle();
            let keys_view = view.clone();
            let keys = cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle() != own || !keys_view.read(cx).has_focus(window) {
                    return;
                }
                if keys_view.update(cx, |view, cx| view.keystroke(&event.keystroke, cx)) {
                    cx.stop_propagation();
                }
            });
            let closing = workspace.clone();
            window.on_window_should_close(cx, move |window, cx| {
                let id = window.window_handle().window_id();
                let workspace = closing.clone();
                cx.defer(move |cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |ws, cx| ws.terminal_window_closed(id, cx));
                    }
                });
                true
            });
            view.read(cx).focus(window);
            let root = cx.new(|_| TerminalWindow {
                workspace,
                agent,
                title,
                view,
                _keys: keys,
            });
            cx.new(|cx| gpui_component::Root::new(root, window, cx))
        },
    )?;
    Ok(handle)
}

#[cfg(all(test, feature = "ui-tests"))]
impl Workspace {
    pub(super) fn terminal_view(&self, agent: &str) -> Option<Entity<TerminalView>> {
        self.terminal.views.get(agent).cloned()
    }
    pub(super) fn terminal_popout(&self) -> Option<gpui::WindowHandle<gpui_component::Root>> {
        self.terminal.popout.as_ref().map(|(h, _)| *h)
    }
}
