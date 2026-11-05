//! R5.1 IconButton: an icon-only control always names itself in a tooltip,
//! with its shortcut as keycaps when it has one.
use super::super::chrome::interactive_control;
use super::super::primitives::{ButtonVariant, button_states, tooltip_view_keys};
use super::*;

/// `IconButton::new("toolbar-changes", IconName::GitBranch, "Changes")
/// .shortcut("g d").render(&p).on_click(…)`. Tertiary by default.
pub(crate) struct IconButton {
    id: SharedString,
    icon: Icon,
    label: SharedString,
    shortcut: Option<SharedString>,
    variant: ButtonVariant,
    enabled: bool,
    selected: bool,
}

impl IconButton {
    /// `label` is required: it is the tooltip (and what a screen reader
    /// would say). `id` is the element id and debug selector.
    pub(crate) fn new(
        id: impl Into<SharedString>,
        icon: impl Into<Icon>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            label: label.into(),
            shortcut: None,
            variant: ButtonVariant::Tertiary,
            enabled: true,
            selected: false,
        }
    }

    /// Its keys ("Ctrl K", "g n"; spaces separate keycaps).
    pub(crate) fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// A tool that is open (its panel showing): the neutral selected fill.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let id = self.id.clone();
        let control = interactive_control(div().id(self.id), *p, self.enabled)
            .debug_selector(move || id.to_string())
            .size(px(t.layout.icon_button))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(px(t.radius.control))
            .tooltip(tooltip_view_keys(self.label, self.shortcut, p))
            .child(self.icon.size(px(t.icon.sm)));
        let control = button_states(control, self.variant, self.enabled, p);
        if self.selected {
            control.bg(rgb(p.selected)).text_color(rgb(p.text))
        } else {
            control
        }
    }
}
