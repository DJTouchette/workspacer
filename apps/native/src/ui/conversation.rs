//! The conversation page: the transcript under a floating title header, and
//! the dock below it with the pending cards (approval, questions, children)
//! above the composer.
use super::*;

/// How far each sloped side of the status tab runs in from its base.
const NOTCH_SLANT: f32 = 14.;

/// The status tab's height above the composer; shorter where space is tight.
fn notch_height(compact: bool) -> f32 {
    if compact { 18. } else { 26. }
}

type Point = gpui::Point<gpui::Pixels>;

/// The tab's sides and top as segments from its left foot to its right foot:
/// each a point, with a control point where it curves.
fn notch_outline(
    bounds: gpui::Bounds<gpui::Pixels>,
    height: f32,
    z: f32,
) -> Vec<(Point, Option<Point>)> {
    // Everything here is in laid-out (already zoomed) pixels: `bounds` comes
    // from layout, so only the shape's own constants take the interface zoom.
    // (Zooming `bounds` again drew the right side short of the label.)
    let at = |x: f32, y: f32| bounds.origin + gpui::point(gpui::px(x), gpui::px(y));
    let w = f32::from(bounds.size.width);
    // The middle of the composer's 1px border row, where the feet join it.
    let base = height * z + 0.5;
    let s = NOTCH_SLANT * z;
    let (foot, rise, shoulder) = (5. * z, 3. * z, 3. * z);
    // Flared foot, sloped side, rounded shoulder; mirrored on the right.
    vec![
        (at(0., base), None),
        (at(foot, base - rise), Some(at(foot * 0.7, base))),
        (at(s - shoulder, 4. * z), None),
        (at(s + shoulder, 0.5), Some(at(s - z, 0.5))),
        (at(w - s - shoulder, 0.5), None),
        (at(w - s + shoulder, 4. * z), Some(at(w - s + z, 0.5))),
        (at(w - foot, base - rise), None),
        (at(w, base), Some(at(w - foot * 0.7, base))),
    ]
}

/// The tab's outline: a filled body whose base covers the composer's top
/// border, and the stroked sides and top (no base line, so the tab opens
/// into the composer).
fn notch_paths(
    bounds: gpui::Bounds<gpui::Pixels>,
    height: f32,
) -> (
    anyhow::Result<gpui::Path<gpui::Pixels>>,
    anyhow::Result<gpui::Path<gpui::Pixels>>,
) {
    let outline = notch_outline(bounds, height, zoom());
    let trace = |b: &mut gpui::PathBuilder| {
        for (ix, (to, ctrl)) in outline.iter().enumerate() {
            match (ix, ctrl) {
                (0, _) => b.move_to(*to),
                (_, Some(ctrl)) => b.curve_to(*to, *ctrl),
                (_, None) => b.line_to(*to),
            }
        }
    };
    let mut fill = gpui::PathBuilder::fill();
    trace(&mut fill);
    // A pixel past the border row: ending flush with it left an antialiased
    // sliver of the border showing under the label at fractional scales.
    // The overlap lands on the composer's own surface colour.
    let floor = bounds.origin.y + gpui::px(height * zoom() + 2.);
    fill.line_to(gpui::point(bounds.right(), floor));
    fill.line_to(gpui::point(bounds.left(), floor));
    fill.close();
    let mut stroke = gpui::PathBuilder::stroke(gpui::px(1.));
    trace(&mut stroke);
    (fill.build(), stroke.build())
}

#[cfg(test)]
mod notch_tests {
    use super::*;

    /// At any interface zoom the outline spans the tab's laid-out bounds
    /// exactly, so the label laid out inside them stays inside the shape.
    #[test]
    fn the_outline_spans_its_bounds_at_every_zoom() {
        for z in [0.9, 1., 1.25] {
            let bounds = gpui::Bounds::new(
                gpui::point(gpui::px(300.), gpui::px(40.)),
                gpui::size(gpui::px(170. * z), gpui::px((22. + 1.) * z)),
            );
            let outline = notch_outline(bounds, 22., z);
            let xs = outline.iter().map(|(p, _)| f32::from(p.x));
            let (min, max) = xs.fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
            assert_eq!((min, max), (300., 300. + 170. * z), "zoom {z}");
            // The flat top covers all but the two slopes.
            let top = &outline[3..5];
            assert!(
                f32::from(top[1].0.x - top[0].0.x) >= 170. * z - 2. * (NOTCH_SLANT + 3.) * z - 0.01
            );
        }
    }
}

struct ComposerActivity {
    label: String,
    /// Something is running or pending: shown in the composer's notch.
    live: bool,
    animate: bool,
}

/// What the conversation decided once per frame, for the pieces it builds.
#[derive(Clone, Copy)]
struct ChatLayout {
    narrow: bool,
    compact: bool,
    /// The selected session can take a message or decision now.
    enabled: bool,
    working: bool,
}

/// The tool rail's width and its gap from the chat's right edge.
const RAIL_ROOM: f32 = 56.;

