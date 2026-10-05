//! Account usage: a one-bar-per-account strip in the sidebar, the full card on
//! hover, and a detail modal on click. Data comes from `usage.report` via
//! `wks_native::usage`.
use super::*;
use gpui::{AnyView, deferred};
use wks_native::usage::{self, Account, Window as UsageWindow};

fn color(window: &UsageWindow, p: Palette) -> u32 {
    if window.pct >= 90. {
        p.error
    } else if window.pct >= 70. || window.pace.as_deref() == Some("overspending") {
        p.warning
    } else {
        p.success
    }
}

fn long_label(window: &UsageWindow) -> &'static str {
    match window.label {
        "5h" => "5-hour",
        "Week" => "Weekly",
        _ => "Monthly",
    }
}

fn pace_line(window: &UsageWindow, p: Palette) -> Option<(String, u32)> {
    let (pace, expected) = (window.pace.as_deref()?, window.expected_pct?);
    let (verdict, tone) = match pace {
        "overspending" => ("Ahead of pace", p.warning),
        "ahead" => ("Slightly ahead of pace", p.muted),
        _ => ("On track", p.success),
    };
    Some((
        format!("{verdict} · expected {}% by now", expected.round() as u64),
        tone,
    ))
}

/// A rounded track with the fill and, when pacing is known, a tick where an
/// even pace would be now.
fn meter(window: &UsageWindow, height: f32, p: Palette) -> Div {
    let fill = if window.pct > 0. {
        (window.pct.clamp(2., 100.) / 100.) as f32
    } else {
        0.
    };
    div()
        .relative()
        .w_full()
        .h(px(height))
        .rounded_full()
        .bg(rgb(p.border))
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(color(window, p)))
                .w(gpui::relative(fill)),
        )
        .when_some(window.expected_pct, |d, expected| {
            d.child(
                div()
                    .absolute()
                    .top(px(-2.))
                    .left(gpui::relative((expected.clamp(0., 100.) / 100.) as f32))
                    .w(px(1.5))
                    .h(px(height + 4.))
                    .rounded_full()
                    .bg(rgb(p.text))
                    .opacity(0.6),
            )
        })
}

fn account_header(account: &Account, p: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(chrome::brand_badge(
            &account.provider,
            account.title.clone(),
            p,
            12.,
        ))
        .when(account.stale, |d| {
            d.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.disabled))
                    .child("stale"),
            )
        })
}

/// The full card: every account, every window, compact. Shown on hover.
fn usage_card(accounts: &[Account], error: Option<&str>, p: Palette) -> Div {
    div()
        .w(px(280.))
        .p_3()
        .rounded(px(p.control_radius))
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.surface))
        .shadow(chrome::floating_shadow(p))
        .text_color(rgb(p.text))
        .flex()
        .flex_col()
        .gap_3()
        .children(accounts.iter().map(|account| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(account_header(account, p))
                .when_some(account.unmeasured.as_ref(), |d, state| {
                    d.child(unmeasured_line(state, p, 10.))
                })
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .children(account.windows.iter().map(|w| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .text_size(px(10.))
                                        .child(div().text_color(rgb(p.muted)).child(w.label))
                                        .child(
                                            div()
                                                .text_color(rgb(color(w, p)))
                                                .child(format!("{}%", w.pct.round() as u64)),
                                        ),
                                )
                                .child(meter(w, 4., p))
                        })),
                )
        }))
        .when_some(error, |d, error| {
            d.child(unmeasured_line(
                &usage::Unmeasured {
                    state: if accounts.is_empty() {
                        "Usage unavailable"
                    } else {
                        "Last refresh failed"
                    },
                    reason: error.to_owned(),
                    error: true,
                },
                p,
                10.,
            ))
        })
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(p.disabled))
                .child("Click for details"),
        )
}

/// "Sign in again · oauth token expired": why an account has no reading.
fn unmeasured_line(state: &usage::Unmeasured, p: Palette, size: f32) -> Div {
    div()
        .text_size(px(size))
        .line_height(gpui::relative(1.4))
        .text_color(rgb(p.muted))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if state.error { p.warning } else { p.muted }))
                .child(state.state),
        )
        .child(state.reason.clone())
}

fn unmeasured_tone(state: &usage::Unmeasured, p: Palette) -> u32 {
    if state.error { p.warning } else { p.disabled }
}

struct UsageHover {
    accounts: Vec<Account>,
    error: Option<String>,
    palette: Palette,
}

impl Render for UsageHover {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl_2()
            .debug_selector(|| "usage-hover-card".into())
            .child(usage_card(
                &self.accounts,
                self.error.as_deref(),
                self.palette,
            ))
    }
}

impl Workspace {
    fn usage_accounts(&self) -> Option<(Vec<Account>, i64)> {
        let now = timing::now_ms() / 1000;
        let accounts = usage::accounts(self.view.usage.as_deref()?, now);
        (!accounts.is_empty()).then_some((accounts, now))
    }

