//! Tall windows (a portrait monitor, or any window much taller than wide).
//!
//! Width is what such a window lacks and height is what it has, so the tall
//! layout spends height instead: the file viewer and background tasks dock
//! below the conversation rather than beside it (where they would only fit
//! as modal sheets), and the sidebar starts as its rail. The rail's flyout
//! (hover it, or Ctrl+K to search) shows the full session list over the
//! conversation without taking its width.
use super::*;
use gpui::{AnyElement, Pixels, deferred};
use std::time::Duration;

/// How long the pointer rests on the rail before its flyout opens, so
/// passing over it or clicking a session does not.
const PEEK_OPEN: Duration = Duration::from_millis(450);
/// Grace after the pointer leaves the rail and flyout, so the gap between
/// them is crossed without the flyout closing.
const PEEK_CLOSE: Duration = Duration::from_millis(250);
/// The conversation keeps at least this much height above a docked panel.
const MIN_CHAT_ABOVE: f32 = 280.;
/// A panel docked below is never shorter than this; when the window cannot
/// fit both, the panel is a sheet as in narrow windows.
const MIN_DOCK_BELOW: f32 = 220.;
/// The rail, and the gap between it and the flyout.
pub(super) const RAIL_WIDTH: f32 = 56.;
const FLYOUT_GAP: f32 = 6.;

/// How the rail's flyout was opened, which decides how it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Peek {
    /// The pointer rested on the rail: leaving both closes it.
    Hover,
    /// Ctrl+K, the rail's search or notice, or a click inside a hover
    /// flyout: stays until Esc, a click outside, or a choice is made.
    Pinned,
}

/// Where a side panel (file viewer, background tasks) docks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Dock {
    /// Beside the conversation, this wide (unzoomed).
    Side(f32),
    /// Below the conversation, this wide and tall (unzoomed).
    Below { width: f32, height: f32 },
}

#[derive(Default)]
pub(super) struct LayoutUi {
    /// The window's shape last frame; `None` before the first.
    portrait: Option<bool>,
    /// The wide layout's sidebar state, restored when the window is wide again.
    landscape_collapsed: bool,
    pub(super) peek: Option<Peek>,
    /// What the flyout opened over; a change (a session chosen, a page
    /// opened) closes it.
    peek_over: Option<(Option<String>, Screen, bool)>,
    /// The session search when the flyout opened, put back when it closes:
    /// the search filters the rail too, and a switcher query should not
    /// outlive the flyout.
    peek_query: String,
    rail_hovered: bool,
    flyout_hovered: bool,
    hover_timer: Option<Task<()>>,
    /// A drag of the docked panel's top edge: pointer y and share at start.
    dock_drag: Option<(Pixels, f32)>,
}

impl Workspace {
    /// The tall layout applies to this window now.
    pub(super) fn portrait(&self, window: &Window) -> bool {
        let size = window.viewport_size();
        self.settings
            .window_layout
            .portrait(f32::from(size.width), f32::from(size.height))
    }

    /// Follow the window between shapes: becoming tall collapses the sidebar
    /// to its rail, and becoming wide again restores what the wide layout had.
    pub(super) fn track_orientation(&mut self, window: &Window) {
        let portrait = self.portrait(window);
        if self.layout.portrait == Some(portrait) {
            return;
        }
        if portrait {
            self.layout.landscape_collapsed = self.sidebar_collapsed;
            self.sidebar_collapsed = true;
        } else if self.layout.portrait.is_some() {
            self.sidebar_collapsed = self.layout.landscape_collapsed;
        }
        self.layout.portrait = Some(portrait);
        self.layout.peek = None;
        self.layout.peek_over = None;
    }

    /// The sidebar's footprint in the window row, gutter included (unzoomed).
    pub(super) fn sidebar_footprint(&self, viewport_width: f32) -> f32 {
        let sidebar = if self.sidebar_collapsed {
            RAIL_WIDTH
        } else {
            wks_native::navigation::sidebar_width(self.settings.sidebar_width, viewport_width)
        };
        sidebar + 8.
    }

