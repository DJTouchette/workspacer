//! The terminal under design v2: a header in the panels' shape (status as
//! a StatusBadge, icon buttons with tooltips) and ANSI colours derived from
//! the theme, so the eight hues stay distinct (v2 retires purple from the
//! UI, but a terminal's magenta must stay magenta).
use super::*;
use crate::ui::primitives::{self, ButtonVariant, StatusBadge, StatusKind};
use gpui::Hsla;

fn to_u32(color: Hsla) -> u32 {
    let c = color.to_rgb();
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
}

/// `color`'s saturation and lightness at another hue (degrees).
fn at_hue(color: u32, degrees: f32) -> u32 {
    let mut c = Hsla::from(rgb(color));
    c.h = degrees / 360.;
    to_u32(c)
}

/// The bright variant: a step lighter on dark themes, darker on light.
fn bright(color: u32, light: bool) -> u32 {
    let mut c = Hsla::from(rgb(color));
    c.l = if light {
        (c.l - 0.08).max(0.)
    } else {
        (c.l + 0.08).min(1.)
    };
    to_u32(c)
}

/// The 16 ANSI colours of a v2 palette. Red, green, yellow and blue are
/// the theme's error, success, warning and accent; magenta and cyan turn
/// the accent's hue; black and white are its surfaces and text.
pub(super) fn ansi_v2(index: u8, p: Palette) -> u32 {
    let light = term::is_light(p.code_block);
    let base = match index & 7 {
        0 => {
            if light {
                p.text
            } else {
                p.roles.border_strong
            }
        }
        1 => p.error,
        2 => p.success,
        3 => p.warning,
        4 => p.accent,
        5 => at_hue(p.accent, 300.),
        6 => at_hue(p.accent, 185.),
        _ => p.prose,
    };
    match index & 15 {
        8 => p.roles.text_tertiary,
        15 => p.text,
        i if i > 8 => bright(base, light),
        _ => base,
    }
}

/// A terminal state in the shared status vocabulary.
fn status_of(status: Option<Status>) -> StatusBadge {
    match status {
        None => StatusBadge::new(StatusKind::Idle).label("No terminal"),
        Some(Status::Starting) => StatusBadge::new(StatusKind::Working).label("Starting…"),
        Some(Status::Attaching) => StatusBadge::new(StatusKind::Working).label("Connecting…"),
        Some(Status::Live) => StatusBadge::new(StatusKind::Working).label("Running"),
        Some(Status::Detached) => StatusBadge::new(StatusKind::Idle).label("In the background"),
        Some(Status::Exited) => StatusBadge::new(StatusKind::Idle).label("Exited"),
        Some(Status::Failed) => StatusBadge::new(StatusKind::Failed).label("Unavailable"),
    }
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn terminal_header_v2(
        &self,
        title: String,
        cwd: String,
        status: Option<Status>,
        view: Option<Entity<TerminalView>>,
        has_shell: bool,
        can_restart: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let tall = self.terminal.tall;
        div()
            .flex_shrink_0()
            .h(px(t.panels.header_height))
            .pl(px(t.space.lg))
            .pr(px(t.space.sm))
            .flex()
            .items_center()
            .gap(px(t.space.md))
            .border_b(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .child(
                Icon::new(IconName::SquareTerminal)
                    .size(px(t.icon.md))
                    .flex_shrink_0()
                    .text_color(rgb(p.roles.text_tertiary)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(t.text.subheading))
                    .font_weight(FontWeight(t.weight.semibold))
                    .text_color(rgb(p.text))
                    .child("Terminal"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(t.space.md))
                    .child(
                        primitives::truncated("terminal-title", title, &p)
                            .flex_shrink()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary)),
                    )
                    .child(
                        primitives::truncated("terminal-cwd", cwd, &p)
                            .flex_1()
                            .font_family(mono_font())
                            .text_size(px(t.text.caption))
                            .text_color(rgb(p.roles.text_tertiary)),
                    ),
            )
            .child(
                status_of(status)
                    .render(&p)
                    .debug_selector(|| "terminal-status".into()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.space.xxs))
                    .child(
                        primitives::icon_button(
                            "terminal-copy",
                            IconName::Copy,
                            "Copy screen text",
                            ButtonVariant::Tertiary,
                            view.is_some(),
                            &p,
                        )
                        .when_some(view, |d, view| {
                            d.on_click(move |_, _, cx| {
                                let text = view.read(cx).text();
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                            })
                        }),
                    )
                    .child(
                        primitives::icon_button(
                            "terminal-restart",
                            IconName::Redo,
                            "End this shell and start a new one",
                            ButtonVariant::Tertiary,
                            can_restart,
                            &p,
                        )
                        .debug_selector(|| "terminal-restart".into())
                        .when(can_restart, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.restart_terminal(window, cx)
                                }),
                            )
                        }),
                    )
                    .child(
                        primitives::icon_button(
                            "terminal-popout",
                            IconName::ExternalLink,
                            "Open in a separate window",
                            ButtonVariant::Tertiary,
                            has_shell,
                            &p,
                        )
                        .debug_selector(|| "terminal-popout".into())
                        .when(has_shell, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.pop_out_terminal(window, cx)
                                }),
                            )
                        }),
                    )
                    .child(
                        primitives::icon_button(
                            "terminal-size",
                            if tall {
                                IconName::Minimize
                            } else {
                                IconName::Maximize
                            },
                            if tall { "Shrink" } else { "Enlarge" },
                            ButtonVariant::Tertiary,
                            true,
                            &p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.terminal.tall = !this.terminal.tall;
                            cx.notify();
                        })),
                    )
                    .child(
                        primitives::icon_button_with_shortcut(
                            "terminal-close",
                            IconName::Close,
                            "Hide terminal (the shell keeps running)",
                            Some("Ctrl `"),
                            ButtonVariant::Tertiary,
                            true,
                            &p,
                        )
                        .debug_selector(|| "terminal-close".into())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.hide_terminal(window, cx)),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wks_native::appearance::{Appearance, Design};

    #[test]
    fn v2_ansi_keeps_eight_distinct_hues_in_every_theme() {
        for appearance in Appearance::ALL {
            let p = appearance.palette_for(Design::V2);
            let colors: Vec<u32> = (0..16).map(|i| ansi_v2(i, p)).collect();
            // Magenta is magenta, not the busy blue.
            assert_ne!(colors[5], p.busy, "{appearance:?}");
            assert_ne!(colors[5], colors[4], "{appearance:?}");
            assert_ne!(colors[6], colors[4], "{appearance:?}");
            // Bright variants differ from their base.
            for i in 1..7 {
                assert_ne!(colors[i], colors[i + 8], "{appearance:?} {i}");
            }
            assert_eq!(colors[1], p.error);
            assert_eq!(colors[15], p.text);
        }
    }
}
