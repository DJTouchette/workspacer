//! The Hosts page: every host's resources, slots, gate and work queue, this
//! hub first and then its federation peers (`docs/host-work-queue.md`). The
//! owner cancels queued work here and tunes this hub's limits. The model,
//! parsing and live overlay live in `wks_native::hosts`.
//!
//! While the page is open the controller watches `host.*` and `queue.*`
//! (see `Command::WatchHosts`) and the page re-reads every 10s as a
//! fallback for events a peer did not forward.
use super::*;
use wks_native::features::{HostAction, Request};
use wks_native::hosts::{self, Card, Item, Limits, Resources, Snapshot, State, Support};
use wks_native::timing::now_ms;

pub(super) const HOSTS_DESCRIPTION: &str =
    "What each machine has free, and the work waiting for it.";
const UNSUPPORTED_TITLE: &str = "Not supported by this hub";
const UNSUPPORTED: &str =
    "This hub predates the work queue. Update it to see host resources and queued work here.";
const UNAVAILABLE_TITLE: &str = "Couldn’t load hosts";
const UNAVAILABLE: &str = "The hub didn’t answer for its hosts. The request may have failed.";
const POLL: std::time::Duration = std::time::Duration::from_secs(10);
/// The gate's defaults (`queue.maxCpuPressure` / `maxMemoryPressure`), for
/// peers whose limits this client cannot read.
const DEFAULT_CPU_LIMIT: f64 = 60.;
const DEFAULT_MEMORY_LIMIT: f64 = 25.;

