//! Settings → Remote: serve this machine's hub over Tailscale HTTPS and pair
//! phones with scoped tokens. Reads and changes use separate request keys so a
//! refresh never cancels a Serve change that is still running.
use super::*;
use gpui_component::{Disableable, switch::Switch};
use wks_native::features::Request;
use wks_native::remote::{Action as RemoteAction, Remote, SCOPES};

/// Settings-local choices that are not worth persisting.
pub(super) struct RemoteUi {
    pub scope: &'static str,
    pub confirm_revoke: Option<String>,
    pub confirm_reset: bool,
}

impl Default for RemoteUi {
    fn default() -> Self {
        Self {
            scope: "triage",
            confirm_revoke: None,
            confirm_reset: false,
        }
    }
}

impl Workspace {
    /// The newest successful read or change, whichever finished last.
    fn remote_state(&self) -> Option<Remote> {
        ["remote", "remote-action"]
            .into_iter()
            .filter_map(|key| self.view.requests.get(key))
            .filter(|s| !s.loading && s.error.is_none() && s.value.is_object())
            .max_by_key(|s| s.number)
            .map(|s| Remote::parse(&s.value))
    }

    fn remote_busy(&self) -> bool {
        ["remote", "remote-action"]
            .into_iter()
            .any(|key| self.view.requests.get(key).is_some_and(|s| s.loading))
    }

    pub(super) fn refresh_remote(&mut self, cx: &mut Context<Self>) {
        if !self
            .view
            .requests
            .get("remote-action")
            .is_some_and(|s| s.loading)
        {
            self.request(Request::Remote, cx);
        }
    }

    fn remote_action(&mut self, action: RemoteAction, cx: &mut Context<Self>) {
        if self.remote_busy() {
            return;
        }
        self.remote.confirm_revoke = None;
        self.remote.confirm_reset = false;
        self.request(Request::RemoteAction(action), cx);
    }

    fn note(&self, text: impl Into<SharedString>, color: u32) -> Div {
        div()
            .text_size(px(12.))
            .text_color(rgb(color))
            .child(text.into())
    }