impl Workspace {
    pub(super) fn render_conversation(
        &mut self,
        narrow: bool,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let selected = self.selected_session().cloned();
        let enabled = self.view.connected
            && !self.view.loading
            && self
                .navigation_selected
                .as_ref()
                .is_none_or(|id| Some(id) == self.view.selected.as_ref())
            && !self.view.busy
            && selected
                .as_ref()
                .is_some_and(|s| !s.stopped() || self.paused(s) && self.resuming.is_none());
        let working = self.view.connected && selected.as_ref().is_some_and(Session::working);
        let layout = ChatLayout {
            narrow,
            compact,
            enabled,
            working,
        };
        let entity = cx.entity().downgrade();
        let transcript = list(self.list.clone(), move |ix, window, cx| {
            entity
                .update(cx, |this, cx| this.render_chat_row(ix, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0()
        .pt(self.header_bounds.size.height + px(16.))
        // This is scrollable tail space, not a smaller viewport: history still
        // paints behind the floating dock, while the last message can clear it.
        .pb(if selected.is_some() {
            self.composer_dock_bounds.size.height + px(12.)
        } else {
            px(0.)
        });
        let terminal_panel = self.render_terminal_panel(window, cx);
        let header = self.render_chat_header(layout, selected.as_ref(), window, cx);
        let empty = self.view.transcript.rows.is_empty();
        let only_children =
            empty && self.view.child.is_none() && !self.child_ui.agents.unanchored.is_empty();
        let chat = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .bg(rgb(p.chat))
            .child(chrome::measure(
                cx.entity().downgrade(),
                |this: &mut Self, bounds, _| {
                    let changed = this.chat_width != bounds.size.width;
                    this.chat_width = bounds.size.width;
                    changed
                },
            ))
            .when(empty && !only_children, |d| {
                d.child(self.render_empty_state(compact, window, cx))
            })
            .when(only_children, |d| {
                d.child(self.render_child_only(window, cx))
            })
            .when(!empty, |d| {
                d.child(transcript).child(self.wheel_smoother(cx))
            })
            .child(header)
            .when(
                self.tools_float(narrow) && selected.is_some() && self.viewed_child().is_none(),
                |d| d.child(self.render_tool_rail(enabled, cx)),
            )
            .when(!self.follow, |d| d.child(self.render_jump_latest(cx)))
            .children(self.render_link_status())
            .child(self.render_dock(layout, selected.as_ref(), window, cx));
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.chat))
            .child(chat)
            .children(terminal_panel)
    }

    /// Whether the session's tools float in the rail rather than the title.
    /// Only where the rail clears the centred chat column, so it never
    /// covers a card's own controls at the column's edge.
    fn tools_float(&self, narrow: bool) -> bool {
        self.settings.tool_rail && !narrow && self.chat_width >= px(CHAT_WIDTH + 2. * RAIL_ROOM)
    }

    /// The session's tools in a floating pill at the chat's right edge,
    /// centred between the title and the composer.
    fn render_tool_rail(&self, enabled: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .absolute()
            .top(self.header_bounds.size.height)
            .bottom(self.composer_dock_bounds.size.height)
            .right_3()
            .flex()
            .items_center()
            .child(
                div()
                    .debug_selector(|| "tool-rail".into())
                    .occlude()
                    .p_1()
                    .rounded_full()
                    .bg(rgb(p.surface))
                    .border_1()
                    .border_color(rgb(p.border))
                    .shadow(chrome::floating_shadow(p))
                    .child(self.chat_actions(enabled, true, cx)),
            )
    }

    /// The title capsule over a transparent fade rather than a ruled strip:
    /// history scrolls softly under the floating pill instead of colliding
    /// with a hard edge.
    fn render_chat_header(
        &mut self,
        layout: ChatLayout,
        selected: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let title = selected
            .map(|s| self.session_title(s))
            .unwrap_or_else(|| "Your sessions".into());
        let notice = self.title_notice();
        let (title_bar, title_actions) = match self.render_child_title_bar(cx) {
            Some(bar) => (bar, None),
            None => (
                self.render_title_bar(layout.narrow, layout.enabled, &title, selected, cx),
                // Wide windows float the tools at the chat's right edge; the
                // capsule still reveals the context figures on hover.
                Some(if !self.tools_float(layout.narrow) {
                    self.chat_actions(layout.enabled, false, cx)
                } else {
                    div()
                }),
            ),
        };
        let title_island = self.render_title_island(title_bar, title_actions, notice, window, cx);
        let plan = self.render_pinned_plan(layout.compact, window, cx);
        // A header that grows or shrinks while reading history keeps the
        // same message under the reader's eye.
        let header_size = chrome::measure(cx.entity().downgrade(), |this: &mut Self, bounds, _| {
            if this.header_bounds == bounds {
                return false;
            }
            let delta = bounds.size.height - this.header_bounds.size.height;
            if !this.follow && delta != px(0.) {
                let mut anchor = this.scroll_anchor();
                anchor.offset_in_item += delta;
                this.list.scroll_to(anchor);
            }
            this.header_bounds = bounds;
            true
        });
        // The caption buttons take the right edge, so the title row's room
        // stops short of them and its centre sits left of the transcript's.
        // Nudge the capsule back by that half-gutter, as far as the room on
        // its right allows. A paint offset, not layout: the capsule's measured
        // width (which decides where the context gauge goes) never moves.
        let nudge = if chrome::custom_caption() {
            // The widest the capsule gets: drawn last frame, or at rest with
            // its actions revealed, but never past its column (where the title
            // truncates instead). The same at rest and open, so revealing the
            // actions grows it about a fixed centre.
            let widest = self
                .title_island_width
                .max(self.title_base_width + self.title_reveal.width + px(2.))
                .min(self.header_bounds.size.width - px(40.));
            // How far right it can go before its widest outline reaches the
            // caption buttons.
            let right = self.header_bounds.center().x + widest / 2.;
            let room = window.viewport_size().width - px(chrome::CAPTION_WIDTH) - right;
            room.clamp(px(0.), px(chrome::CAPTION_WIDTH / 2.))
        } else {
            px(0.)
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .bg(gpui::linear_gradient(
                180.,
                gpui::linear_color_stop(rgb(p.chat), 0.6),
                gpui::linear_color_stop(gpui::Hsla::from(rgb(p.chat)).opacity(0.), 1.),
            ))
            // Only the top chrome row drags, with later occluding title pills
            // and caption controls excluded by GPUI's hit test. Occluding the
            // drag surface itself keeps shell focus from cancelling OS moves.
            .when(chrome::custom_caption(), |d| {
                d.child(
                    chrome::drag_region(div())
                        .debug_selector(|| "chat-drag-region".into())
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(56.)),
                )
                .pr(px(chrome::CAPTION_WIDTH))
            })
            .child(
                chrome::chat_column()
                    .relative()
                    .pt_3()
                    .pb_5()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(header_size)
                    .child(
                        div()
                            .relative()
                            .left(nudge)
                            // Passes the column's width limit on to the island.
                            .max_w_full()
                            .min_w_0()
                            .flex()
                            .child(chrome::measure(
                                cx.entity().downgrade(),
                                |this: &mut Self, bounds, _| {
                                    let changed = this.title_island_width != bounds.size.width;
                                    this.title_island_width = bounds.size.width;
                                    changed
                                },
                            ))
                            .child(title_island),
                    )
                    // Centred under the capsule, so it takes the same nudge.
                    .children(plan.map(|plan| plan.relative().left(nudge))),
            )
    }

    /// The notice the title capsule carries: this window's own first, and a
    /// local backend that is still starting says so in the empty state
    /// instead.
    pub(super) fn title_notice(&self) -> String {
        if !self.local_notice.is_empty() {
            self.local_notice.clone()
        } else if !self.view.connected && self.view.notice.starts_with("Starting Rust backend") {
            String::new()
        } else if !self.view.connected {
            self.view
                .notice
                .trim_end_matches(". Reconnecting…")
                .to_owned()
        } else {
            self.view.notice.clone()
        }
    }

    fn render_jump_latest(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        div()
            .absolute()
            .left_0()
            .w_full()
            .bottom(self.composer_dock_bounds.size.height + px(6.))
            .flex()
            .justify_center()
            .child(
                self.button("latest", "Jump to latest", true)
                    .shadow(chrome::floating_shadow(p))
                    .debug_selector(|| "jump-latest".into())
                    .mx_auto()
                    .rounded_full()
                    .bg(rgb(p.surface))
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.follow = true;
                        this.list.scroll_to(ListOffset {
                            item_ix: this.view.transcript.rows.len(),
                            offset_in_item: px(0.),
                        });
                        cx.notify();
                    })),
            )
    }

    /// The dock and its backdrop: opaque behind the cards and hint line so
    /// scrolled-back history never shows through their gaps, fading out over
    /// the 12px above them (the transcript's spare padding).
    fn render_dock(
        &mut self,
        layout: ChatLayout,
        selected: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let compact = layout.compact;
        let composer_docked = selected.is_some() && self.view.child.is_none();
        // Gutters and the gap above the composer stay inside this clip, so
        // card shadows keep their room.
        let cards = chrome::chat_column()
            .id("conversation-dock-cards")
            .debug_selector(|| "conversation-dock-cards".into())
            .flex_shrink_0()
            .max_h(self.dock_cards_budget(selected, compact, window))
            .overflow_y_scroll()
            .pt(px(if compact { 8. } else { 12. }))
            .pb(px(match (composer_docked, compact) {
                (false, _) => 0.,
                (true, true) => 4.,
                (true, false) => 8.,
            }))
            .flex()
            .flex_col()
            .gap(px(if compact { 4. } else { 8. }))
            .when(self.view.child.is_none(), |d| {
                d.child(self.render_pending(window, cx))
            })
            .when_some(selected.and_then(|s| s.approval.as_ref()), |d, approval| {
                d.child(self.render_approval_card(approval, layout, window, cx))
            })
            .children(selected.and_then(|session| self.render_limit_card(session, compact, cx)))
            .when_some(selected.filter(|s| s.questions.is_some()), |d, session| {
                d.child(self.render_questions(session, layout.enabled, compact, window, cx))
            })
            .children(self.render_child_bar(cx))
            // Closest to the composer: what will be sent from it next.
            .children(selected.and_then(|session| {
                self.render_queue_panel(session, layout.enabled, compact, window, cx)
            }));
        let composer_layer = chrome::chat_column()
            .relative()
            .flex_shrink_0()
            // Room for the status tab on the composer's top edge, while it shows.
            .when(
                composer_docked && self.composer_activity(layout).live,
                |d| d.pt(px(notch_height(compact) - if compact { 4. } else { 8. })),
            )
            .pb(px(if compact { 8. } else { 16. }))
            .child(chrome::measure(
                cx.entity().downgrade(),
                |this: &mut Self, bounds, _| {
                    let changed = this.composer_layer_height != bounds.size.height;
                    this.composer_layer_height = bounds.size.height;
                    changed
                },
            ))
            .when(composer_docked, |d| {
                d.child(self.render_composer(layout, window, cx))
            });
        let dock_height = f32::from(self.composer_dock_bounds.size.height);
        let opaque_until = 1. - DOCK_FADE / (dock_height + DOCK_FADE).max(DOCK_FADE * 2.);
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(DOCK_FADE))
            .bg(gpui::linear_gradient(
                0.,
                gpui::linear_color_stop(rgb(p.chat), opaque_until),
                gpui::linear_color_stop(gpui::Hsla::from(rgb(p.chat)).opacity(0.), 1.),
            ))
            // Two layers under one measured frame (bug #26): the cards scroll
            // in their own region, capped at what the measured composer
            // leaves of the dock's share; the composer never scrolls or
            // clips. (Measuring inside a scroll container would count as
            // content: a phantom pt+pb scroll range.)
            .child(
                chrome::chat_frame()
                    .id("conversation-dock")
                    .debug_selector(|| "conversation-dock".into())
                    .relative()
                    .flex()
                    .flex_col()
                    .child(chrome::measure(
                        cx.entity().downgrade(),
                        |this: &mut Self, bounds, _| {
                            let changed = this.composer_dock_bounds != bounds;
                            this.composer_dock_bounds = bounds;
                            changed
                        },
                    ))
                    .child(cards)
                    .child(composer_layer),
            )
    }

    /// The height the dock's cards may take: their share of the window less
    /// the measured composer, never under the floor beside a big draft.
    fn dock_cards_budget(
        &self,
        selected: Option<&Session>,
        compact: bool,
        window: &Window,
    ) -> gpui::Pixels {
        let height = window.viewport_size().height;
        // A pending question set takes more of the dock: it is what the agent
        // is waiting on, and its own list scrolls inside the card.
        let questions_pending =
            self.view.child.is_none() && selected.is_some_and(|s| s.questions.is_some());
        let dock_share = height
            * match (questions_pending, compact) {
                (true, _) => 0.7,
                (false, true) => 0.45,
                (false, false) => 0.55,
            };
        // Reserve a title row, one notice row and some readable transcript.
        // Unlike the scaled UI units, the viewport remains physical pixels.
        let readable = if unzoom(height) < 300. {
            gpui::px(24.)
        } else {
            px(24.)
        };
        (dock_share - self.composer_layer_height)
            .max(px(DOCK_CARDS_FLOOR))
            .min((height - px(104.) - gpui::px(24.) - self.composer_layer_height).max(readable))
    }

    fn render_approval_card(
        &self,
        approval: &serde_json::Value,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use serde_json::Value;
        let p = self.appearance.palette();
        let (compact, enabled) = (layout.compact, layout.enabled);
        if let Some(plan) = approval
            .get("toolName")
            .and_then(Value::as_str)
            .filter(|tool| *tool == "ExitPlanMode")
            .and(approval.pointer("/toolInput/plan"))
            .and_then(Value::as_str)
            .filter(|plan| !plan.trim().is_empty())
        {
            return self.render_plan_card(plan, layout, window, cx);
        }
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let label = approval
            .get("toolName")
            .or_else(|| approval.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("Tool");
        let summary = approval
            .pointer("/toolInput/command")
            .or_else(|| approval.pointer("/toolInput/file_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_owned();
        let details = serde_json::to_string_pretty(
            approval
                .get("toolInput")
                .or_else(|| approval.get("raw"))
                .unwrap_or(approval),
        )
        .unwrap_or_default();
        let open = self.extras.approval_details;
        // The card the user is deciding on, sent with the decision so it can
        // never land on a newer one.
        let card = self.selected_session().and_then(Session::card);
        let always = wks_native::session_controls::always_options(approval);
        div()
            .debug_selector(|| "approval-card".into())
            .occlude()
            .w_full()
            .p(px(if compact { 8. } else { 12. }))
            .rounded(px(p.panel_radius))
            .shadow(chrome::floating_shadow(p))
            .bg(rgb(p.surface))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(p.warning))
                    .child(status_dot(p.warning))
                    .child(format!("Permission needed · {label}"))
                    .when(compact, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(p.text))
                                .font_family(mono.clone())
                                .child(summary.clone()),
                        )
                        .child(self.approval_toggle(
                            "approval-toggle-compact",
                            if open { "Hide" } else { "Details" },
                            cx,
                        ))
                    }),
            )
            .when(!compact, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(mono.clone())
                                .child(if summary.is_empty() {
                                    "Review request details".to_owned()
                                } else {
                                    summary
                                }),
                        )
                        .child(self.approval_toggle(
                            "approval-toggle",
                            if open { "Hide details" } else { "Details" },
                            cx,
                        )),
                )
            })
            .when(open, |d| {
                d.child(
                    div()
                        .id("approval-details")
                        .max_h(px(if compact { 52. } else { 120. }))
                        .overflow_y_scroll()
                        .p_3()
                        .rounded(px(p.control_radius))
                        .bg(rgb(p.code_block))
                        .border_1()
                        .border_color(rgb(p.border))
                        .font_family(mono)
                        .text_color(rgb(p.prose))
                        .child(details),
                )
            })
            .when(self.extras.deny_open, |d| {
                d.child(self.render_deny_note(enabled, "Deny and tell it", cx))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child({
                        let card = card.clone();
                        self.primary_button("approve", "Allow once", enabled)
                            .debug_selector(|| "approve".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(Action::Approve(true, card.clone()), cx)
                                }))
                            })
                    })
                    // One button per grant the card offers, in the hub's own
                    // words: the user sees exactly what "always" would allow.
                    .children(always.into_iter().enumerate().map(|(ix, option)| {
                        let card = card.clone();
                        let enabled = enabled && card.is_some();
                        self.button(
                            SharedString::from(format!("always-option-{ix}")),
                            option.label.clone(),
                            enabled,
                        )
                        .debug_selector(move || format!("always-option-{ix}"))
                        .when(enabled, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                let Some(card) = card.clone() else {
                                    return;
                                };
                                this.act(
                                    Action::AlwaysAllow {
                                        option: option.id.clone(),
                                        label: option.label.clone(),
                                        card,
                                    },
                                    cx,
                                )
                            }))
                        })
                    }))
                    .child({
                        let card = card.clone();
                        self.button("deny", "Deny", enabled)
                            .debug_selector(|| "deny".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(Action::Approve(false, card.clone()), cx)
                                }))
                            })
                    })
                    // Claude reads a deny's note (stream: the deny message; PTY:
                    // the hook's reason); Codex gets it as a user message
                    // ("Denied …: note"). Other providers have no field for it.
                    .when(
                        !self.extras.deny_open
                            && self
                                .selected_session()
                                .is_some_and(|s| matches!(s.provider_id(), "claude" | "codex")),
                        |d| {
                            d.child(
                                self.button("deny-with-note", "Deny with note…", enabled)
                                    .debug_selector(|| "deny-with-note".into())
                                    .when(enabled, |d| {
                                        let card = card.clone();
                                        d.on_click(cx.listener(move |this, _, window, cx| {
                                            this.tools.deny_card = card.clone();
                                            this.extras.deny_open = true;
                                            this.extras
                                                .deny_note
                                                .update(cx, |input, cx| input.focus(window, cx));
                                            cx.notify();
                                        }))
                                    }),
                            )
                        },
                    ),
            )
    }

    /// Claude finished planning (`ExitPlanMode` waiting for approval): the
    /// plan itself, readable, with Approve plan / Keep planning. Keeping on
    /// planning is a deny with a note, so the feedback reaches the model.
    fn render_plan_card(
        &self,
        plan: &str,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let (compact, enabled) = (layout.compact, layout.enabled);
        let card = self.selected_session().and_then(Session::card);
        let note_card = card.clone();
        div()
            .debug_selector(|| "plan-card".into())
            .occlude()
            .w_full()
            .p(px(if compact { 8. } else { 12. }))
            .rounded(px(p.panel_radius))
            .shadow(chrome::floating_shadow(p))
            .bg(rgb(p.surface))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(p.accent))
                    .child(status_dot(p.accent))
                    .child("Plan ready · review it before Claude starts"),
            )
            .child(
                div()
                    .id("plan-body")
                    .debug_selector(|| "plan-body".into())
                    .max_h(px(if compact { 140. } else { 320. }))
                    .overflow_y_scroll()
                    .px_3()
                    .py_2()
                    .rounded(px(p.control_radius))
                    .bg(rgb(p.chat))
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(self.render_markdown("approval-plan", plan, window, cx)),
            )
            .when(self.extras.deny_open, |d| {
                d.child(self.render_deny_note(enabled, "Keep planning", cx))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.primary_button("approve", "Approve plan", enabled)
                            .debug_selector(|| "plan-approve".into())
                            .when(enabled, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(Action::Approve(true, card.clone()), cx)
                                }))
                            }),
                    )
                    .when(!self.extras.deny_open, |d| {
                        d.child(
                            self.button("plan-revise", "Keep planning…", enabled)
                                .debug_selector(|| "plan-revise".into())
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        this.tools.deny_card = note_card.clone();
                                        this.extras.deny_open = true;
                                        this.extras
                                            .deny_note
                                            .update(cx, |input, cx| input.focus(window, cx));
                                        cx.notify();
                                    }))
                                }),
                        )
                    }),
            )
    }

    /// The account behind this session is out of a usage window: say which,
    /// when it comes back, and offer to carry on with the other provider (the
    /// existing handoff). Short of that, the provider's own warning as a line.
    fn render_limit_card(
        &self,
        session: &Session,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let p = self.appearance.palette();
        let limits = &session.limits;
        let card = || {
            div()
                .debug_selector(|| "limit-card".into())
                .occlude()
                .w_full()
                .p(px(if compact { 8. } else { 12. }))
                .rounded(px(p.panel_radius))
                .shadow(chrome::floating_shadow(p))
                .bg(rgb(p.surface))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_2()
                .text_size(px(12.))
        };
        let Some((window, resets_at)) = limits.exhausted() else {
            let warning = limits.warning.clone()?;
            return Some(
                card().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(rgb(p.warning))
                        .child(status_dot(p.warning))
                        .child(warning),
                ),
            );
        };
        if session.working()
            || self.extras.limit_dismissed.as_ref()
                == Some(&(session.id.clone(), window, resets_at))
        {
            return None;
        }
        let provider = match session.provider_id() {
            "codex" => "Codex",
            _ => "Claude",
        };
        let when = resets_at
            .map(|at| {
                let now = wks_native::timing::now_ms() / 1000;
                format!(" · resets in {}", wks_native::usage::resets_in(at, now))
            })
            .unwrap_or_default();
        let other = if provider == "Codex" {
            "Continue with Claude…"
        } else {
            "Continue with Codex…"
        };
        let can_hand_off = self.supported_session() && self.view.connected && !self.demo;
        let key = (session.id.clone(), window, resets_at);
        Some(
            card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(rgb(p.warning))
                        .child(status_dot(p.warning))
                        .child(format!("{provider}'s {window} usage limit is reached{when}")),
                )
                .child(
                    div()
                        .text_color(rgb(p.muted))
                        .child("Messages wait until it resets. A handoff carries this work to the other agent now."),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            self.primary_button("limit-handoff", other, can_hand_off)
                                .debug_selector(|| "limit-handoff".into())
                                .when(can_hand_off, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.open_feature(Screen::Handoff, window, cx)
                                    }))
                                }),
                        )
                        .child(
                            self.button("limit-dismiss", "Wait for the reset", true)
                                .debug_selector(|| "limit-dismiss".into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.extras.limit_dismissed = Some(key.clone());
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    /// The one-line note box under a Deny with note / Keep planning.
    fn render_deny_note(&self, enabled: bool, send: &'static str, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(|| "deny-note".into())
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        let k = &event.keystroke;
                        if k.key == "enter" && !k.modifiers.shift {
                            cx.stop_propagation();
                            this.send_deny_note(window, cx);
                        } else if k.key == "escape" {
                            cx.stop_propagation();
                            this.extras.deny_open = false;
                            cx.notify();
                        }
                    }))
                    .child(Input::new(&self.extras.deny_note)),
            )
            .child(
                self.button("deny-note-send", send, enabled)
                    .debug_selector(|| "deny-note-send".into())
                    .when(enabled, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.send_deny_note(window, cx)),
                        )
                    }),
            )
    }

    /// Deny with the typed note; an empty note focuses the box instead.
    pub(super) fn send_deny_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let note = self.extras.deny_note.read(cx).value().trim().to_owned();
        if note.is_empty() {
            self.extras
                .deny_note
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        self.extras
            .deny_note
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.extras.deny_open = false;
        self.act(Action::DenyWithNote(note, self.tools.deny_card.clone()), cx);
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn approval_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.button(id, label, true)
            .on_click(cx.listener(|this, _, _, cx| {
                this.extras.approval_details = !this.extras.approval_details;
                cx.notify();
            }))
    }

    /// The floating composer with its draft attachments, and the activity
    /// and send-hint line under it.
    fn render_composer(
        &mut self,
        layout: ChatLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let ChatLayout {
            narrow,
            compact,
            enabled,
            working,
        } = layout;
        let uploading = self.uploading();
        let can_send = enabled && !uploading;
        let selected = self.selected_session();
        let draft_cwd = selected.map(|s| s.cwd.clone()).unwrap_or_default();
        // The title capsule carries the context gauge; only where it is too
        // narrow does the composer keep its meter.
        let meter = selected
            .filter(|_| !self.title_carries_context())
            .and_then(|s| gauge::context_meter(s, p));
        let draft_files = self.draft_files(cx);
        let viewport = window.viewport_size().height;
        let attachments_share = if unzoom(viewport) < 300. { 0.05 } else { 0.10 };
        let focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        // A round tonal stop holding the familiar filled square.
        let stop = chrome::interactive_control(div().id("stop"), p, enabled)
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .bg(rgb(p.selected))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Interrupt").build(window, cx)
            })
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(rgb(p.border)))
                    .active(|s| s.bg(rgb(p.border)))
            })
            .child(div().size(px(10.)).rounded(px(2.5)).bg(rgb(if enabled {
                p.text
            } else {
                p.disabled
            })));
        // Mid-turn the send steers the running turn; Queue holds the draft
        // for the turn's end and Send now has the agent read it at once.
        let steering = self.composer_busy();
        let send_label = if self.view.busy {
            "Sending…"
        } else {
            "Send message"
        };
        let tip = |text: &'static str| {
            move |window: &mut Window, cx: &mut App| {
                gpui_component::tooltip::Tooltip::new(text).build(window, cx)
            }
        };
        // Mid-turn the round Send gives way to Queue and a filled Send now
        // (icon-only where the window is narrow).
        let queue_button = if narrow {
            self.icon_button(
                "composer-queue",
                "Queue for when this turn ends (Tab)",
                IconName::Inbox,
                can_send,
            )
            .size(px(32.))
            .bg(rgb(p.selected))
        } else {
            self.pill_button("composer-queue", "Queue", IconName::Inbox, can_send, false)
                .tooltip(tip("Hold it until this turn ends (Tab)"))
        }
        .debug_selector(|| "composer-queue".into())
        .when(can_send, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.submit(Submit::Queue, window, cx)))
        });
        let now_button = if narrow {
            self.primary_icon_button(
                "composer-send-now",
                "Send now — the agent reads it right away",
                IconName::ArrowUp,
                can_send,
            )
            .size(px(32.))
        } else {
            self.pill_button(
                "composer-send-now",
                "Send now",
                IconName::ArrowUp,
                can_send,
                true,
            )
            .tooltip(tip(
                "The agent sets its current work aside and reads it right away",
            ))
        }
        .debug_selector(|| "composer-send-now".into())
        .when(can_send, |d| {
            d.on_click(cx.listener(|this, _, window, cx| this.submit(Submit::Now, window, cx)))
        });
        let attach_label = if uploading {
            "Attaching file…"
        } else {
            "Attach a file (or paste an image)"
        };
        let slash_rows = self.slash_menu(cx);
        let edge: gpui::Hsla = if focused {
            rgb(p.accent).into()
        } else {
            gpui::Hsla::from(rgb(p.border)).opacity(0.55)
        };
        let activity = self.composer_activity(layout);
        let notch = activity
            .live
            .then(|| self.render_notch(&activity.label, activity.animate, edge, compact));
        let composer = div()
            .id("floating-composer")
            .debug_selector(|| "chat-composer".into())
            .key_context(match (self.settings.enter_sends, slash_rows.is_empty()) {
                (true, true) => "Composer ComposerEnter",
                (false, true) => "Composer",
                (true, false) => "Composer ComposerEnter SlashMenu",
                (false, false) => "Composer SlashMenu",
            })
            .occlude()
            .relative()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(edge)
            .rounded(px(p.composer_radius))
            .shadow(chrome::floating_shadow(p))
            .px(px(if compact { 10. } else { 14. }))
            .pt(px(if compact { 8. } else { 14. }))
            .pb(px(if compact { 8. } else { 10. }))
            .flex()
            .flex_col()
            .gap_2()
            .when(!draft_files.is_empty() || uploading, |d| {
                d.child(
                    div()
                        .id("composer-attachments-scroll")
                        .debug_selector(|| "composer-attachments-scroll".into())
                        .max_h((viewport * attachments_share).min(px(80.)))
                        .overflow_y_scroll()
                        .child(self.render_draft_attachments(draft_files, &draft_cwd, cx)),
                )
            })
            .when(!slash_rows.is_empty(), |d| {
                d.child(self.render_slash_menu(slash_rows, cx))
            })
            .child(
                Input::new(&self.composer)
                    .appearance(false)
                    .disabled(self.view.busy),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                self.icon_button(
                                    "attach-file",
                                    attach_label,
                                    IconName::Plus,
                                    can_send,
                                )
                                .size(px(32.))
                                .when(can_send, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_attachment(window, cx)
                                    }))
                                }),
                            )
                            .children(meter),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(working, |d| {
                                d.child(stop.when(enabled, |d| {
                                    d.on_click(
                                        cx.listener(|this, _, _, cx| this.act(Action::Stop, cx)),
                                    )
                                }))
                            })
                            .when(steering, |d| d.child(queue_button).child(now_button))
                            // Queue and Send now replace Send mid-turn; Enter
                            // still steers from the keyboard.
                            .when(!steering, |d| {
                                d.child(
                                    self.primary_icon_button(
                                        "send",
                                        send_label,
                                        IconName::ArrowUp,
                                        can_send,
                                    )
                                    .debug_selector(|| "composer-send".into())
                                    .size(px(32.))
                                    .rounded_full()
                                    .when(can_send, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.send(&SendMessage, window, cx)
                                        }))
                                    }),
                                )
                            }),
                    ),
            );
        div()
            .debug_selector(|| "chat-composer-group".into())
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.render_cold_note(cx))
            // The tab paints after the composer: GPUI draws a div's border
            // over its own children, so only a sibling can open it.
            .child(div().relative().child(composer).children(notch))
            .child(self.render_activity_line(layout, steering, activity))
    }

    /// Draft attachments with their thumbnails (images only; PDFs keep the
    /// file chip). Loading and failure fall back to the name.
    fn draft_files(&mut self, cx: &mut Context<Self>) -> Vec<attachments::DraftFile> {
        self.view
            .selected
            .as_ref()
            .and_then(|id| self.extras.attachments.get(id))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, path)| {
                let preview = if name.to_lowercase().ends_with(".pdf") {
                    Some(None)
                } else {
                    self.attachment_preview(&path, cx)
                };
                (name, path, preview)
            })
            .collect()
    }

    /// What the session is doing, beside how to send.
    /// What the selected conversation is doing. Live states (connecting,
    /// loading, sending, working, waiting) ride in the composer's notch; the
    /// rest (a finished turn's summary, Ready) stay on the hint line.
    fn composer_activity(&self, layout: ChatLayout) -> ComposerActivity {
        let now = timing::now_ms();
        let clock = self
            .view
            .selected
            .as_ref()
            .and_then(|id| self.turn_clocks.get(id));
        let animate = (!self.view.connected && self.connection_copy().animated)
            || self.view.loading
            || self.view.busy
            || layout.working;
        let live = if !self.view.connected {
            Some(self.connection_copy().label.to_owned())
        } else if self.view.loading {
            Some("Loading conversation…".to_owned())
        } else if self.view.busy {
            Some("Sending…".to_owned())
        } else if layout.working {
            Some(
                clock
                    .and_then(|clock| clock.elapsed_label(now))
                    .map(|elapsed| format!("Working · {elapsed}"))
                    .unwrap_or_else(|| "Working…".into()),
            )
        } else if clock.and_then(|clock| clock.elapsed_label(now)).is_some() {
            Some("Waiting…".to_owned())
        } else {
            None
        };
        if let Some(label) = live {
            return ComposerActivity {
                label,
                live: true,
                animate,
            };
        }
        let label = clock
            .and_then(|clock| clock.latest_completion_for(&self.view.transcript))
            .unwrap_or_else(|| {
                if layout.enabled {
                    "Ready"
                } else {
                    "Session unavailable"
                }
                .into()
            });
        ComposerActivity {
            label,
            live: false,
            animate: false,
        }
    }

    /// The live status on a tab joined to the composer's top edge: sloped
    /// sides with flared feet, its base covering the composer's border so
    /// the two read as one surface.
    fn render_notch(&self, label: &str, animate: bool, edge: gpui::Hsla, compact: bool) -> Div {
        let p = self.appearance.palette();
        let height = notch_height(compact);
        let fill = gpui::Hsla::from(rgb(p.surface));
        div()
            .absolute()
            // The tab's base covers the composer's 1px top border.
            .top(px(-height))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .debug_selector(|| "composer-notch".into())
                    .relative()
                    // Sized by its content: the label never spills past the
                    // sloped sides.
                    .flex_shrink_0()
                    // The extra (unzoomed) pixel covers the composer's border.
                    .h(px(height) + gpui::px(1.))
                    .max_w(px(320.))
                    .px(px(NOTCH_SLANT + 10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(p.text))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let (fill_path, stroke_path) = notch_paths(bounds, height);
                                if let Ok(path) = fill_path {
                                    window.paint_path(path, fill);
                                }
                                if let Ok(path) = stroke_path {
                                    window.paint_path(path, edge);
                                }
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .child(if animate {
                        brand_spinner(11., p)
                    } else {
                        status_dot(p.warning)
                    })
                    .child(
                        div()
                            .debug_selector(|| "composer-notch-label".into())
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .child(label.to_owned()),
                    ),
            )
    }

    fn render_activity_line(
        &self,
        layout: ChatLayout,
        steering: bool,
        activity: ComposerActivity,
    ) -> Div {
        let p = self.appearance.palette();
        let enter_sends = self.settings.enter_sends;
        let send_key = if enter_sends {
            "Enter"
        } else if cfg!(target_os = "macos") {
            "⌘ Enter"
        } else {
            "Ctrl Enter"
        };
        let now_key = match (enter_sends, cfg!(target_os = "macos")) {
            (true, true) => "⌘ Enter",
            (true, false) => "Ctrl Enter",
            (false, true) => "⌘ ⇧ Enter",
            (false, false) => "Ctrl Shift Enter",
        };
        div()
            .when(layout.compact, |d| d.hidden())
            .occlude()
            .bg(rgb(p.chat))
            .rounded_md()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .text_size(px(10.))
            .text_color(rgb(p.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    // Live states show in the notch instead.
                    .when(!activity.live, |d| {
                        d.child(div().truncate().child(activity.label))
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "composer-send-hint".into())
                    .flex()
                    .gap_1()
                    .items_center()
                    .flex_shrink_0()
                    .child(keycap(send_key, p))
                    .map(|d| {
                        if !steering {
                            return d.child("to send");
                        }
                        // Mid-turn: how each key delivers the draft.
                        d.child("steer")
                            .child(keycap("Tab", p))
                            .child("queue")
                            .when(!layout.narrow, |d| {
                                d.child(keycap(now_key, p)).child("send now")
                            })
                    }),
            )
            .when(!layout.narrow && !steering, |d| {
                d.child(if enter_sends {
                    "Shift Enter for a new line"
                } else {
                    "Enter for a new line"
                })
            })
    }
}
