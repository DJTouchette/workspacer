//! The conversation page: the transcript under a floating title header, and
//! the dock below it with the pending cards (approval, questions, children)
//! above the composer.
use super::*;

/// What the conversation decided once per frame, for the pieces it builds.
#[derive(Clone, Copy)]
struct ChatLayout {
    narrow: bool,
    compact: bool,
    /// The selected session can take a message or decision now.
    enabled: bool,
    working: bool,
}

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
            .when(!self.follow, |d| d.child(self.render_jump_latest(cx)))
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
                Some(self.chat_actions(layout.enabled, cx)),
            ),
        };
        let title_island = self.render_title_island(title_bar, title_actions, notice, window, cx);
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
                    .child(title_island),
            )
    }

    /// The notice the title capsule carries: this window's own first, and a
    /// local backend that is still starting says so in the empty state
    /// instead.
    fn title_notice(&self) -> String {
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
                d.child(self.render_approval_card(approval, layout, cx))
            })
            .when_some(selected.filter(|s| s.questions.is_some()), |d, session| {
                d.child(self.render_questions(session, layout.enabled, compact, window, cx))
            })
            .children(self.render_child_bar(cx));
        let composer_layer = chrome::chat_column()
            .relative()
            .flex_shrink_0()
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
        cx: &mut Context<Self>,
    ) -> Div {
        use serde_json::Value;
        let p = self.appearance.palette();
        let (compact, enabled) = (layout.compact, layout.enabled);
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
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .debug_selector(|| "deny-note".into())
                                .on_key_down(cx.listener(
                                    |this, event: &gpui::KeyDownEvent, window, cx| {
                                        let k = &event.keystroke;
                                        if k.key == "enter" && !k.modifiers.shift {
                                            cx.stop_propagation();
                                            this.send_deny_note(window, cx);
                                        } else if k.key == "escape" {
                                            cx.stop_propagation();
                                            this.extras.deny_open = false;
                                            cx.notify();
                                        }
                                    },
                                ))
                                .child(Input::new(&self.extras.deny_note)),
                        )
                        .child(
                            self.button("deny-note-send", "Deny and tell it", enabled)
                                .debug_selector(|| "deny-note-send".into())
                                .when(enabled, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.send_deny_note(window, cx)
                                    }))
                                }),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.primary_button("approve", "Allow once", enabled).when(
                        enabled,
                        |d| {
                            d.on_click(
                                cx.listener(|this, _, _, cx| this.act(Action::Approve(true), cx)),
                            )
                        },
                    ))
                    .child(self.button("deny", "Deny", enabled).when(enabled, |d| {
                        d.on_click(
                            cx.listener(|this, _, _, cx| this.act(Action::Approve(false), cx)),
                        )
                    }))
                    // Only Claude reads a deny's note (stream: the deny message; PTY: the
                    // hook's reason); other providers' approval APIs have no field for it.
                    .when(
                        !self.extras.deny_open
                            && self
                                .selected_session()
                                .is_some_and(|s| s.provider_id() == "claude"),
                        |d| {
                            d.child(
                                self.button("deny-with-note", "Deny with note…", enabled)
                                    .debug_selector(|| "deny-with-note".into())
                                    .when(enabled, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
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
        self.act(Action::DenyWithNote(note), cx);
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
        let stop = if narrow {
            self.icon_button("stop", "Interrupt", IconName::WindowClose, enabled)
        } else {
            self.quiet_button("stop", "Interrupt", IconName::WindowClose, enabled)
        };
        let send_label = if self.view.busy {
            "Sending…"
        } else if working {
            "Queue message"
        } else {
            "Send message"
        };
        let attach_label = if uploading {
            "Attaching file…"
        } else {
            "Attach a file (or paste an image)"
        };
        let composer = div()
            .id("floating-composer")
            .debug_selector(|| "chat-composer".into())
            .key_context(if self.settings.enter_sends {
                "Composer ComposerEnter"
            } else {
                "Composer"
            })
            .occlude()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(if focused {
                rgb(p.accent).into()
            } else {
                gpui::Hsla::from(rgb(p.border)).opacity(0.55)
            })
            .rounded(px(p.composer_radius))
            .shadow(chrome::floating_shadow(p))
            .p(px(if compact { 8. } else { 12. }))
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
                            .child(
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
                            ),
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
            .child(composer)
            .child(self.render_activity_line(layout))
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
    fn render_activity_line(&self, layout: ChatLayout) -> Div {
        let p = self.appearance.palette();
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
        let activity = if !self.view.connected {
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
            clock.and_then(|clock| clock.latest_completion_for(&self.view.transcript))
        };
        let activity = activity.unwrap_or_else(|| {
            if layout.enabled {
                "Ready"
            } else {
                "Session unavailable"
            }
            .into()
        });
        let enter_sends = self.settings.enter_sends;
        let send_key = if enter_sends {
            "Enter"
        } else if cfg!(target_os = "macos") {
            "⌘ Enter"
        } else {
            "Ctrl Enter"
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
                    .when(animate, |d| {
                        d.child(brand_spinner(12., p, "composer-activity"))
                    })
                    .child(div().truncate().child(activity)),
            )
            .child(
                div()
                    .debug_selector(|| "composer-send-hint".into())
                    .flex()
                    .gap_1()
                    .items_center()
                    .flex_shrink_0()
                    .child(keycap(send_key, p))
                    .child("to send"),
            )
            .when(!layout.narrow, |d| {
                d.child(if enter_sends {
                    "Shift Enter for a new line"
                } else {
                    "Enter for a new line"
                })
            })
    }
}