impl Workspace {
    /// Watch host and queue events, read now, and keep re-reading while
    /// the page stays open.
    pub(super) fn open_hosts(&mut self, cx: &mut Context<Self>) {
        self.command(Command::WatchHosts(true), cx);
        self.request(Request::Hosts, cx);
        if self.extras.hosts_poll.is_some() {
            return;
        }
        self.extras.hosts_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let open = this
                    .update(cx, |this, cx| {
                        let open = this.screen == Screen::Hosts && !this.new_session;
                        if !open {
                            this.extras.hosts_poll = None;
                            this.command(Command::WatchHosts(false), cx);
                        } else if this.view.connected
                            && !this.view.requests.get("hosts").is_some_and(|s| s.loading)
                        {
                            // Not `request`: that clears the page's notice
                            // (a launch's confirmation).
                            this.command(Command::Request(Request::Hosts), cx);
                        }
                        open
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        }));
    }

    /// The newest snapshot the hub returned, from a read or a write.
    pub(super) fn host_snapshot(&self) -> Option<Snapshot> {
        ["hosts", "host-action"]
            .iter()
            .filter_map(|key| self.view.requests.get(*key))
            .filter(|s| !s.loading && s.error.is_none() && s.value.is_object())
            .max_by_key(|s| s.number)
            .map(|s| Snapshot::parse(&s.value))
    }

    /// Where a launch could run, for the New Agent host picker: the latest
    /// `queue.targets` read, else the Hosts snapshot's. `None` when this hub
    /// has not answered one (older hubs), which hides the picker.
    pub(super) fn launch_targets(&self) -> Option<hosts::Targets> {
        let read = self
            .view
            .requests
            .get("queue-targets")
            .filter(|s| !s.loading && s.error.is_none())
            .and_then(|s| hosts::parse_targets(&s.value));
        read.or_else(|| self.host_snapshot().and_then(|s| s.targets.ok()))
    }

    /// Settle the limits editor and a launch sent through the queue.
    pub(super) fn sync_hosts(&mut self, next: &View, window: &mut Window, cx: &mut Context<Self>) {
        if self.extras.limits_sent
            && let Some(state) = next.requests.get("host-action")
            && !state.loading
            && state.number > self.extras.limits_after
            && matches!(state.request, Request::HostAction(HostAction::SetConfig(_)))
        {
            self.extras.limits_sent = false;
            match &state.error {
                Some(error) => self.extras.limits_error = error.clone(),
                None => {
                    self.extras.limits_open = false;
                    self.extras.limits_error.clear();
                }
            }
        }
        if self.extras.queue_sent
            && let Some(state) = next.requests.get("queue-submit")
            && !state.loading
            && state.number > self.extras.queue_receipt
        {
            self.extras.queue_receipt = state.number;
            self.extras.queue_sent = false;
            match &state.error {
                Some(error) => self.spawn_error = error.clone(),
                None => {
                    let line = hosts::parse_submitted(&state.value)
                        .map(|(host, item)| hosts::submitted_line(&host, &item))
                        .unwrap_or_else(|| "Submitted to the queue.".into());
                    self.prompt
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    self.spawn_error.clear();
                    self.open_feature(Screen::Hosts, window, cx);
                    self.extras.notice = line;
                }
            }
        }
    }

    pub(super) fn render_hosts(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let page = div().flex().flex_col().gap_3();
        let read_error = self
            .view
            .requests
            .get("hosts")
            .filter(|s| !s.loading)
            .and_then(|s| s.error.clone())
            .filter(|e| !e.is_empty());
        let Some(snapshot) = self.host_snapshot() else {
            return page.child(match read_error {
                Some(error) => super::jobs::jobs_state(
                    IconName::TriangleAlert,
                    UNAVAILABLE_TITLE,
                    UNAVAILABLE,
                    p.warning,
                    p,
                )
                .debug_selector(|| "hosts-unavailable".into())
                .child(
                    div()
                        .max_w(px(420.))
                        .text_center()
                        .font_family(mono_font())
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(error),
                ),
                None => div().py_2().child(chrome::notice_line(
                    "Loading hosts…",
                    chrome::Tone::Loading,
                    p,
                )),
            });
        };
        if snapshot.support() == Support::Unsupported {
            return page.child(
                super::jobs::jobs_state(IconName::Info, UNSUPPORTED_TITLE, UNSUPPORTED, p.muted, p)
                    .debug_selector(|| "hosts-unsupported".into()),
            );
        }
        // A failed cancel says why above the cards; a refused limits change
        // says so in the editor.
        let cancel_error = self
            .view
            .requests
            .get("host-action")
            .filter(|s| matches!(s.request, Request::HostAction(HostAction::Cancel { .. })))
            .filter(|s| !s.loading)
            .and_then(|s| s.error.clone());
        let limits = snapshot.limits.as_ref().ok().cloned();
        let cards = hosts::cards(&snapshot, &self.view.hosts_live);
        let mut page = page
            .when_some(read_error, |d, e| {
                d.child(chrome::notice_line(e, chrome::Tone::Error, p))
            })
            .when_some(cancel_error, |d, e| {
                d.child(
                    chrome::notice_line(e, chrome::Tone::Error, p)
                        .debug_selector(|| "hosts-cancel-error".into()),
                )
            });
        for (ix, card) in cards.iter().enumerate() {
            page = page.child(self.render_host_card(ix, card, limits.as_ref(), cx));
        }
        if cards.len() == 1 {
            page = page.child(
                div()
                    .pt_1()
                    .text_size(px(chrome::scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child("No federation peers. Peers this hub connects to show here with their own queues."),
            );
        }
        page
    }

    fn render_host_card(
        &self,
        ix: usize,
        card: &Card,
        limits: Option<&Limits>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let now = now_ms();
        let gate_color = match card.resources.as_ref().and_then(|r| r.gate.as_ref()) {
            Some(g) if g.open => p.success,
            Some(_) => p.warning,
            None => p.muted,
        };
        let offline = card.connected == Some(false);
        let mut badges: Vec<(String, u32)> = Vec::new();
        if !card.local {
            match card.connected {
                Some(true) => badges.push(("connected".into(), p.success)),
                Some(false) => badges.push(("offline".into(), p.error)),
                None => {}
            }
            match card.dispatch {
                Some(true) => badges.push(("dispatch on".into(), p.accent)),
                Some(false) => badges.push(("dispatch off".into(), p.muted)),
                None => {}
            }
        }
        let gate_label = card
            .resources
            .as_ref()
            .map_or("No sample", Resources::gate_label);
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(rgb(if offline { p.muted } else { gate_color })),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(chrome::scale::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(card.name.clone()),
            )
            .children(badges.into_iter().map(|(text, color)| pill(text, color, p)))
            .child(div().flex_1())
            .child(
                pill(gate_label.to_owned(), gate_color, p)
                    .debug_selector(move || format!("host-gate-{ix}")),
            )
            .when(card.local, |d| {
                d.child(
                    self.quiet_button(
                        "host-limits",
                        "Limits",
                        IconName::Settings2,
                        self.view.connected && limits.is_some(),
                    )
                    .debug_selector(|| "host-limits".into())
                    .when(self.view.connected && limits.is_some(), |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.toggle_limits(window, cx)),
                        )
                    }),
                )
            });
        let mut body = chrome::card(p)
            .debug_selector(move || format!("host-card-{ix}"))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(header);
        match &card.resources {
            Some(r) => {
                let limit = |key: &str, default: f64| {
                    if card.local {
                        limits.and_then(|l| l.get(key)).unwrap_or(default)
                    } else {
                        default
                    }
                };
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_4()
                            .child(meter(
                                "CPU",
                                r.cpu_pressure,
                                limit("maxCpuPressure", DEFAULT_CPU_LIMIT),
                                format!(
                                    "{} · PSI {}",
                                    hosts::pct(r.cpu_pct),
                                    hosts::psi(r.cpu_pressure)
                                ),
                                p,
                            ))
                            .child(meter(
                                "Memory",
                                r.memory_pressure,
                                limit("maxMemoryPressure", DEFAULT_MEMORY_LIMIT),
                                format!(
                                    "PSI {} · {}",
                                    hosts::psi(r.memory_pressure),
                                    r.memory_free()
                                ),
                                p,
                            ))
                            .child(meter(
                                "IO",
                                r.io_pressure,
                                100.,
                                format!("PSI {}", hosts::psi(r.io_pressure)),
                                p,
                            )),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("host-facts-{ix}"))
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(
                                [
                                    r.agent_line(),
                                    r.shell_line(),
                                    r.queued_line(),
                                    format!("disk {}", r.disk_free()),
                                    r.sampled_at
                                        .map(|at| {
                                            format!("sampled {}", wks_native::jobs::since(at, now))
                                        })
                                        .unwrap_or_default(),
                                ]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · "),
                            ),
                    )
                    .children(r.gate_reasons().iter().map(|reason| {
                        chrome::notice_line(reason.clone(), chrome::Tone::Warning, p)
                    }));
            }
            None => {
                body = body.child(
                    div()
                        .text_size(px(chrome::scale::CAPTION))
                        .text_color(rgb(p.muted))
                        .child(if offline {
                            "Offline: no recent resource sample."
                        } else {
                            "No resource sample yet."
                        }),
                );
            }
        }
        if card.local && self.extras.limits_open {
            body = body.child(self.render_limits(limits, cx));
        }
        if !card.error.is_empty() {
            body = body.child(chrome::notice_line(
                card.error.clone(),
                chrome::Tone::Error,
                p,
            ));
        }
        let queue = if card.items.is_empty() {
            div()
                .text_size(px(chrome::scale::CAPTION))
                .text_color(rgb(p.muted))
                .child("Nothing queued.")
        } else {
            let mut list = div()
                .flex()
                .flex_col()
                .rounded(px(p.control_radius))
                .border_1()
                .border_color(rgb(p.border))
                .overflow_hidden();
            for (row, item) in card.items.iter().enumerate() {
                list = list.child(self.render_queue_row(ix, row, &card.host, item, now, cx));
            }
            list
        };
        body.child(super::jobs::group_label("Queue", p.muted))
            .child(queue)
    }

    fn render_queue_row(
        &self,
        card: usize,
        row: usize,
        host: &str,
        item: &Item,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let color = state_color(&item.state, p);
        let busy = self
            .view
            .requests
            .get("host-action")
            .is_some_and(|s| s.loading);
        let can_cancel = item.cancellable() && self.view.connected && !busy;
        let note = item.note();
        let (id, host) = (item.id.clone(), host.to_owned());
        div()
            .debug_selector(move || format!("queue-row-{card}-{row}"))
            .when(row > 0, |d| d.border_t_1().border_color(rgb(p.border)))
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .child(pill(item.state.label().to_owned(), color, p))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(chrome::scale::META))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(item.title()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(chrome::scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child(format!("{} · {}", item.tags(), item.when(now))),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font())
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(item.what()),
                    )
                    .when(!note.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(if item.state == State::Failed {
                                    p.error
                                } else {
                                    p.muted
                                }))
                                .child(note),
                        )
                    }),
            )
            .when(item.cancellable(), |d| {
                d.child(
                    self.quiet_button(
                        SharedString::from(format!("queue-cancel-{card}-{row}")),
                        "Cancel",
                        IconName::Close,
                        can_cancel,
                    )
                    .debug_selector(move || format!("queue-cancel-{card}-{row}"))
                    .when(can_cancel, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.request(
                                Request::HostAction(HostAction::Cancel {
                                    id: id.clone(),
                                    host: host.clone(),
                                }),
                                cx,
                            );
                        }))
                    }),
                )
            })
    }

    fn toggle_limits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.extras.limits_open = !self.extras.limits_open;
        self.extras.limits_error.clear();
        if self.extras.limits_open
            && let Some(limits) = self.host_snapshot().and_then(|s| s.limits.ok())
        {
            for (input, (key, ..)) in self.extras.limit_inputs.iter().zip(hosts::LIMITS) {
                let value = limits.text(key);
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        cx.notify();
    }

    fn save_limits(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.host_snapshot().and_then(|s| s.limits.ok()) else {
            return;
        };
        let fields: Vec<(&'static str, String)> = hosts::LIMITS
            .iter()
            .zip(&self.extras.limit_inputs)
            .map(|((key, ..), input)| (*key, input.read(cx).value().to_string()))
            .collect();
        match hosts::config_patch(&current, &fields) {
            Ok(Some(patch)) => {
                self.extras.limits_error.clear();
                self.extras.limits_sent = true;
                self.extras.limits_after = self
                    .view
                    .requests
                    .get("host-action")
                    .map_or(0, |s| s.number);
                self.request(Request::HostAction(HostAction::SetConfig(patch)), cx);
            }
            Ok(None) => {
                self.extras.limits_open = false;
                self.extras.limits_error.clear();
            }
            Err(error) => self.extras.limits_error = error,
        }
        cx.notify();
    }

    /// This hub's six `queue.*` limits; only the owner may change them, and
    /// the hub's refusal shows here.
    fn render_limits(&self, limits: Option<&Limits>, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let busy = self.extras.limits_sent;
        let mut grid = div().flex().flex_wrap().gap_3();
        for (ix, ((key, label, unit), input)) in hosts::LIMITS
            .iter()
            .zip(&self.extras.limit_inputs)
            .enumerate()
        {
            let effective = limits.map(|l| l.text(key)).unwrap_or_default();
            grid = grid.child(
                div()
                    .debug_selector(move || format!("limit-{ix}"))
                    .w(px(200.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(chrome::scale::CAPTION))
                            .text_color(rgb(p.muted))
                            .child(if unit.is_empty() {
                                (*label).to_owned()
                            } else {
                                format!("{label} ({unit})")
                            }),
                    )
                    .child(Input::new(input).disabled(busy))
                    .when(effective.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(chrome::scale::CAPTION))
                                .text_color(rgb(p.muted))
                                .child("Not reported"),
                        )
                    }),
            );
        }
        div()
            .debug_selector(|| "host-limits-editor".into())
            .pt_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(projects::section_label("Queue limits", p))
            .child(grid)
            .when(!self.extras.limits_error.is_empty(), |d| {
                d.child(
                    chrome::notice_line(self.extras.limits_error.clone(), chrome::Tone::Error, p)
                        .debug_selector(|| "host-limits-error".into()),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.primary_button(
                            "host-limits-save",
                            if busy { "Saving…" } else { "Save limits" },
                            !busy && self.view.connected,
                        )
                        .debug_selector(|| "host-limits-save".into())
                        .when(!busy && self.view.connected, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.save_limits(cx)))
                        }),
                    )
                    .child(
                        self.button("host-limits-close", "Close", !busy)
                            .when(!busy, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.extras.limits_open = false;
                                    this.extras.limits_error.clear();
                                    cx.notify();
                                }))
                            }),
                    ),
            )
    }
}

