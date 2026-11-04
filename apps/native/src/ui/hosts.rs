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
use wks_native::hosts::{
    self, Card, HostChoice, Item, Limits, Resources, Snapshot, State, Support,
};
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
        // Project links are not polled: the matrix asks every peer, and
        // `projects.linksChanged` re-reads it.
        self.command(Command::Request(Request::ProjectLinks), cx);
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
        // A targets read for the form's local project preselects that
        // project's folder on the chosen peer, unless the user picked one.
        if let Some(state) = next.requests.get("queue-targets")
            && !state.loading
            && state.number > self.extras.targets_seen
        {
            self.extras.targets_seen = state.number;
            if state.error.is_none()
                && !self.extras.launch_host_cwd_picked
                && let HostChoice::Peer(peer) = &self.extras.launch_host
                && let Request::QueueTargets { cwd: Some(cwd), .. } = &state.request
                && !cwd.is_empty()
                && *cwd == self.projects.cwd
                && let Some(resolved) = hosts::parse_targets(&state.value)
                    .and_then(|t| t.find(peer).and_then(|t| t.resolved_cwd.clone()))
            {
                self.extras.launch_host_cwd = resolved;
            }
        }
        // "Link & use" landed: use the folder, and re-read the targets so
        // the peer reports it as linked.
        if let Some((after, peer, path)) = self.extras.link_use.clone()
            && let Some(state) = next.requests.get("link-action")
            && !state.loading
            && state.number > after
        {
            self.extras.link_use = None;
            match &state.error {
                Some(error) => self.spawn_error = error.clone(),
                None => {
                    if self.extras.launch_host == HostChoice::Peer(peer) {
                        self.extras.launch_host_cwd = path;
                        self.extras.launch_host_cwd_picked = true;
                    }
                    self.spawn_error.clear();
                    self.refresh_launch_targets(cx);
                }
            }
        }
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
                    let best = matches!(&state.request,
                        Request::QueueSubmit(params) if params["host"] == "any");
                    let line = hosts::parse_submitted(&state.value)
                        .map(|(host, item)| hosts::submitted_line(&host, &item, best))
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
        if self.design().is_v2() {
            return self.render_hosts_v2(cx);
        }
        let p = self.palette();
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
        page.child(self.render_project_links(cx))
    }

    fn render_host_card(
        &self,
        ix: usize,
        card: &Card,
        limits: Option<&Limits>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
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
                    self.pquiet(
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
        let p = self.palette();
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
                    self.pquiet(
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
        let p = self.palette();
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
                        self.pprimary(
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
                        self.pbutton("host-limits-close", "Close", !busy)
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
pub(super) fn pill(text: String, color: u32, p: Palette) -> Div {
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

// area-pages: the Hosts page under design v2. One summary line per machine
// ("studio · Accepting work · 3 of 4 agent slots free"), a fixed status
// column, and the metrics and queue behind Details. Plain words replace the
// ops terms: the gate is "Accepting work" / "Paused: low memory", PSI lives
// in tooltips, and queue and session ids sit behind "Copy ID".
use super::primitives::{self, ButtonSize, ButtonVariant, StatusBadge, StatusKind};

impl Workspace {
    fn render_hosts_v2(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let page = div().flex().flex_col().gap(px(t.space.lg));
        let read_error = self
            .view
            .requests
            .get("hosts")
            .filter(|s| !s.loading)
            .and_then(|s| s.error.clone())
            .filter(|e| !e.is_empty());
        let Some(snapshot) = self.host_snapshot() else {
            return page.child(match read_error {
                Some(error) => primitives::empty_state(
                    IconName::TriangleAlert,
                    UNAVAILABLE_TITLE,
                    Some(UNAVAILABLE.into()),
                    &p,
                )
                .debug_selector(|| "hosts-unavailable".into())
                .child(
                    primitives::truncated("hosts-unavailable-detail", error, &p)
                        .max_w(px(t.layout.dialog_width))
                        .font_family(mono_font())
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary)),
                ),
                None => chrome::notice_line("Loading hosts…", chrome::Tone::Loading, p),
            });
        };
        if snapshot.support() == Support::Unsupported {
            return page.child(
                primitives::empty_state(
                    IconName::Info,
                    UNSUPPORTED_TITLE,
                    Some(UNSUPPORTED.into()),
                    &p,
                )
                .debug_selector(|| "hosts-unsupported".into()),
            );
        }
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
            })
            .child(primitives::section_label("Machines", &p));
        for (ix, card) in cards.iter().enumerate() {
            page = page.child(self.render_host_v2(ix, card, limits.as_ref(), cx));
        }
        if cards.len() == 1 {
            page = page.child(
                div()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child("No other machines yet. Machines this hub connects to show here with their own queues."),
            );
        }
        page.child(
            div()
                .pt(px(t.space.xl))
                .child(self.render_project_links(cx)),
        )
    }

    /// One machine: the summary row, then Details (metrics, limits, queue)
    /// when opened.
    fn render_host_v2(
        &self,
        ix: usize,
        card: &Card,
        limits: Option<&Limits>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let now = now_ms();
        let open = self.extras.pages.hosts_open.contains(&card.host);
        let status = host_status(card);
        // "This machine · studio" reads as "studio", then "This machine".
        let name = card
            .name
            .strip_prefix("This machine · ")
            .unwrap_or(&card.name)
            .to_owned();
        let this_machine = card.local && name != "This machine";
        let toggle = card.host.clone();
        let details = primitives::button(
            SharedString::from(format!("host-details-{ix}")),
            "Details",
            Some(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }),
            ButtonVariant::Ghost,
            ButtonSize::Small,
            true,
            &p,
        )
        .debug_selector(move || format!("host-details-{ix}"))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            let set = &mut this.extras.pages.hosts_open;
            if !set.remove(&toggle) {
                set.insert(toggle.clone());
            }
            cx.notify();
        }));
        let gate_tooltip = card
            .resources
            .as_ref()
            .map(|r| {
                if r.gate_reasons().is_empty() {
                    "Starts queued work when a slot is free".to_owned()
                } else {
                    r.gate_reasons()
                        .iter()
                        .map(|r| plain_reason_sentence(r))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            })
            .unwrap_or_else(|| "No resource sample yet".to_owned());
        let summary = div()
            .id(("host-summary", ix))
            .debug_selector(move || format!("host-summary-{ix}"))
            .px(px(t.space.xl))
            .py(px(t.space.lg))
            .flex()
            .items_center()
            .gap(px(t.space.lg))
            .min_w_0()
            .child(primitives::avatar_tile_sized(&name, t.layout.avatar_lg, &p))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(t.space.md))
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(t.text.subheading))
                                    .font_weight(FontWeight(t.weight.semibold))
                                    .text_color(rgb(p.text))
                                    .child(name.clone()),
                            )
                            .when(this_machine, |d| {
                                d.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(t.text.meta))
                                        .text_color(rgb(p.roles.text_tertiary))
                                        .child("This machine"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("host-facts-{ix}"))
                            .min_w_0()
                            .truncate()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_secondary))
                            .child(host_summary(card)),
                    ),
            )
            .child(
                div()
                    .id(("host-gate", ix))
                    .debug_selector(move || format!("host-gate-{ix}"))
                    .w(px(t.layout.status_column + t.space.xxxl))
                    .flex_shrink_0()
                    .flex()
                    .tooltip(primitives::tooltip_view(gate_tooltip, None, &p))
                    .child(status.render(&p)),
            )
            .child(details);
        let mut body = primitives::card(&p)
            .p(px(0.))
            .gap(px(0.))
            .overflow_hidden()
            .debug_selector(move || format!("host-card-{ix}"))
            .child(summary);
        if !open {
            return body;
        }
        let mut detail = div()
            .debug_selector(move || format!("host-detail-{ix}"))
            .border_t(px(t.stroke.hairline))
            .border_color(rgb(p.border))
            .px(px(t.space.xl))
            .pt(px(t.space.lg))
            .pb(px(t.space.xl))
            .flex()
            .flex_col()
            .gap(px(t.space.lg));
        match &card.resources {
            Some(r) => {
                let limit = |key: &str, default: f64| {
                    if card.local {
                        limits.and_then(|l| l.get(key)).unwrap_or(default)
                    } else {
                        default
                    }
                };
                let cpu_limit = limit("maxCpuPressure", DEFAULT_CPU_LIMIT);
                let memory_limit = limit("maxMemoryPressure", DEFAULT_MEMORY_LIMIT);
                detail = detail
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .gap(px(t.space.xxl))
                            .child(metric_v2(
                                (ix, 0),
                                "CPU",
                                hosts::pct(r.cpu_pct),
                                r.cpu_pressure,
                                cpu_limit,
                                &p,
                            ))
                            .child(metric_v2(
                                (ix, 1),
                                "Memory",
                                format!("{} free", hosts::megabytes(r.memory_available_mb)),
                                r.memory_pressure,
                                memory_limit,
                                &p,
                            ))
                            .child(metric_v2(
                                (ix, 2),
                                "Disk activity",
                                load_word(r.io_pressure, 100.).to_owned(),
                                r.io_pressure,
                                100.,
                                &p,
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(t.text.meta))
                            .text_color(rgb(p.roles.text_tertiary))
                            .child(
                                [
                                    r.agent_line(),
                                    r.shell_line(),
                                    r.queued_line(),
                                    format!("{} disk", r.disk_free()),
                                    r.sampled_at
                                        .map(|at| {
                                            format!("updated {}", wks_native::jobs::since(at, now))
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
                        chrome::notice_line(plain_reason_sentence(reason), chrome::Tone::Warning, p)
                    }));
            }
            None => {
                detail = detail.child(
                    div()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child(if card.connected == Some(false) {
                            "Offline: no recent resource sample."
                        } else {
                            "No resource sample yet."
                        }),
                );
            }
        }
        if card.local {
            let can_edit = self.view.connected && limits.is_some();
            detail = detail.child(
                div().flex().child(
                    primitives::button(
                        "host-limits",
                        "Queue limits",
                        Some(IconName::Settings),
                        ButtonVariant::Secondary,
                        ButtonSize::Small,
                        can_edit,
                        &p,
                    )
                    .debug_selector(|| "host-limits".into())
                    .when(can_edit, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.toggle_limits(window, cx)),
                        )
                    }),
                ),
            );
            if self.extras.limits_open {
                detail = detail.child(self.render_limits(limits, cx));
            }
        }
        if !card.error.is_empty() {
            detail = detail.child(chrome::notice_line(
                card.error.clone(),
                chrome::Tone::Error,
                p,
            ));
        }
        detail = detail.child(primitives::section_label("Queue", &p).pt(px(t.space.xs)));
        detail = if card.items.is_empty() {
            detail.child(
                div()
                    .text_size(px(t.text.meta))
                    .text_color(rgb(p.roles.text_tertiary))
                    .child("Nothing queued."),
            )
        } else {
            let mut list = div()
                .flex()
                .flex_col()
                .rounded(px(t.radius.control))
                .border(px(t.stroke.hairline))
                .border_color(rgb(p.border))
                .overflow_hidden();
            for (row, item) in card.items.iter().enumerate() {
                list = list.child(self.render_queue_row_v2(ix, row, &card.host, item, now, cx));
            }
            detail.child(list)
        };
        body = body.child(detail);
        body
    }

    /// One queued, running or finished item: a fixed status column, then
    /// the title, where it came from and when, what it runs and why it
    /// waits; Cancel for queued work and its ids behind the overflow.
    fn render_queue_row_v2(
        &self,
        card: usize,
        row: usize,
        host: &str,
        item: &Item,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let t = p.tokens;
        let busy = self
            .view
            .requests
            .get("host-action")
            .is_some_and(|s| s.loading);
        let can_cancel = item.cancellable() && self.view.connected && !busy;
        let note = self.queue_note_v2(item);
        let (id, host) = (item.id.clone(), host.to_owned());
        let copy_id = item.id.clone();
        let mut menu = vec![
            primitives::menu_item(
                SharedString::from(format!("queue-copy-id-{card}-{row}")),
                "Copy ID",
                Some(IconName::Copy),
                false,
                true,
                &p,
            )
            .debug_selector(move || format!("queue-copy-id-{card}-{row}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                this.close_page_menu(cx);
            }))
            .into_any_element(),
        ];
        if !item.session_id.is_empty() {
            let session = item.session_id.clone();
            menu.push(
                primitives::menu_item(
                    SharedString::from(format!("queue-copy-session-{card}-{row}")),
                    "Copy session ID",
                    Some(IconName::Copy),
                    false,
                    true,
                    &p,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(session.clone()));
                    this.close_page_menu(cx);
                }))
                .into_any_element(),
            );
        }
        div()
            .debug_selector(move || format!("queue-row-{card}-{row}"))
            .when(row > 0, |d| {
                d.border_t(px(t.stroke.hairline))
                    .border_color(rgb(p.border))
            })
            .px(px(t.space.lg))
            .py(px(t.space.md))
            .flex()
            .items_center()
            .gap(px(t.space.lg))
            .min_w_0()
            .child(
                div()
                    .w(px(t.layout.status_column))
                    .flex_shrink_0()
                    .child(queue_status(&item.state).render(&p)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(t.space.xxs))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(t.space.md))
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(t.text.body))
                                    .font_weight(FontWeight(t.weight.medium))
                                    .text_color(rgb(p.text))
                                    .child(item.title()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(t.text.meta))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child(format!("{} · {}", item.tags(), item.when(now))),
                            ),
                    )
                    .child(
                        primitives::truncated(
                            SharedString::from(format!("queue-what-{card}-{row}")),
                            item.what(),
                            &p,
                        )
                        .font_family(mono_font())
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_secondary)),
                    )
                    .when(!note.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(t.text.meta))
                                .text_color(rgb(if item.state == State::Failed {
                                    p.error
                                } else {
                                    p.roles.text_tertiary
                                }))
                                .child(note),
                        )
                    }),
            )
            .when(item.cancellable(), |d| {
                d.child(
                    primitives::button(
                        SharedString::from(format!("queue-cancel-{card}-{row}")),
                        "Cancel",
                        None,
                        ButtonVariant::Ghost,
                        ButtonSize::Small,
                        can_cancel,
                        &p,
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
            .child(self.page_overflow(format!("queue-{card}-{row}"), menu, cx))
    }

    /// The line under a queue row, with a running session named by its
    /// title rather than its id.
    fn queue_note_v2(&self, item: &Item) -> String {
        if item.state == State::Running && !item.session_id.is_empty() {
            return match self.view.sessions.iter().find(|s| s.id == item.session_id) {
                Some(session) => format!("Session: {}", self.session_title(session)),
                None => "Running in a session".to_owned(),
            };
        }
        item.note()
    }
}

/// A machine's status in the shared vocabulary: "Accepting work",
/// "Paused: low memory", "Offline".
fn host_status(card: &Card) -> StatusBadge {
    if card.connected == Some(false) {
        return StatusBadge::new(StatusKind::Failed).label("Offline");
    }
    match card.resources.as_ref().and_then(|r| r.gate.as_ref()) {
        Some(gate) if gate.open => StatusBadge::new(StatusKind::Idle).label("Accepting work"),
        Some(gate) => StatusBadge::new(StatusKind::Paused).label(paused_label(&gate.reasons)),
        None => StatusBadge::new(StatusKind::Idle).label("No sample yet"),
    }
}

/// "Paused: low memory", from the gate's first reason.
pub(super) fn paused_label(reasons: &[String]) -> String {
    match reasons.first() {
        Some(reason) => format!("Paused: {}", plain_reason(reason)),
        None => "Paused".to_owned(),
    }
}

/// A gate reason ("memory pressure 34.0 >= 25.0") in two or three plain
/// words.
pub(super) fn plain_reason(reason: &str) -> &'static str {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("memory") {
        "low memory"
    } else if lower.contains("cpu") {
        "busy CPU"
    } else if lower.contains("io") {
        "busy disk"
    } else if lower.contains("disk") {
        "low disk space"
    } else if lower.contains("slot") {
        "no free slots"
    } else {
        "resources"
    }
}

