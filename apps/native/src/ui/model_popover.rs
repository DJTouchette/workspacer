//! Design v2: model, effort and access in a popover hanging from the
//! composer's model chip or the header's Model tool, instead of a trip to
//! the full Model page (which stays one click away, and on `g m`). It
//! hangs from the control that opened it (r5-shell, R5.4), and Up/Down,
//! Enter and Esc drive it.
use super::primitives::{self, ButtonSize, ButtonVariant, tooltip_view};
use super::system::{self, EdgeTone, IconButton, Side};
use super::*;
use gpui::{AnyElement, KeyDownEvent, deferred};

// r3-shell
/// The models this app knows per provider (R3.5): listed even when the
/// agent reports none, after whatever its catalog says. Curated; ids are
/// the providers' own.
pub(super) const KNOWN_MODELS: [(&str, &[&str]); 2] = [
    (
        "claude",
        &["claude-opus-5-5", "claude-sonnet-4-6", "claude-haiku-4-5"],
    ),
    ("codex", &["gpt-5.6-sol", "gpt-5.6-luna", "gpt-5.4-mini"]),
];

/// One row of the popover's model list.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ListedModel {
    pub(super) id: String,
    pub(super) is_default: bool,
    pub(super) window: Option<u64>,
}

/// The curated model a catalog's bare family alias stands for ("sonnet" →
/// `claude-sonnet-4-6`), so the alias and its curated twin are one row
/// (R3.5). `None` for anything but a bare alias of a known family.
pub(super) fn curated_twin(provider: &str, id: &str) -> Option<&'static str> {
    let alias = id.trim();
    if alias.is_empty() || !alias.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let infix = format!("-{alias}-");
    KNOWN_MODELS
        .iter()
        .find(|(p, _)| *p == provider)
        .and_then(|(_, ids)| ids.iter().find(|m| m.contains(&infix)))
        .copied()
}

/// The catalog's models, then the known ones it does not already name (by
/// id or by human name, so an alias that reads the same hides its twin). A
/// bare family alias becomes its curated row, which keeps the alias's
/// default flag and context window.
pub(super) fn listed_models(provider: &str, catalog: &[ModelChoice]) -> Vec<ListedModel> {
    let mut out: Vec<ListedModel> = Vec::new();
    for m in catalog {
        let id = curated_twin(provider, &m.id).unwrap_or(&m.id);
        if let Some(row) = out.iter_mut().find(|row| row.id == id) {
            row.is_default |= m.is_default;
            row.window = row.window.or(m.windows.first().copied());
            continue;
        }
        out.push(ListedModel {
            id: id.to_owned(),
            is_default: m.is_default,
            window: m.windows.first().copied(),
        });
    }
    let known = KNOWN_MODELS
        .iter()
        .find(|(p, _)| *p == provider)
        .map_or(&[][..], |(_, ids)| *ids);
    for id in known {
        let name = wks_native::format::model_name(id);
        let named = out
            .iter()
            .any(|m| m.id == *id || wks_native::format::model_name(&m.id) == name);
        if !named {
            out.push(ListedModel {
                id: (*id).to_owned(),
                is_default: false,
                window: None,
            });
        }
    }
    out
}

/// The listed row an id selects: the exact id, or for a bare family alias
/// ("sonnet", as agents report it) the first listed model of that family,
/// which the list orders newest first. Display only: the session keeps its
/// alias until the user picks a row.
pub(super) fn listed_match<'a>(models: &'a [ListedModel], id: &str) -> Option<&'a ListedModel> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    models.iter().find(|m| m.id == id).or_else(|| {
        let alias = id.to_ascii_lowercase();
        if !alias.chars().all(|c| c.is_ascii_lowercase()) {
            return None;
        }
        let (prefix, infix) = (format!("{alias}-"), format!("-{alias}-"));
        models.iter().find(|m| {
            let m = m.id.to_ascii_lowercase();
            m.starts_with(&prefix) || m.contains(&infix)
        })
    })
}

/// The access the popover shows as selected: the reported mode, or "Ask
/// first" (the agents' own default) until one is reported.
pub(super) fn effective_access(
    current: Option<Permission>,
    choices: &[Permission],
) -> Option<Permission> {
    current.or(Some(Permission::Ask).filter(|ask| choices.contains(ask)))
}