    /// Tailscale status and the HTTPS switch.
    pub(super) fn render_remote_sharing(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let busy = self.remote_busy();
        let state = self.remote_state();
        let refresh = self
            .quiet_button("remote-refresh", "Check again", IconName::Redo, !busy)
            .when(!busy, |d| {
                d.on_click(cx.listener(|this, _, _, cx| this.refresh_remote(cx)))
            });
        let body = match &state {
            None if busy => div().child(self.note("Checking Tailscale…", p.muted)),
            None => div().child(self.note("Not checked yet.", p.muted)),
            Some(remote) if !remote.error.is_empty() => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(self.note(remote.error.clone(), p.warning))
                .child(self.note(
                    "Remote sharing is managed by the machine that owns this hub.",
                    p.muted,
                )),
            Some(remote) if !remote.available => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.note(
                    "Tailscale isn't running on this machine. Install it, sign in, then check again.",
                    p.muted,
                ))
                .child(
                    self.quiet_button(
                        "tailscale-download",
                        "Get Tailscale",
                        IconName::ExternalLink,
                        true,
                    )
                    .on_click(|_, _, cx| cx.open_url(wks_native::remote::TAILSCALE_DOWNLOAD)),
                ),
            Some(remote) => self.render_serve_switch(remote, busy, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(body)
            .child(self.feature_message("remote-action"))
            .child(self.feature_message("remote"))
            .child(div().flex().child(refresh))
    }

    fn render_serve_switch(&self, remote: &Remote, busy: bool, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let serving = remote.serving;
        let enabled = remote.can_serve && !busy;
        let confirm = self.remote.confirm_reset;
        let hint_link = wks_native::remote::hint_link(&remote.hint).map(str::to_owned);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(status_dot(if remote.reachable() {
                        p.success
                    } else {
                        p.disabled
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(if remote.reachable() {
                                        format!("https://{}", remote.magic_name)
                                    } else if remote.magic_name.is_empty() {
                                        "Tailscale is running".to_owned()
                                    } else {
                                        remote.magic_name.clone()
                                    }),
                            )
                            .child(self.note(
                                if busy {
                                    "Applying…"
                                } else if serving {
                                    "Shared with your tailnet over HTTPS"
                                } else {
                                    "Only this computer can reach Workspacer"
                                },
                                p.muted,
                            )),
                    )
                    .child(
                        Switch::new("tailscale-serve")
                            .checked(serving)
                            .disabled(!enabled)
                            .tooltip("Share over Tailscale HTTPS")
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                if *checked {
                                    this.remote_action(RemoteAction::Serve(true), cx);
                                } else {
                                    // Reset clears every Serve handler, not just ours.
                                    this.remote.confirm_reset = true;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .when(!remote.hint.is_empty(), |d| {
                d.child(self.note(remote.hint.clone(), p.warning))
                    .when_some(hint_link, |d, link| {
                        d.child(
                            div().flex().child(
                                self.quiet_button(
                                    "tailscale-hint",
                                    "Open Tailscale admin",
                                    IconName::ExternalLink,
                                    true,
                                )
                                .on_click(move |_, _, cx| cx.open_url(&link)),
                            ),
                        )
                    })
            })
            .when(confirm, |d| {
                d.child(
                    div()
                        .p_3()
                        .rounded(px(p.control_radius))
                        .bg(rgb(p.base))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.note(
                            "Turning this off runs `tailscale serve reset`, which removes every Tailscale Serve handler on this machine, not only Workspacer's.",
                            p.warning,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    self.primary_button("tailscale-reset", "Stop sharing", true)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.remote_action(RemoteAction::Serve(false), cx)
                                        })),
                                )
                                .child(self.button("tailscale-keep", "Keep sharing", true).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.remote.confirm_reset = false;
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
            })
    }

    /// Scope picker, QR code and link for one pairing.
    pub(super) fn render_phone_pairing(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let busy = self.remote_busy();
        let Some(remote) = self.remote_state() else {
            return div().child(self.note("Check Tailscale above first.", p.muted));
        };
        if !remote.reachable() {
            return div().child(self.note(
                "Turn on Tailscale HTTPS sharing above, then pair a phone signed in to the same tailnet.",
                p.muted,
            ));
        }
        if !remote.can_manage_tokens {
            return div().child(self.note(
                "Only the machine that owns this hub can create pairing links.",
                p.muted,
            ));
        }
        let scope = self.remote.scope;
        let description = SCOPES
            .iter()
            .find(|(id, ..)| *id == scope)
            .map(|(.., text)| *text)
            .unwrap_or_default();
        let url = remote.phone_url(scope);
        let picker = self.segmented(
            "pairing-scope",
            SCOPES
                .iter()
                .map(|(id, label, _)| (*id, (*label).to_owned()))
                .collect(),
            scope,
            |this, scope, _, cx: &mut Context<Self>| {
                this.remote.scope = scope;
                cx.notify();
            },
            cx,
        );
        let pairing = match url {
            Some(url) => {
                let copy = url.clone();
                let open = url.clone();
                div()
                    .flex()
                    .gap_4()
                    .items_start()
                    .child(qr_code(&url).debug_selector(|| "pairing-qr".into()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.note(
                                "Scan with your phone's camera. Add it to the home screen for notifications.",
                                p.muted,
                            ))
                            .child(
                                div()
                                    .p_2()
                                    .rounded(px(p.control_radius))
                                    .bg(rgb(p.base))
                                    .font_family(self.fonts.resolved(&self.settings.code_font, "JetBrains Mono"))
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .overflow_hidden()
                                    .child(url),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(
                                        self.quiet_button("pairing-copy", "Copy link", IconName::Copy, true)
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                            }),
                                    )
                                    .child(
                                        self.quiet_button("pairing-open", "Open here", IconName::ExternalLink, true)
                                            .on_click(move |_, _, cx| cx.open_url(&open)),
                                    ),
                            ),
                    )
            }
            None => div().child(
                self.primary_button(
                    "pairing-create",
                    if busy {
                        "Creating…"
                    } else {
                        "Create pairing link"
                    },
                    !busy,
                )
                .debug_selector(|| "pairing-create".into())
                .when(!busy, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        this.remote_action(RemoteAction::Pair(scope.to_owned()), cx)
                    }))
                }),
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(picker)
            .child(self.note(
                description,
                if scope == "operator" {
                    p.warning
                } else {
                    p.muted
                },
            ))
            .child(pairing)
    }

    /// Every pairing token, each revocable.
    pub(super) fn render_paired_devices(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let busy = self.remote_busy();
        let pairings = self
            .remote_state()
            .filter(|remote| remote.can_manage_tokens)
            .map(|remote| remote.pairings)
            .unwrap_or_default();
        if pairings.is_empty() {
            return div().child(self.note("No pairing links yet.", p.muted));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(pairings.into_iter().enumerate().map(|(ix, pairing)| {
                let confirming = self.remote.confirm_revoke.as_deref() == Some(&pairing.token);
                let token = pairing.token.clone();
                let created = chrono::DateTime::parse_from_rfc3339(&pairing.created)
                    .map(|at| {
                        format!(
                            "Created {}",
                            at.with_timezone(&chrono::Local).format("%b %-d, %Y")
                        )
                    })
                    .unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(wks_native::remote::scope_label(&pairing.scope)),
                            )
                            .when(!created.is_empty(), |d| {
                                d.child(self.note(created, p.muted))
                            }),
                    )
                    .child(
                        self.button(
                            format!("pairing-revoke-{ix}"),
                            if confirming {
                                "Confirm revoke"
                            } else {
                                "Revoke"
                            },
                            !busy,
                        )
                        .debug_selector(move || format!("pairing-revoke-{ix}"))
                        .when(confirming, |d| d.text_color(rgb(p.warning)))
                        .when(!busy, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                if this.remote.confirm_revoke.as_deref() == Some(&token) {
                                    this.remote_action(RemoteAction::Revoke(token.clone()), cx);
                                } else {
                                    this.remote.confirm_revoke = Some(token.clone());
                                    cx.notify();
                                }
                            }))
                        }),
                    )
            }))
            .child(self.note(
                "Revoking disconnects that phone immediately. Its link stops working.",
                p.disabled,
            ))
    }
}

/// Black modules on a white quiet zone so it scans in every theme.
fn qr_code(text: &str) -> Div {
    let Some(rows) = wks_native::remote::qr_modules(text) else {
        return div();
    };
    let module = px((176. / rows.len() as f32).floor().max(2.));
    div()
        .flex_shrink_0()
        .p_3()
        .rounded(px(8.))
        .bg(rgb(0xffffff))
        .flex()
        .flex_col()
        .children(rows.into_iter().map(move |row| {
            div().flex().children(
                row.into_iter()
                    .map(move |dark| div().size(module).when(dark, |d| d.bg(rgb(0x000000)))),
            )
        }))
}
