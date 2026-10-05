//! Fleet and supervisor wakes as cards, following desktop `FleetMessageCard`
//! and the mobile `fleetCardHtml`. A wake arrives as a plain user turn, so
//! without this it reads as a giant "You" bubble with raw session IDs. The
//! card names the worker(s) instead of the user, shows each one's live state,
//! and keeps the delivered text one disclosure away.
use super::*;
use gpui_component::tooltip::Tooltip;
use wks_native::transcript::{Fleet, FleetEntry, FleetKind};

/// How the card reads at a glance: overline, icon and tone.
fn kind_face(kind: FleetKind, p: Palette) -> (&'static str, Icon, u32) {
    let lucide = |path: &'static str| Icon::empty().path(path);
    match kind {
        FleetKind::Finished => (
            "Fleet · worker finished",
            Icon::new(IconName::CircleCheck),
            p.success,
        ),
        FleetKind::Failed => (
            "Fleet · worker failed",
            Icon::new(IconName::CircleX),
            p.error,
        ),
        FleetKind::Escalated => (
            "Fleet · worker escalated",
            Icon::new(IconName::TriangleAlert),
            p.warning,
        ),
        FleetKind::CatchUp => ("Fleet · catch-up", Icon::new(IconName::Inbox), p.success),
        FleetKind::Blocked => (
            "Supervisor · decision needed",
            Icon::new(IconName::TriangleAlert),
            p.warning,
        ),
        FleetKind::Threshold => (
            "Fleet · threshold crossed",
            Icon::new(IconName::TriangleAlert),
            p.warning,
        ),
        // Not a completion: accent and a megaphone, never the success check.
        FleetKind::Progress => (
            "Fleet · progress update",
            lucide("lucide/megaphone.svg"),
            p.accent,
        ),
    }
}

/// One entry's state, preferring what the session shows now over what the
/// wake said then: an approval that was granted reads "Resolved".
pub(super) fn entry_status(
    kind: FleetKind,
    entry: &FleetEntry,
    live: Option<&Session>,
    p: Palette,
) -> (&'static str, u32) {
    if entry.stopped {
        return ("Stopped", p.error);
    }
    if entry.failed.is_some() {
        return ("Failed", p.error);
    }
    match entry.blocked_on.as_deref() {
        Some("approval") => {
            return match live {
                Some(s) if s.approval.is_some() => ("Needs approval", p.warning),
                Some(_) => ("Resolved", p.success),
                None => ("Approval requested", p.muted),
            };
        }
        Some(_) => {
            return match live {
                Some(s) if s.questions.is_some() => ("Has a question", p.busy),
                Some(_) => ("Answered", p.success),
                None => ("Question asked", p.muted),
            };
        }
        None => {}
    }
    if entry.needs_decision {
        return ("Needs a decision", p.warning);
    }
    match kind {
        FleetKind::Finished | FleetKind::CatchUp => ("Finished", p.success),
        FleetKind::Failed => ("Failed", p.error),
        FleetKind::Escalated => ("Escalated", p.warning),
        FleetKind::Blocked => ("Blocked", p.warning),
        FleetKind::Threshold => ("Threshold crossed", p.warning),
        FleetKind::Progress => ("Still running", p.accent),
    }
}