/// What an access mode lets the agent do, in one plain line (r5-shell:
/// each Access option says what it allows).
pub(super) fn access_consequence(permission: Permission) -> &'static str {
    match permission {
        Permission::Ask => "Asks before it runs a command or edits a file.",
        Permission::AcceptEdits => "Edits files without asking; asks before other tools.",
        Permission::Plan => "Reads and plans only; changes nothing until you approve.",
        Permission::FullAccess => "Runs commands and edits files without asking.",
    }
}

/// The v2 word for an access mode, the same in the popover and on the
/// Model page.
pub(super) fn access_word(permission: Permission) -> &'static str {
    match permission {
        Permission::Ask => "Ask first",
        Permission::AcceptEdits => "Can edit",
        Permission::Plan => "Plan only",
        Permission::FullAccess => "Full access",
    }
}

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
        self.shell.model_from_header = false;
        let models = self.popover_models();
        self.shell.model_cursor = models
            .iter()
            .position(|m| listed_match(&models, &s.model).is_some_and(|hit| hit.id == m.id))
            .unwrap_or(models.len());
        let focus = self
            .shell
            .model_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus);
        cx.notify();
    }

    /// The popover's models for the selected session's provider.
    pub(super) fn popover_models(&self) -> Vec<ListedModel> {
        let provider = match self.selected_session().map(|s| s.provider_id()) {
            Some("codex") => "codex",
            _ => "claude",
        };
        listed_models(provider, &self.catalog_models)
    }

    /// Close the popover and give the keys back to the window.
    pub(super) fn close_model_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shell.model_popover = false;
        window.focus(&self.focus);
        cx.notify();
    }

    /// Up/Down walk the models (then "Custom id…"), Enter picks and applies
    /// the row the keys are on, Esc closes.
    fn model_popover_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let models = self.popover_models();
        let last = models.len();
        match event.keystroke.key.as_str() {
            "down" => self.shell.model_cursor = (self.shell.model_cursor + 1).min(last),
            "up" => self.shell.model_cursor = self.shell.model_cursor.saturating_sub(1),
            "escape" => {
                self.close_model_popover(window, cx);
                cx.stop_propagation();
                return;
            }
            "enter" => {
                cx.stop_propagation();
                if self.view.busy || !self.view.connected {
                    return;
                }
                match models.get(self.shell.model_cursor) {
                    Some(model) => {
                        self.model_choice = model.id.clone();
                        self.context_window = model.window;
                        self.reconcile_effort(window, cx);
                        self.apply_model_change(cx);
                        self.close_model_popover(window, cx);
                    }
                    None => {
                        self.model_choice = "__custom".into();
                        self.model.update(cx, |input, cx| input.focus(window, cx));
                    }
                }
                cx.notify();
                return;
            }
            _ => return,
        }
        cx.stop_propagation();
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
        // R3.5: the effective mode is always shown selected; until the
        // agent reports one, that is its default, asking first.
        let shown = effective_access(current, Permission::choices(&provider));
        let switching = self.view.busy && self.extras.access_switching;
        // r5-shell: say what is in force, plainly.
        let note = match (switching, current, shown) {
            (true, _, _) => "Switching…".to_owned(),
            (false, Some(current), _) => format!(
                "Currently: {}. A change applies at once; nothing restarts.",
                access_word(current)
            ),
            (false, None, Some(shown)) if session.permission_mode.is_empty() => {
                format!("Currently: {} (the default).", access_word(shown))
            }
            (false, None, _) if session.permission_mode.is_empty() => {
                "Currently: the agent's default.".to_owned()
            }
            (false, None, _) => format!(
                "Currently: “{}”, which this app does not offer. Pick one to change it.",
                session.permission_mode
            ),
        };
        let track = div()
            .debug_selector(|| "popover-access".into())
            .flex()
            .flex_col()
            .gap(px(t.space.xxs))
            .children(
                Permission::choices(&provider)
                    .iter()
                    .copied()
                    .map(|permission| {
                        let id =
                            SharedString::from(format!("popover-access-{}", permission.label()));
                        let active = shown == Some(permission);
                        // Unreported, the shown default can still be set.
                        let settable = !active || current.is_none();
                        let provider = provider.clone();
                        let label = access_word(permission);
                        primitives::list_row(id.clone(), active, !busy, &p)
                            .debug_selector(move || id.to_string())
                            .py(px(t.space.xs))
                            .items_start()
                            .child(
                                div()
                                    .size(px(t.icon.sm))
                                    .mt(px(t.space.xxs))
                                    .flex_shrink_0()
                                    .when(active, |d| {
                                        d.child(
                                            Icon::new(IconName::Check)
                                                .size(px(t.icon.sm))
                                                .text_color(rgb(p.accent)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .font_weight(FontWeight(t.weight.medium))
                                            .text_color(rgb(if active {
                                                p.text
                                            } else {
                                                p.roles.text_secondary
                                            }))
                                            .child(label),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(t.text.meta))
                                            .text_color(rgb(p.roles.text_tertiary))
                                            .child(access_consequence(permission)),
                                    ),
                            )
                            .when(!busy && settable, |d| {
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
        let models = self.popover_models();
        let cursor = self.shell.model_cursor;
        let hit = (self.model_choice == "__custom")
            .then(|| listed_match(&models, &typed).map(|m| m.id.clone()))
            .flatten();
        let custom = self.model_choice == "__custom" && hit.is_none();
        let chosen = if let Some(hit) = hit {
            hit
        } else {
            self.model_choice.clone()
        };
        let row = |id: SharedString, label: String, raw: String, selected: bool, keyed: bool| {
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
                .when(keyed && !selected, |d| d.bg(rgb(p.roles.hover)))
                .when(keyed, |d| system::edge(d, Side::All, EdgeTone::Strong, &p))
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
        for (n, model) in models.iter().enumerate() {
            let name = wks_native::format::model_name(&model.id);
            let label = if model.is_default {
                format!("{name} · default")
            } else {
                name
            };
            let id = model.id.clone();
            let window = model.window;
            list = list.child(
                row(
                    format!("popover-model-{n}").into(),
                    label,
                    model.id.clone(),
                    !custom && chosen == model.id,
                    cursor == n,
                )
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, window_, cx| {
                        this.shell.model_cursor = n;
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
        if self.catalog_models.is_empty() && loading {
            list = list.child(
                div()
                    .px(px(t.space.md))
                    .py(px(t.space.xs))
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child("Checking the agent's own list…"),
            );
        }
        let last = models.len();
        list = list.child(
            row(
                "popover-model-custom".into(),
                "Custom id…".into(),
                String::new(),
                custom,
                cursor == last,
            )
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    this.shell.model_cursor = last;
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
                    system::edge(div(), Side::All, EdgeTone::Strong, &p)
                        .debug_selector(|| "popover-model-input".into())
                        .h(px(t.layout.control_height_sm + t.space.xs))
                        .px(px(t.space.md))
                        .flex()
                        .items_center()
                        .rounded(px(t.radius.control))
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
        // r5-shell (R5.4): it hangs from the control that opened it, the
        // composer's chip or the header's "…", whether or not a decision
        // waits (closing it shows the decision again).
        let from_header = self.shell.model_from_header || anchor.size.width <= gpui::Pixels::ZERO;
        // Hangs from its chip: below it in the top half of the window,
        // above it (the composer's chip) in the bottom half; from the
        // header, under the strip at its right, by the Model tool.
        let (left, top, bottom) = if from_header {
            let strip = self.header_bounds.top() + px(t.shell().header);
            let left = (self.header_bounds.right() - px(width + t.space.md))
                .max(px(t.space.md))
                .min(viewport.width - px(width + t.space.md));
            (left, Some(strip + px(t.space.xs)), None)
        } else if anchor.size.width > gpui::Pixels::ZERO {
            // Left edges aligned, as an anchored menu's (`BelowStart` /
            // `AboveStart`), kept inside the window.
            let left = anchor
                .left()
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
        let focus = self.shell.model_focus.clone();
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
            .when_some(focus, |d, focus| d.track_focus(&focus))
            .on_key_down(cx.listener(Self::model_popover_key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().child(primitives::card_header(
                        "Model and access",
                        None,
                        &p,
                    )))
                    .child(
                        IconButton::new("model-popover-close", IconName::Close, "Close")
                            .shortcut("Esc")
                            .render(&p)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_model_popover(window, cx)
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
                system::edge(div(), Side::Top, EdgeTone::Divider, &p)
                    .pt(px(t.space.md))
                    .flex()
                    .items_center()
                    .gap(px(t.space.sm))
                    .child(
                        primitives::button(
                            "model-popover-page",
                            "All model settings",
                            None,
                            ButtonVariant::Tertiary,
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
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.apply_model_change(cx);
                                this.close_model_popover(window, cx);
                            }))
                        }),
                    ),
            );
        // r6-surfaces (R6.1): the popover takes the keyboard, so its layer
        // is a scrim over the whole window (sidebar included), mounted at
        // the window root like every overlay; the card still hangs from the
        // control that opened it.
        Some(
            deferred(
                div()
                    .id("model-popover-layer")
                    .debug_selector(|| "model-popover-layer".into())
                    .absolute()
                    .inset_0()
                    .bg(system::scrim_color(&p))
                    .occlude()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_model_popover(window, cx)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str, is_default: bool) -> ModelChoice {
        ModelChoice {
            id: id.into(),
            label: String::new(),
            windows: vec![],
            is_default,
            efforts: vec![],
            default_effort: None,
        }
    }

    #[test]
    fn a_family_alias_selects_its_listed_model_not_custom() {
        let models = listed_models("claude", &[]);
        let hit = |id: &str| listed_match(&models, id).map(|m| m.id.as_str());
        assert_eq!(hit("claude-haiku-4-5"), Some("claude-haiku-4-5"));
        assert_eq!(hit("sonnet"), Some("claude-sonnet-4-6"));
        assert_eq!(hit(" Opus "), Some("claude-opus-5-5"));
        // Exact ids the list does not know stay custom.
        assert_eq!(hit("claude-sonnet-4-5"), None);
        assert_eq!(hit("my-finetune"), None);
        assert_eq!(hit(""), None);
    }

    #[test]
    fn known_models_are_listed_when_the_agent_reports_none() {
        let ids = |v: Vec<ListedModel>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(
            ids(listed_models("claude", &[])),
            ["claude-opus-5-5", "claude-sonnet-4-6", "claude-haiku-4-5"]
        );
        assert_eq!(
            ids(listed_models("codex", &[])),
            ["gpt-5.6-sol", "gpt-5.6-luna", "gpt-5.4-mini"]
        );
        // The catalog leads; a known model it already names (by id or by
        // the same human name) is not listed twice.
        let catalog = [
            choice("claude-sonnet-4-6", true),
            choice("claude-opus-5-5-20260101", false),
        ];
        assert_eq!(
            ids(listed_models("claude", &catalog)),
            [
                "claude-sonnet-4-6",
                "claude-opus-5-5-20260101",
                "claude-haiku-4-5"
            ]
        );
        assert!(listed_models("claude", &catalog)[0].is_default);
    }

    #[test]
    fn a_catalog_alias_is_its_curated_row_not_a_second_one() {
        let mut sonnet = choice("sonnet", true);
        sonnet.windows = vec![200_000, 1_000_000];
        let catalog = [choice("opus", false), sonnet, choice("fable", false)];
        let models = listed_models("claude", &catalog);
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude-opus-5-5",
                "claude-sonnet-4-6",
                "fable",
                "claude-haiku-4-5"
            ]
        );
        assert!(models[1].is_default);
        assert_eq!(models[1].window, Some(200_000));
        assert_eq!(curated_twin("claude", "haiku"), Some("claude-haiku-4-5"));
        assert_eq!(curated_twin("claude", "claude-sonnet-4-6"), None);
        assert_eq!(curated_twin("codex", "gpt"), None);
    }

    #[test]
    fn access_shows_ask_first_until_the_agent_reports_one() {
        let claude = Permission::choices("claude");
        assert_eq!(effective_access(None, claude), Some(Permission::Ask));
        assert_eq!(
            effective_access(Some(Permission::Plan), claude),
            Some(Permission::Plan)
        );
        assert_eq!(effective_access(None, &[]), None);
    }
}