/// A gate reason as a sentence for tooltips and the detail notice:
/// "Memory pressure is 34.0, over its limit of 25.0".
pub(super) fn plain_reason_sentence(reason: &str) -> String {
    let parts: Vec<&str> = reason.split(">=").map(str::trim).collect();
    if let [what, limit] = parts.as_slice()
        && let Some((name, value)) = what.rsplit_once(' ')
    {
        let mut name = name.to_owned();
        if let Some(first) = name.get(0..1) {
            name = first.to_uppercase() + &name[1..];
        }
        return format!("{name} is {value}, over its limit of {limit}");
    }
    reason.to_owned()
}

/// "Quiet" / "Busy" / "Saturated" for a pressure against its limit.
fn load_word(value: Option<f64>, limit: f64) -> &'static str {
    match value.map(|v| v / limit.max(0.1) * 100.) {
        None => "Unknown",
        Some(share) if share >= 90. => "Saturated",
        Some(share) if share >= 70. => "Busy",
        Some(_) => "Quiet",
    }
}

/// The machine's one-line summary: agent slots free and queued work.
fn host_summary(card: &Card) -> String {
    let mut parts = Vec::new();
    if !card.local {
        match card.dispatch {
            Some(true) => parts.push("Takes work from this hub".to_owned()),
            Some(false) => parts.push("Not taking work from this hub".to_owned()),
            None => {}
        }
    }
    match &card.resources {
        Some(r) => {
            if let (Some(free), Some(slots)) = (r.free_agents(), r.agent_slots) {
                parts.push(format!(
                    "{free} of {slots} agent slot{} free",
                    if slots == 1 { "" } else { "s" }
                ));
            }
            match r.queued {
                Some(0) | None => {}
                Some(n) => parts.push(format!("{n} queued")),
            }
        }
        None if card.connected == Some(false) => parts.push("Not connected".to_owned()),
        None => parts.push("Waiting for its first sample".to_owned()),
    }
    if parts.is_empty() {
        "Nothing queued".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// One metric column: label left, value right (never overlapping), the
/// meter of its pressure against the limit beneath, and the raw pressure
/// (PSI) in a tooltip.
fn metric_v2(
    id: (usize, usize),
    label: &'static str,
    value: String,
    pressure: Option<f64>,
    limit: f64,
    p: &Palette,
) -> Stateful<Div> {
    let t = p.tokens;
    let share = pressure.map(|v| (v / limit.max(0.1) * 100.).clamp(0., 100.));
    let intent = match share {
        Some(s) if s >= 90. => primitives::Intent::Danger,
        Some(s) if s >= 70. => primitives::Intent::Warning,
        Some(_) => primitives::Intent::Info,
        None => primitives::Intent::Neutral,
    };
    let tooltip = match pressure {
        Some(v) => format!(
            "Pressure (PSI) {} · pauses new work at {}",
            hosts::psi(Some(v)),
            hosts::psi(Some(limit))
        ),
        None => "Not reported".to_owned(),
    };
    div()
        .id(SharedString::from(format!("host-metric-{}-{}", id.0, id.1)))
        .debug_selector(move || format!("host-metric-{}-{}", id.0, id.1))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(t.space.sm))
        .tooltip(primitives::tooltip_view(tooltip, None, p))
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(t.space.md))
                .min_w_0()
                .text_size(px(t.text.meta))
                .child(
                    div()
                        .flex_shrink_0()
                        .font_weight(FontWeight(t.weight.medium))
                        .text_color(rgb(p.roles.text_secondary))
                        .child(label),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(if pressure.is_some() {
                            p.text
                        } else {
                            p.roles.text_tertiary
                        }))
                        .child(value),
                ),
        )
        .child(primitives::meter(
            share.map_or(0., |s| (s / 100.) as f32),
            intent,
            p,
        ))
}