    /// Where a side panel docks, or `None` when the window cannot keep the
    /// conversation usable beside or above it (the panel is then a sheet).
    pub(super) fn dock(&self, window: &Window) -> Option<Dock> {
        let width = unzoom(window.viewport_size().width);
        let available = width - self.sidebar_footprint(width);
        if self.portrait(window) {
            let height = unzoom(window.viewport_size().height);
            let room = height - MIN_CHAT_ABOVE;
            if room < MIN_DOCK_BELOW {
                return None;
            }
            let share =
                wks_native::navigation::portrait_dock_share(self.settings.portrait_dock_share);
            return Some(Dock::Below {
                width: available,
                height: (height * share).clamp(MIN_DOCK_BELOW, room),
            });
        }
        let panel = (available * 0.5).clamp(360., 760.);
        (available - panel >= file_viewer::MIN_CHAT_BESIDE).then_some(Dock::Side(panel))
    }

    /// The height a docked-below panel takes from the conversation (unzoomed).
    pub(super) fn dock_below_height(&self, window: &Window) -> f32 {
        let showing = self.viewer_in_slot() || self.tasks_showing();
        match self.dock(window) {
            Some(Dock::Below { height, .. }) if showing => height,
            _ => 0.,
        }
    }

    /// The slot a panel docked below the conversation sits in: it grows up
    /// from the window's bottom edge, and its top edge drags to resize.
    /// `panel` is pinned to the slot's bottom at its full height, so it
    /// slides up into view as the slot grows.
    pub(super) fn dock_below_slot(
        &self,
        id: &'static str,
        height: f32,
        panel: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .relative()
            .w_full()
            .h(px(height))
            .flex_shrink_0()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .w_full()
                    .h(px(height))
                    .pt_1()
                    .pb_2()
                    .pr_2()
                    .child(panel),
            )
            .child(
                div()
                    .id("dock-below-resize")
                    .debug_selector(|| "dock-below-resize".into())
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .cursor(gpui::CursorStyle::ResizeUpDown)
                    .hover(move |s| s.bg(rgb(p.border)))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if event.click_count == 2 {
                                this.settings.portrait_dock_share =
                                    wks_native::navigation::PORTRAIT_DOCK_SHARE;
                                this.layout.dock_drag = None;
                                this.save_settings(cx);
                            } else {
                                this.layout.dock_drag = Some((
                                    event.position.y,
                                    wks_native::navigation::portrait_dock_share(
                                        this.settings.portrait_dock_share,
                                    ),
                                ));
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(self.dock_drag_watch(cx))
    }

    /// While the docked panel's edge is dragged, follow the pointer anywhere
    /// in the window and save the share on release.
    fn dock_drag_watch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let dragging = self.layout.dock_drag.is_some();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let moving = entity.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase.capture() {
                        let _ = moving.update(cx, |this, cx| {
                            if let Some((start, share)) = this.layout.dock_drag {
                                let height = f32::from(window.viewport_size().height).max(1.);
                                // Dragging up grows the panel.
                                let moved = f32::from(start - event.position.y) / height;
                                this.settings.portrait_dock_share =
                                    wks_native::navigation::portrait_dock_share(share + moved);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        });
                    }
                });
                let releasing = entity.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase.capture() {
                        let _ = releasing.update(cx, |this, cx| {
                            if this.layout.dock_drag.take().is_some() {
                                this.save_settings(cx);
                                cx.stop_propagation();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size(px(0.))
    }

    /// The pointer entered or left the rail.
    pub(super) fn rail_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.layout.rail_hovered = hovered;
        self.arm_peek(cx);
    }

    /// The pointer entered or left the flyout.
    fn flyout_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.layout.flyout_hovered = hovered;
        self.arm_peek(cx);
    }

    /// Open a hover flyout once the pointer rests on the rail, and close it
    /// once the pointer has left both rail and flyout for a moment.
    fn arm_peek(&mut self, cx: &mut Context<Self>) {
        let over = self.layout.rail_hovered || self.layout.flyout_hovered;
        let wait = match (self.layout.peek, over) {
            (None, true) if self.layout.rail_hovered => PEEK_OPEN,
            (Some(Peek::Hover), false) => PEEK_CLOSE,
            _ => {
                self.layout.hover_timer = None;
                return;
            }
        };
        self.layout.hover_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.layout.hover_timer = None;
                let over = this.layout.rail_hovered || this.layout.flyout_hovered;
                match this.layout.peek {
                    None if this.layout.rail_hovered && this.sidebar_collapsed => {
                        this.open_peek(Peek::Hover, cx)
                    }
                    Some(Peek::Hover) if !over => {
                        this.layout.peek = None;
                        cx.notify();
                    }
                    _ => {}
                }
            });
        }));
    }

    pub(super) fn open_peek(&mut self, peek: Peek, cx: &mut Context<Self>) {
        if self.layout.peek.is_none() {
            self.layout.peek_query = self.search.read(cx).value().to_string();
        }
        self.layout.peek = Some(peek);
        self.layout.peek_over = Some(self.peek_context());
        cx.notify();
    }

    /// The rail's search: the flyout, pinned, with its search focused.
    pub(super) fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_collapsed {
            self.open_peek(Peek::Pinned, cx);
        }
        self.search.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Close the flyout, putting back the search it opened with.
    pub(super) fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layout.peek.take().is_none() {
            return false;
        }
        self.layout.peek_over = None;
        self.layout.hover_timer = None;
        self.layout.flyout_hovered = false;
        let query = std::mem::take(&mut self.layout.peek_query);
        if self.search.read(cx).value() != query.as_str() {
            self.search
                .update(cx, |input, cx| input.set_value(query, window, cx));
        }
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            window.focus(&self.focus);
        }
        cx.notify();
        true
    }

    fn peek_context(&self) -> (Option<String>, Screen, bool) {
        (
            self.navigation_selected
                .clone()
                .or_else(|| self.view.selected.clone()),
            self.screen,
            self.new_session,
        )
    }

    /// A session or page chosen from the flyout closes it, except while its
    /// search is focused: there Alt+↑/↓ step through matches live and Enter
    /// closes it.
    pub(super) fn settle_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.peek.is_none() {
            return;
        }
        if !self.sidebar_collapsed {
            self.layout.peek = None;
            self.layout.peek_over = None;
            return;
        }
        let now = self.peek_context();
        if self.layout.peek_over.as_ref() == Some(&now) {
            return;
        }
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            self.layout.peek_over = Some(now);
            return;
        }
        self.close_peek(window, cx);
    }

    /// Enter in the session search opens the highlighted match: the session
    /// already open if it matches, else the first match.
    pub(super) fn open_search_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Projects && !self.new_session {
            return;
        }
        let rows = self.visible_sessions(cx);
        let open = self
            .navigation_selected
            .as_ref()
            .or(self.view.selected.as_ref());
        let chosen = rows
            .iter()
            .find(|&&ix| Some(&self.view.sessions[ix].id) == open)
            .or(rows.first())
            .map(|&ix| self.view.sessions[ix].id.clone());
        let Some(id) = chosen else {
            return;
        };
        self.new_session = false;
        self.screen = Screen::Conversation;
        if Some(&id) != open {
            self.command(Command::Select(id), cx);
        }
        self.close_peek(window, cx);
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// The full sidebar beside the rail, over the conversation. A pinned
    /// flyout dims the conversation, and a click there closes it.
    pub(super) fn render_rail_flyout(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let peek = self.layout.peek.filter(|_| self.sidebar_collapsed)?;
        let p = self.appearance.palette();
        let left = px(8. + RAIL_WIDTH + FLYOUT_GAP);
        let panel = self.render_sidebar_panel(window, cx);
        let flyout = div()
            .id("sidebar-flyout")
            .debug_selector(|| "sidebar-flyout".into())
            .occlude()
            .absolute()
            .top_0()
            .bottom_0()
            .left(left)
            .py_2()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.flyout_hover(*hovered, cx)))
            // Working inside a hover flyout (typing a search, say) keeps it.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.layout.peek == Some(Peek::Hover) {
                        this.layout.peek = Some(Peek::Pinned);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .h_full()
                    .rounded(px(p.panel_radius))
                    .shadow(chrome::floating_shadow(p))
                    .child(panel),
            );
        let layer = div()
            .absolute()
            .inset_0()
            .when(peek == Peek::Pinned, |d| {
                d.child(
                    div()
                        .id("sidebar-flyout-backdrop")
                        .debug_selector(|| "sidebar-flyout-backdrop".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .left(px(8. + RAIL_WIDTH))
                        .occlude()
                        .bg(gpui::Hsla::from(rgb(p.shadow)).opacity(0.25))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.close_peek(window, cx);
                            }),
                        ),
                )
            })
            .child(flyout);
        Some(deferred(layer).with_priority(1).into_any_element())
    }
}
