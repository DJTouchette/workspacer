//! Design v2: model, effort and access in a popover hanging from the title
//! capsule's model chip, instead of a trip to the full Model page (which
//! stays one click away, and on `g m`).
use super::primitives::{self, ButtonSize, ButtonVariant, tooltip_view};
use super::*;
use gpui::{AnyElement, deferred};

impl Workspace {
    /// Open the popover on the selected session's current model, effort and
    /// context window: the same starting point the Model page uses.
    pub(super) fn open_model_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.design().is_v2() {
            self.open_feature(Screen::Model, window, cx);
            return;
        }
        if self.shell.model_popover {
            self.shell.model_popover = false;
            cx.notify();
            return;
        }
        let Some(s) = self.selected_session().cloned() else {
            return;
        };
        if !self.supported_session() {
            self.extras.notice =
                "Model controls are available for Claude and Codex sessions.".into();
            cx.notify();
            return;
        }
        let provider = if s.provider == "codex" {
            "codex"
        } else {
            "claude"
        };
        self.choose_provider(provider, window, cx);
        // The catalog is scoped to the session's own folder.
        self.projects.cwd = s.cwd.clone();
        self.model_choice = "__custom".into();
        self.model_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&String::from("__custom"), window, cx)
        });
        self.model
            .update(cx, |input, cx| input.set_value(s.model.clone(), window, cx));
        self.context_window = s.context_window;
        self.effort = s.effort.clone();
        self.extras.model_base = Some((s.model.clone(), s.context_window, s.effort.clone()));
        self.select_exact_model(window, cx);
        self.reconcile_effort(window, cx);
        self.load_models(true, cx);
        self.shell.model_popover = true;
        cx.notify();
    }

    /// The live access choice, with an explicit "Unknown" while the
    /// provider has not reported a mode this client offers.
    fn render_access_choice(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let Some(session) = self.selected_session() else {
            return div();
        };
        let provider = session.provider_id().to_owned();
        let current = Permission::from_wire(&provider, session.access_mode());
        let switching = self.view.busy && self.extras.access_switching;
        let note = match (switching, current) {
            (true, _) => "Switching…".to_owned(),
            (false, Some(current)) => {
                format!(
                    "{} Applies at once; nothing restarts.",
                    current.description()
                )
            }
            (false, None) if session.permission_mode.is_empty() => {
                "The agent has not reported its mode yet. Choosing one sets it.".to_owned()
            }
            (false, None) => format!(
                "Running in “{}”, which this app does not offer.",
                session.permission_mode
            ),
        };
        let track = primitives::segmented(&p)
            .debug_selector(|| "popover-access".into())
            .children(
                Permission::choices(&provider)
                    .iter()
                    .copied()
                    .map(|permission| {
                        let id =
                            SharedString::from(format!("popover-access-{}", permission.label()));
                        let active = current == Some(permission);
                        let provider = provider.clone();
                        let label = match permission {
                            Permission::Ask => "Ask",
                            Permission::AcceptEdits => "Edits",
                            Permission::Plan => "Plan",
                            Permission::FullAccess => "Full",
                        };
                        primitives::segment(id.clone(), label, active, &p)
                            .debug_selector(move || id.to_string())
                            .tooltip(tooltip_view(permission.label(), None, &p))
                            .when(active && permission == Permission::FullAccess, |d| {
                                d.text_color(rgb(p.warning))
                            })
                            .when(!busy && !active, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    let Ok(mode) = permission.wire(&provider) else {
                                        return;
                                    };
                                    this.extras.access_switching = true;
                                    this.act(
                                        Action::SetPermission {
                                            mode: mode.to_owned(),
                                            label: permission.label().to_owned(),
                                        },
                                        cx,
                                    );
                                }))
                            })
                    }),
            )
            .when(current.is_none(), |d| {
                d.child(
                    primitives::segment("popover-access-unknown", "Unknown", true, &p)
                        .debug_selector(|| "popover-access-unknown".into())
                        .text_color(rgb(p.roles.text_tertiary))
                        .tooltip(tooltip_view("The current mode is not known", None, &p)),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.sm))
            .child(field_label("Access", &p))
            .child(track)
            .child(
                div()
                    .text_size(px(t.text.meta))
                    .line_height(gpui::relative(t.text.line_height))
                    .text_color(rgb(p.roles.text_secondary))
                    .child(note),
            )
    }

    pub(super) fn render_model_popover(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.shell.model_popover
            || !self.design().is_v2()
            || self.screen != Screen::Conversation
            || self.new_session
            || self.selected_session().is_none()
        {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let busy = self.view.busy || !self.view.connected;
        let anchor = self.shell.model_anchor;
        let width = t.layout.dialog_width;
        // Centred under the chip, kept inside the window.
        let viewport = window.viewport_size();
        let left = (anchor.center().x - px(width / 2.))
            .max(px(t.space.md))
            .min(viewport.width - px(width + t.space.md));
        let top = anchor.bottom() + px(t.space.sm);
        let card = primitives::popover(&p)
            .id("model-popover")
            .debug_selector(|| "model-popover".into())
            .occlude()
            .w(px(width))
            .max_h(viewport.height - top - px(t.space.xl))
            .overflow_y_scroll()
            .p(px(t.space.xl))
            .gap(px(t.space.lg))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(t.text.subheading))
                            .font_weight(FontWeight(t.weight.semibold))
                            .text_color(rgb(p.text))
                            .child("Model, effort and access"),
                    )
                    .child(
                        primitives::icon_button(
                            "model-popover-close",
                            IconName::Close,
                            "Close",
                            ButtonVariant::Ghost,
                            true,
                            &p,
                        )
                        .debug_selector(|| "model-popover-close".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shell.model_popover = false;
                            cx.notify();
                        })),
                    ),
            )
            .child(self.render_model_select(busy, false, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.sm))
                    .child(field_label("Effort", &p))
                    .child(
                        div()
                            .debug_selector(|| "popover-effort-picker".into())
                            .child(Select::new(&self.effort_picker).disabled(busy)),
                    ),
            )
            .child(self.render_context_choice(busy, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child("From the next turn; a busy agent may queue it."),
                    )
                    .child(
                        primitives::button(
                            "apply-model",
                            "Apply",
                            None,
                            ButtonVariant::Primary,
                            ButtonSize::Small,
                            !busy,
                            &p,
                        )
                        .debug_selector(|| "popover-apply-model".into())
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.apply_model_change(cx)))
                        }),
                    ),
            )
            .child(div().h(px(t.stroke.hairline)).bg(rgb(p.border)))
            .child(self.render_access_choice(busy, cx))
            .child(
                primitives::button(
                    "model-popover-page",
                    "All model settings",
                    Some(IconName::ArrowRight),
                    ButtonVariant::Ghost,
                    ButtonSize::Small,
                    true,
                    &p,
                )
                .debug_selector(|| "model-popover-page".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.shell.model_popover = false;
                    this.open_feature(Screen::Model, window, cx);
                })),
            );
        Some(
            deferred(
                div()
                    .id("model-popover-layer")
                    .debug_selector(|| "model-popover-layer".into())
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.shell.model_popover = false;
                            cx.notify();
                        }),
                    )
                    .child(card.absolute().left(left).top(top)),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

fn field_label(text: &'static str, p: &Palette) -> Div {
    let t = p.tokens;
    div()
        .text_size(px(t.text.meta))
        .font_weight(FontWeight(t.weight.medium))
        .text_color(rgb(p.roles.text_secondary))
        .child(text)
}