/// A labelled pressure meter: the fill is the share of the gate's limit
/// in use, toned like the context gauge (amber from 70%, red from 90%).
/// Unknown stays an empty, muted track, never a zero reading.
fn meter(label: &'static str, value: Option<f64>, limit: f64, text: String, p: Palette) -> Div {
    let share = value.map(|v| (v / limit.max(0.1) * 100.).clamp(0., 100.));
    let color = super::gauge::gauge_color(share, p);
    let fill = match share {
        Some(s) if s > 0. => (s.max(2.) / 100.) as f32,
        _ => 0.,
    };
    div()
        .w(px(200.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(px(chrome::scale::CAPTION))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(p.muted))
                        .child(label),
                )
                .child(
                    div()
                        .text_color(rgb(if value.is_some() { p.text } else { p.muted }))
                        .child(text),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(4.))
                .rounded_full()
                .bg(rgb(p.border))
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(rgb(color))
                        .w(gpui::relative(fill)),
                ),
        )
}

fn state_color(state: &State, p: Palette) -> u32 {
    match state {
        State::Queued => p.accent,
        State::Running => p.success,
        State::Done => p.muted,
        State::Failed => p.error,
        State::Cancelled => p.muted,
        State::Unknown | State::Other(_) => p.warning,
    }
}

/// A small state pill: its tone's text on a faint wash of the same tone.
fn pill(text: String, color: u32, p: Palette) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py(px(1.))
        .rounded_full()
        .bg(if color == p.muted {
            chrome::tonal(p, 0.07)
        } else {
            gpui::Hsla::from(rgb(color)).opacity(0.14)
        })
        .text_size(px(chrome::scale::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(text)
}
