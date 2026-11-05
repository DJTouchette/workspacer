//! R5.1 NavItem: a destination in the sidebar. Icon + label in a row
//! ([`NavItem::render`]), or an icon tile whose label is its tooltip
//! ([`NavItem::render_tile`]). The count sits in its own slot (row) or on
//! the icon's corner (tile), so a count appearing never moves the icon.
use super::super::chrome::interactive_control;
use super::super::primitives::{CountTone, count_badge, count_text, tooltip_view_keys};
use super::*;

/// A sidebar destination.
pub(crate) struct NavItem {
    id: SharedString,
    icon: Icon,
    label: SharedString,
    selected: bool,
    count: Option<(usize, CountTone)>,
    shortcut: Option<SharedString>,
}

impl NavItem {
    /// `id` is the element id and debug selector (keep `nav-*`).
    pub(crate) fn new(
        id: impl Into<SharedString>,
        icon: impl Into<Icon>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            label: label.into(),
            selected: false,
            count: None,
            shortcut: None,
        }
    }

    /// The destination is on screen.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A count badge (nothing when `n` is 0). `CountTone::NeedsYou` only
    /// for what waits on this user now.
    pub(crate) fn count(mut self, n: usize, tone: CountTone) -> Self {
        self.count = (n > 0).then_some((n, tone));
        self
    }

    /// The keys that open it ("g p"), shown in the tooltip.
    pub(crate) fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    fn colors(&self, p: &Palette) -> (u32, u32) {
        if self.selected {
            (p.text, p.text)
        } else {
            (p.roles.text_secondary, p.roles.text_secondary)
        }
    }

    /// The row: icon slot · label · count slot. Add `.on_click`.
    pub(crate) fn render(self, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let sys = t.system();
        let (fg, icon_fg) = self.colors(p);
        let (hover, pressed, text) = (p.roles.hover, p.roles.pressed, p.text);
        let NavItem {
            id,
            icon,
            label,
            selected,
            count,
            shortcut,
        } = self;
        let selector = id.clone();
        interactive_control(div().id(id), *p, true)
            .debug_selector(move || selector.to_string())
            .w_full()
            .h(px(sys.nav_row))
            .px(px(t.space.md))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .rounded(px(t.radius.control))
            .text_size(px(t.text.body))
            .font_weight(FontWeight(t.weight.medium))
            .text_color(rgb(fg))
            .when(selected, |d| d.bg(rgb(p.selected)))
            .when(!selected, |d| {
                d.hover_text_style(move |s| s.bg(rgb(hover)).text_color(rgb(text)))
                    .active_text_style(move |s| s.bg(rgb(pressed)))
            })
            .when_some(shortcut.clone(), |d, keys| {
                d.tooltip(tooltip_view_keys(label.clone(), Some(keys), p))
            })
            .child(
                div()
                    .size(px(sys.nav_icon_slot))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(icon_fg))
                    .child(icon.size(px(t.icon.md))),
            )
            .child(div().flex_1().min_w_0().truncate().child(label.clone()))
            .children(count.map(|(n, tone)| count_badge(count_text(n), tone, p)))
    }

    /// The icon tile: the label and shortcut in its tooltip, the count on
    /// the icon's top-right corner (absolute, so the grid never moves).
    pub(crate) fn render_tile(self, p: &Palette) -> Stateful<Div> {
        let t = p.tokens;
        let sys = t.system();
        let (fg, _) = self.colors(p);
        let (hover, pressed, text) = (p.roles.hover, p.roles.pressed, p.text);
        let NavItem {
            id,
            icon,
            label,
            selected,
            count,
            shortcut,
        } = self;
        let selector = id.clone();
        interactive_control(div().id(id), *p, true)
            .debug_selector(move || selector.to_string())
            .relative()
            .size(px(sys.nav_tile))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(t.radius.control))
            .text_color(rgb(fg))
            .when(selected, |d| d.bg(rgb(p.selected)))
            .when(!selected, |d| {
                d.hover_text_style(move |s| s.bg(rgb(hover)).text_color(rgb(text)))
                    .active_text_style(move |s| s.bg(rgb(pressed)))
            })
            .tooltip(tooltip_view_keys(label.clone(), shortcut.clone(), p))
            .child(icon.size(px(t.icon.md)))
            .children(count.map(|(n, tone)| {
                div()
                    .absolute()
                    .top(px(-t.space.xs))
                    .right(px(-t.space.xs))
                    .child(count_badge(count_text(n), tone, p))
            }))
    }
}