/// A tinted pill: the one strong mark per entry.
fn status_badge(label: &'static str, tone: u32) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py(px(1.))
        .rounded_full()
        .bg(gpui::Hsla::from(rgb(tone)).opacity(0.14))
        .text_color(rgb(tone))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn short_id(id: &str) -> String {
    if id.chars().count() > 10 {
        format!("{}…", id.chars().take(8).collect::<String>())
    } else {
        id.to_owned()
    }
}

impl Workspace {
    /// The card title: the worker's own name, or how many there are. The
    /// session's current label wins over the one frozen into the wake.
    pub(super) fn fleet_title(&self, fleet: &Fleet) -> String {
        match fleet.entries.as_slice() {
            [entry] => self
                .view
                .sessions
                .iter()
                .find(|s| s.id == entry.session_id)
                .map(|s| self.session_title(s))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| entry.label.clone()),
            entries => format!("{} sessions", entries.len()),
        }
    }

    /// Opening is offered only for this conversation's own workers (or an
    /// agent with no recorded parent), and rechecked at click time.
    fn fleet_openable(&self, owner: &str, id: &str) -> bool {
        self.view.connected
            && self.view.sessions.iter().any(|s| {
                s.id == id && (s.parent_session_id.is_empty() || s.parent_session_id == owner)
            })
    }

    pub(super) fn render_fleet_card(
        &mut self,
        key: &str,
        fleet: Fleet,
        copy: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let owner = self.view.selected.clone().unwrap_or_default();
        let (overline, icon, tone) = kind_face(fleet.kind, p);
        let single = fleet.entries.len() == 1;
        let lookup = |id: &str| self.view.sessions.iter().find(|s| s.id == id).cloned();
        let header_status = single.then(|| {
            let entry = &fleet.entries[0];
            entry_status(fleet.kind, entry, lookup(&entry.session_id).as_ref(), p)
        });
        let title = self.fleet_title(&fleet);
        let mut card = div()
            .debug_selector(|| "fleet-card".into())
            .relative()
            .w_full()
            .overflow_hidden()
            .rounded(px(p.panel_radius))
            .border_1()
            .border_color(rgb(p.border))
            .bg(gpui::Hsla::from(rgb(p.surface)).opacity(0.6))
            .flex()
            .flex_col()
            // Tone rail, as the desktop Surface's: the kind before any words.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.))
                    .bg(rgb(tone)),
            )
            .child(
                div()
                    .debug_selector(|| "fleet-card-header".into())
                    .pl_4()
                    .pr_2()
                    .pt_2()
                    .pb_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon.size(px(14.)).text_color(rgb(tone)).flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(chrome::scale::OVERLINE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(p.muted))
                                    .child(overline.to_uppercase()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "fleet-card-title".into())
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(p.text))
                                    .child(title),
                            ),
                    )
                    .when_some(header_status, |d, (label, tone)| {
                        d.child(
                            status_badge(label, tone).debug_selector(|| "fleet-card-status".into()),
                        )
                    })
                    .child(
                        self.icon_button(
                            SharedString::from(format!("copy-{key}")),
                            "Copy wake",
                            IconName::Copy,
                            true,
                        )
                        .size(px(24.))
                        .opacity(0.)
                        .group_hover("message", |style| style.opacity(1.))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                    ),
            );
        for (index, entry) in fleet.entries.iter().enumerate() {
            let live = lookup(&entry.session_id);
            let (status, status_tone) = entry_status(fleet.kind, entry, live.as_ref(), p);
            let openable = self.fleet_openable(&owner, &entry.session_id);
            let provider = live
                .as_ref()
                .map(|s| s.provider.clone())
                .unwrap_or_default();
            let model = live
                .as_ref()
                .map(Session::display_model)
                .unwrap_or_default();
            let name = live
                .as_ref()
                .map(|s| self.session_title(s))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| entry.label.clone());
            let id = entry.session_id.clone();
            let cwd = entry.cwd.clone().filter(|c| c != "?");
            let reply = format!("Re: session:{id} ({}) — ", entry.label);
            let id_tooltip = if live.is_some() {
                format!("session:{id}")
            } else {
                format!("session:{id} · no longer in this workspace")
            };
            let mut row = div()
                .debug_selector(move || format!("fleet-entry-{index}"))
                .pl_4()
                .pr_2()
                .py_2()
                .when(!single || index > 0, |d| {
                    d.border_t_1().border_color(rgb(p.border))
                })
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(chrome::provider_mark(&provider, 20., p))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .when(!single, |d| {
                                    d.child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(p.text))
                                            .child(name),
                                    )
                                })
                                // Quiet metadata: model, folder and a short,
                                // hoverable ID — never the raw UUID in prose.
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .min_w_0()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.muted))
                                        .when(!model.is_empty(), |d| {
                                            d.child(div().flex_shrink_0().child(model))
                                                .child(div().text_color(rgb(p.disabled)).child("·"))
                                        })
                                        .when_some(cwd.clone(), |d, cwd| {
                                            d.child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(chrome::project_label(&cwd).to_owned()),
                                            )
                                            .child(div().text_color(rgb(p.disabled)).child("·"))
                                        })
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("{key}-id-{index}")))
                                                .debug_selector(move || {
                                                    format!("fleet-entry-id-{index}")
                                                })
                                                .flex_shrink_0()
                                                .font_family(super::mono_font())
                                                .text_color(rgb(p.disabled))
                                                .child(short_id(&id))
                                                .tooltip(move |window, cx| {
                                                    Tooltip::new(id_tooltip.clone())
                                                        .build(window, cx)
                                                }),
                                        ),
                                ),
                        )
                        .when(!single, |d| {
                            d.child(
                                status_badge(status, status_tone)
                                    .debug_selector(move || format!("fleet-entry-status-{index}")),
                            )
                        })
                        .child(
                            self.icon_button(
                                SharedString::from(format!("{key}-open-{id}")),
                                if openable {
                                    "Open session"
                                } else {
                                    "Not an available worker of this conversation"
                                },
                                IconName::ArrowRight,
                                openable,
                            )
                            .debug_selector(move || format!("fleet-entry-open-{index}"))
                            .when(openable, |d| {
                                let owner = owner.clone();
                                let id = id.clone();
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    if this.view.selected.as_deref() == Some(owner.as_str())
                                        && this.fleet_openable(&owner, &id)
                                    {
                                        this.command(Command::Select(id.clone()), cx);
                                    }
                                }))
                            }),
                        )
                        .child(
                            interactive_reply(self, key, &entry.session_id, index, p).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.prefill(&reply, window, cx)
                                }),
                            ),
                        ),
                );
            if let Some(failed) = &entry.failed {
                row = row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.error))
                        .child(failed.clone()),
                );
            }
            if let Some(crossed) = &entry.crossed {
                row = row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.warning))
                        .child(crossed.clone()),
                );
            }
            // A still-running worker's own words are shown open.
            if let Some(note) = &entry.note {
                row = row.child(
                    div()
                        .pl_2()
                        .border_l_2()
                        .border_color(rgb(p.accent))
                        .text_size(px(13.))
                        .text_color(rgb(p.prose))
                        .child(note.clone()),
                );
            }
            if let Some(last) = &entry.last_reply {
                let open_key = format!("{key}-reply-{index}");
                let open = self.chat.open.get(&open_key).copied().unwrap_or(false);
                row = row.child(self.fleet_disclosure(open_key, "Last reply", open, cx));
                if open {
                    row = row.child(
                        div()
                            .pl_2()
                            .border_l_2()
                            .border_color(rgb(p.border))
                            .text_size(px(13.))
                            .text_color(rgb(p.prose))
                            .child(last.clone()),
                    );
                }
            }
            card = card.child(row);
        }
        if !fleet.reports.is_empty() {
            let mut reports = div().pl_4().pr_2().pb_2().flex().flex_col().gap_2();
            for (i, (title, report)) in fleet.reports.iter().enumerate() {
                let report_key = format!("{key}-report-{i}");
                reports = reports.child(if title.starts_with("Structured result") {
                    self.render_result(&report_key, title, report, window, cx)
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(title.clone()),
                        )
                        .child(self.render_raw(&report_key, report, window, cx))
                        .into_any_element()
                });
            }
            card = card.child(reports);
        }
        // The delivered text stays one quiet disclosure away: it is what the
        // agent read, and the identity of record.
        let original = format!("{key}-original");
        let expanded = self.chat.open.get(&original).copied().unwrap_or(false);
        card = card.child(div().pl_4().pr_2().pb_2().child(self.fleet_disclosure(
            original.clone(),
            "Original wake",
            expanded,
            cx,
        )));
        if expanded {
            card = card.child(div().pl_4().pr_2().pb_2().child(self.render_raw(
                &original,
                &fleet.text,
                window,
                cx,
            )));
        }
        card
    }

    /// A small chevron toggle that keeps the reader's place, like tool cards.
    fn fleet_disclosure(
        &self,
        key: String,
        label: &'static str,
        open: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = self.appearance.palette();
        chrome::interactive_control(
            div().id(SharedString::from(format!("toggle-{key}"))),
            p,
            true,
        )
        .debug_selector(move || format!("fleet-toggle-{label}"))
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .rounded(px(p.control_radius))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(p.muted))
        .hover(move |s| s.text_color(rgb(p.text)))
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(px(12.)),
        )
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_open(key.clone(), false, cx)))
    }
}

/// The Reply affordance: Lucide's reply arrow, as desktop's icon-only button.
fn interactive_reply(
    ws: &Workspace,
    key: &str,
    session: &str,
    index: usize,
    p: Palette,
) -> Stateful<Div> {
    chrome::interactive_control(
        div().id(SharedString::from(format!("{key}-reply-{session}"))),
        p,
        true,
    )
    .debug_selector(move || format!("fleet-entry-reply-{index}"))
    .size(px(28.))
    .rounded_full()
    .flex()
    .items_center()
    .justify_center()
    .flex_shrink_0()
    .text_color(rgb(p.muted))
    .hover_text_style(move |s| s.bg(rgb(p.selected)).text_color(rgb(p.text)))
    .tooltip(|window, cx| Tooltip::new("Reply to this session").build(window, cx))
    .child(
        gpui::svg()
            .path("lucide/reply.svg")
            .size(px(13.))
            .text_color(rgb(if ws.view.connected {
                p.muted
            } else {
                p.disabled
            })),
    )
}