/// A queue item's state in the shared vocabulary.
fn queue_status(state: &State) -> StatusBadge {
    match state {
        State::Queued => StatusBadge::new(StatusKind::Queued),
        State::Running => StatusBadge::new(StatusKind::Working).label("Running"),
        State::Done => StatusBadge::new(StatusKind::Done),
        State::Failed => StatusBadge::new(StatusKind::Failed),
        State::Cancelled => StatusBadge::new(StatusKind::Idle).label("Cancelled"),
        State::Unknown => StatusBadge::new(StatusKind::NeedsYou).label("Unknown"),
        State::Other(other) => StatusBadge::new(StatusKind::Idle).label(other.clone()),
    }
}

#[cfg(test)]
mod v2_tests {
    use super::*;

    #[test]
    fn gate_reasons_read_as_plain_words() {
        assert_eq!(
            paused_label(&["memory pressure 34.0 >= 25.0".to_owned()]),
            "Paused: low memory"
        );
        assert_eq!(plain_reason("cpu pressure 70 >= 60"), "busy CPU");
        assert_eq!(
            plain_reason_sentence("memory pressure 34.0 >= 25.0"),
            "Memory pressure is 34.0, over its limit of 25.0"
        );
        assert_eq!(plain_reason_sentence("something odd"), "something odd");
        assert_eq!(paused_label(&[]), "Paused");
    }

    #[test]
    fn load_words_follow_the_limit() {
        assert_eq!(load_word(Some(10.), 100.), "Quiet");
        assert_eq!(load_word(Some(75.), 100.), "Busy");
        assert_eq!(load_word(Some(95.), 100.), "Saturated");
        assert_eq!(load_word(None, 100.), "Unknown");
    }
}
