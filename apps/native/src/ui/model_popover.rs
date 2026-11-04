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

    /// The live access choice. Until the agent reports a mode this app
    /// offers, nothing is selected and the note says so: "unknown" is never
    /// a choice. A choice applies at once.
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
                    "{} Changes at once; nothing restarts.",
                    current.description()
                )
            }
            (false, None) if session.permission_mode.is_empty() => {
                "The agent has not reported its access yet. Choosing one sets it.".to_owned()
            }
            (false, None) => format!(
                "Running in “{}”, which this app does not offer. Choosing one sets it.",
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
                            Permission::Ask => "Ask first",
                            Permission::AcceptEdits => "Can edit",
                            Permission::Plan => "Plan only",
                            Permission::FullAccess => "Full access",
                        };
                        primitives::segment(id.clone(), label, active, &p)
                            .debug_selector(move || id.to_string())
                            .flex_1()
                            .tooltip(tooltip_view(permission.description(), None, &p))
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
            );
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
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(note),
            )
    }

    /// The model as a real list (human names, raw id on hover), then
    /// "Custom…", which reveals the exact-id field.
    fn render_model_list(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let catalog = &self.view.catalog;
        let loading = catalog.key == self.catalog_key(cx) && catalog.loading;
        // An exact id typed (or the session's own) that the list knows
        // selects its row rather than "Custom…".
        let typed = self.model.read(cx).value().trim().to_owned();
        let known =
            self.model_choice == "__custom" && self.catalog_models.iter().any(|m| m.id == typed);
        let custom = self.model_choice == "__custom" && !known;
        let chosen = if known {
            typed
        } else {
            self.model_choice.clone()
        };
        let row = |id: SharedString, label: String, raw: String, selected: bool| {
            chrome::interactive_control(div().id(id.clone()), p, !busy)
                .debug_selector(move || id.to_string())
                .h(px(t.layout.row_height))
                .px(px(t.space.md))
                .rounded(px(t.radius.control))
                .flex()
                .items_center()
                .gap(px(t.space.sm))
                .text_size(px(t.text.body))
                .text_color(rgb(if selected {
                    p.text
                } else {
                    p.roles.text_secondary
                }))
                .when(selected, |d| d.bg(rgb(p.selected)))
                .when(!selected && !busy, |d| {
                    d.hover(move |s| s.bg(rgb(p.roles.hover)))
                })
                .when(!raw.is_empty(), |d| d.tooltip(tooltip_view(raw, None, &p)))
                .child(div().flex_1().min_w_0().truncate().child(label))
                .when(selected, |d| {
                    d.child(
                        Icon::new(IconName::Check)
                            .size(px(t.icon.sm))
                            .text_color(rgb(p.accent)),
                    )
                })
        };
        let mut list = div()
            .debug_selector(|| "popover-model-list".into())
            .flex()
            .flex_col()
            .gap(px(t.space.xxs));
        for (n, model) in self.catalog_models.iter().enumerate() {
            let name = wks_native::format::model_name(&model.id);
            let label = if model.is_default {
                format!("{name} · default")
            } else {
                name
            };
            let id = model.id.clone();
            let window = model.windows.first().copied();
            list = list.child(
                row(
                    format!("popover-model-{n}").into(),
                    label,
                    model.id.clone(),
                    !custom && chosen == model.id,
                )
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, window_, cx| {
                        if this.model_choice != id {
                            this.model_choice = id.clone();
                            this.context_window = window;
                            this.reconcile_effort(window_, cx);
                        }
                        cx.notify();
                    }))
                }),
            );
        }
        if self.catalog_models.is_empty() {
            list = list.child(
                div()
                    .px(px(t.space.md))
                    .py(px(t.space.xs))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child(if loading {
                        "Loading models…"
                    } else {
                        "No model list from this agent: enter a model id."
                    }),
            );
        }
        list = list.child(
            row(
                "popover-model-custom".into(),
                "Custom…".into(),
                String::new(),
                custom,
            )
            .when(!busy, |d| {
                d.on_click(cx.listener(|this, _, window, cx| {
                    this.model_choice = "__custom".into();
                    this.model.update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                }))
            }),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.sm))
            .child(field_label("Model", &p))
            .child(
                div()
                    .id("popover-model-scroll")
                    .max_h(px(t.layout.row_height * 6.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .when(custom, |d| {
                d.child(
                    div()
                        .debug_selector(|| "popover-model-input".into())
                        .h(px(t.layout.control_height_sm + t.space.xs))
                        .px(px(t.space.md))
                        .flex()
                        .items_center()
                        .rounded(px(t.radius.control))
                        .border(px(t.stroke.hairline))
                        .border_color(rgb(p.roles.border_strong))
                        .child(Input::new(&self.model).appearance(false).disabled(busy)),
                )
            })
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
        let viewport = window.viewport_size();
        // Hangs from its chip: below it in the top half of the window,
        // above it (the composer's chip) in the bottom half; without a chip,
        // under the title.
        let (left, top, bottom) = if anchor.size.width > px(0.) {
            let left = (anchor.center().x - px(width / 2.))
                .max(px(t.space.md))
                .min(viewport.width - px(width + t.space.md));
            if anchor.center().y > viewport.height / 2. {
                (
                    left,
                    None,
                    Some(viewport.height - anchor.top() + px(t.space.sm)),
                )
            } else {
                (left, Some(anchor.bottom() + px(t.space.sm)), None)
            }
        } else {
            (
                (viewport.width - px(width)) / 2.,
                Some(self.header_bounds.bottom().max(px(t.layout.page_top))),
                None,
            )
        };
        let room = match (top, bottom) {
            (Some(top), _) => viewport.height - top - px(t.space.xl),
            (_, Some(bottom)) => viewport.height - bottom - px(t.space.xl),
            _ => viewport.height,
        };
        let card = primitives::popover(&p)
            .id("model-popover")
            .debug_selector(|| "model-popover".into())
            .occlude()
            .w(px(width))
            .max_h(room)
            .overflow_y_scroll()
            .p(px(t.space.lg))
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
                            .child("Model and access"),
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
            .child(self.render_model_list(busy, cx))
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
            .child(self.render_access_choice(busy, cx))
            .child(
                div()
                    .pt(px(t.space.md))
                    .border_t(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .child(
                        primitives::button(
                            "model-popover-page",
                            "All model settings",
                            None,
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
                    )
                    .child(div().flex_1())
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
                        .tooltip(tooltip_view(
                            "Model and effort apply from the next turn; a busy agent may queue it",
                            None,
                            &p,
                        ))
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.apply_model_change(cx);
                                this.shell.model_popover = false;
                                cx.notify();
                            }))
                        }),
                    ),
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
                    .child(
                        card.absolute()
                            .left(left)
                            .when_some(top, |d, top| d.top(top))
                            .when_some(bottom, |d, bottom| d.bottom(bottom)),
                    ),
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