    /// One line per account with its most pressing window, or the reason it
    /// has none. Hover shows every window; click opens the detail modal. A
    /// failed report read with nothing to show says so rather than vanishing.
    pub(super) fn render_usage_strip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let p = self.appearance.palette();
        let accounts = self.usage_accounts().map(|(a, _)| a).unwrap_or_default();
        let error = self.view.usage_error.clone();
        if accounts.is_empty() && error.is_none() {
            return None;
        }
        let hover = accounts.clone();
        let hover_error = error.clone();
        let row = || div().flex().items_center().gap_2().text_size(px(11.));
        Some(
            div()
                .id("sidebar-usage")
                .debug_selector(|| "sidebar-usage".into())
                .mx_2()
                .mb_1()
                .px_2()
                .py(px(6.))
                .rounded(px(p.control_radius))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(p.surface)))
                .flex()
                .flex_col()
                .gap(px(5.))
                .when(accounts.is_empty(), |d| {
                    d.child(
                        row()
                            .debug_selector(|| "sidebar-usage-error".into())
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .size(px(11.))
                                    .text_color(rgb(p.warning)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child("Usage unavailable"),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child("Retry"),
                            ),
                    )
                })
                .children(accounts.iter().enumerate().map(|(ix, account)| {
                    let name = div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .child(chrome::brand_badge(
                            &account.provider,
                            account.title.clone(),
                            p,
                            11.,
                        ));
                    match (account.primary(), &account.unmeasured) {
                        (Some(w), _) => row()
                            .child(name)
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .child(w.label),
                            )
                            .child(div().w(px(56.)).flex_shrink_0().child(meter(w, 4., p)))
                            .child(
                                div()
                                    .w(px(30.))
                                    .flex_shrink_0()
                                    .flex()
                                    .justify_end()
                                    .text_size(px(10.))
                                    .text_color(rgb(color(w, p)))
                                    .child(format!("{}%", w.pct.round() as u64)),
                            ),
                        (None, state) => row()
                            .debug_selector(move || format!("sidebar-usage-unmeasured-{ix}"))
                            .child(name)
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(state
                                        .as_ref()
                                        .map_or(p.disabled, |s| unmeasured_tone(s, p))))
                                    .child(state.as_ref().map_or("No reading", |s| s.state)),
                            ),
                    }
                }))
                .tooltip(move |_, cx| {
                    AnyView::from(cx.new(|_| UsageHover {
                        accounts: hover.clone(),
                        error: hover_error.clone(),
                        palette: p,
                    }))
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.usage_open = true;
                    this.command(Command::RefreshUsage, cx);
                    cx.notify();
                })),
        )
    }

    /// Detail modal over the whole window. Backdrop click, ✕ and Esc close it.
    pub(super) fn render_usage_modal(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.usage_open {
            return None;
        }
        let p = self.appearance.palette();
        let (accounts, now) = self.usage_accounts().unwrap_or_default();
        let updated = self
            .view
            .usage
            .as_deref()
            .and_then(usage::evaluated_at)
            .map(|at| format!("Updated {}", usage::ago(at, now)));
        let card = div()
            .id("usage-modal")
            .debug_selector(|| "usage-modal".into())
            .occlude()
            .w(px(460.))
            .max_h(gpui::relative(0.8))
            .overflow_y_scroll()
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .shadow(chrome::floating_shadow(p))
            .text_color(rgb(p.text))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Usage"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.muted))
                                    .child(match &updated {
                                        Some(updated) => format!("Rate-limit windows per account · {updated}"),
                                        None => "Rate-limit windows per account".into(),
                                    }),
                            ),
                    )
                    .child(
                        self.icon_button("usage-refresh", "Refresh usage", IconName::Redo, true)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.command(Command::RefreshUsage, cx)
                            })),
                    )
                    .child(
                        self.icon_button("usage-close", "Close", IconName::Close, true)
                            .debug_selector(|| "usage-close".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.usage_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .children(accounts.iter().map(|account| {
                div()
                    .px_5()
                    .py_4()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(account_header(account, p))
                    .when_some(account.unmeasured.as_ref(), |d, state| {
                        d.child(unmeasured_line(state, p, 12.))
                    })
                    .when_some(account.failure.clone().filter(|_| account.unmeasured.is_none()), |d, failure| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.warning))
                                .child(format!("Last refresh failed: {failure}")),
                        )
                    })
                    .children(account.windows.iter().map(|w| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(long_label(w)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(color(w, p)))
                                            .child(format!("{}%", w.pct.round() as u64)),
                                    )
                                    .child(div().flex_1())
                                    .when_some(w.resets_at, |d, reset| {
                                        d.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(p.muted))
                                                .child(format!(
                                                    "resets in {}",
                                                    usage::resets_in(reset, now)
                                                )),
                                        )
                                    }),
                            )
                            .child(meter(w, 8., p))
                            .when_some(pace_line(w, p), |d, (line, tone)| {
                                d.child(div().text_size(px(11.)).text_color(rgb(tone)).child(line))
                            })
                    }))
            }))
            .when_some(self.view.usage_error.clone(), |d, error| {
                d.child(
                    div()
                        .debug_selector(|| "usage-modal-error".into())
                        .px_5()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .child(chrome::notice_line(
                            if accounts.is_empty() {
                                format!("Usage unavailable: {error}")
                            } else {
                                format!("Last refresh failed; showing the previous reading. {error}")
                            },
                            chrome::Tone::Warning,
                            p,
                            "usage-modal-error-icon",
                        )),
                )
            })
            .when(accounts.is_empty() && self.view.usage_error.is_none(), |d| {
                d.child(
                    div()
                        .px_5()
                        .py_6()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child("No account has a measured window right now."),
                )
            })
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(11.))
                    .text_color(rgb(p.disabled))
                    .child("Windows are account-wide: every session on a login shares them. The tick marks an even pace."),
            );
        Some(
            deferred(
                div()
                    .id("usage-backdrop")
                    .debug_selector(|| "usage-backdrop".into())
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(gpui::Hsla::from(rgb(p.shadow)).opacity(0.45))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.usage_open = false;
                            cx.notify();
                        }),
                    )
                    .child(card),
            )
            .with_priority(2),
        )
    }
}
