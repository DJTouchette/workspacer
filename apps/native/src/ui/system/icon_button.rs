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
    dot: Option<(&'static str, Option<u32>)>,
    badge: Option<AnyElement>,
    icon_size: Option<f32>,
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
            dot: None,
            badge: None,
            icon_size: None,
        }
    }

    /// Its keys ("Ctrl K", "g n"; spaces separate keycaps).
    pub(crate) fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    /// [`Self::shortcut`] when there is one.
    pub(crate) fn shortcut_opt(mut self, keys: Option<&'static str>) -> Self {
        self.shortcut = keys.map(SharedString::from);
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

    /// A quiet dot on the icon's top-right corner: something new behind it
    /// (Settings: an update is ready). `id` is its debug selector. Accent by
    /// default; [`Self::dot_color`] gives it a status role instead.
    /// Absolute, so the icon never moves.
    pub(crate) fn dot(mut self, id: Option<&'static str>) -> Self {
        self.dot = id.map(|id| (id, None));
        self
    }

    /// [`Self::dot`] in a palette role (`roles.status.working` for a shell
    /// that runs).
    pub(crate) fn dot_color(mut self, id: Option<&'static str>, color: u32) -> Self {
        self.dot = id.map(|id| (id, Some(color)));
        self
    }

    /// A trailing fact beside the icon (a count badge, "Branch +3 −1"): the
    /// button widens to hold it and keeps its height.
    pub(crate) fn badge(mut self, badge: Option<impl IntoElement>) -> Self {
        self.badge = badge.map(IntoElement::into_any_element);
        self
    }

    /// The header strip's tools draw their icon at `icon.md` (16).
    pub(crate) fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = Some(size);
        self
    }

    pub(crate) fn render(self, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let id = self.id.clone();
        let badged = self.badge.is_some();
        let control = interactive_control(div().id(self.id), *p, self.enabled)
            .debug_selector(move || id.to_string())
            .h(px(t.layout.icon_button))
            .min_w(px(t.layout.icon_button))
            .when(badged, |d| d.px(px(t.space.sm)).gap(px(t.space.xs)))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .rounded(px(t.radius.control))
            .text_size(px(t.text.meta))
            .font_weight(FontWeight(t.weight.medium))
            .tooltip(tooltip_view_keys(self.label, self.shortcut, p))
            .child(self.icon.size(px(self.icon_size.unwrap_or(t.icon.sm))))
            .children(self.badge)
            .when_some(self.dot, |d, (dot, color)| {
                d.relative().child(
                    div()
                        .debug_selector(move || dot.into())
                        .absolute()
                        .top(px(t.space.xxs))
                        .right(px(t.space.xxs))
                        .size(px(t.layout.status_dot))
                        .rounded(px(t.radius.full))
                        .bg(rgb(color.unwrap_or(p.accent))),
                )
            });
        let control = button_states(control, self.variant, self.enabled, p);
        if self.selected {
            control.bg(rgb(p.selected)).text_color(rgb(p.text))
        } else {
            control
        }
    }
}
