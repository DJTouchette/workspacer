//! Native self-update UI: background checks on the build's own channel, the
//! Settings → About card, the sidebar "Update" pill, and the installer
//! handoff once a verified download lands.
use super::*;
use wks_native::features::Request;

/// First check shortly after launch, then this often.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

impl Workspace {
    /// Called by the app after the window opens; tests never start it, so
    /// they make no network requests.
    pub fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        self.extras._update_timer = Some(cx.spawn(async move |this, cx| {
            let mut wait = std::time::Duration::from_secs(8);
            loop {
                cx.background_executor().timer(wait).await;
                wait = RECHECK;
                let gone = this
                    .update(cx, |this, cx| {
                        let downloading = this
                            .view
                            .requests
                            .get("update-download")
                            .is_some_and(|s| s.loading);
                        if !downloading {
                            this.request(Request::Updates, cx);
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        }));
    }

    /// The last successful check.
    pub(super) fn update_status(&self) -> Option<&serde_json::Value> {
        self.view
            .requests
            .get("updates")
            .filter(|s| !s.loading && s.error.is_none() && s.value.is_object())
            .map(|s| s.value.as_ref())
    }

    pub(super) fn update_available(&self) -> bool {
        self.update_status()
            .is_some_and(|s| s["update_available"] == true)
    }

    /// Once the installer is downloaded and verified, start the helper and
    /// quit so it can replace the files; it relaunches the app afterwards.
    pub(super) fn hand_off_update(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.view.requests.get("update-download") else {
            return;
        };
        if state.loading || state.number <= self.extras.update_handoff {
            return;
        }
        self.extras.update_handoff = state.number;
        let Some(installer) = state.value["installer"].as_str() else {
            return;
        };
        match wks_native::updates::hand_off(std::path::Path::new(installer)) {
            Ok(()) => cx.quit(),
            Err(error) => {
                self.extras.notice = format!("Update could not start: {error:#}");
                cx.notify();
            }
        }
    }

    /// Settings → About: channel, versions, status and actions.
    pub(super) fn render_update_card(&self, cx: &mut Context<Self>) -> Div {
        let p = self.appearance.palette();
        let installed = wks_native::features::installed_version();
        let channel = wks_native::updates::Channel::of(&installed);
        let checking = self.view.requests.get("updates").is_some_and(|s| s.loading);
        let downloading = self
            .view
            .requests
            .get("update-download")
            .is_some_and(|s| s.loading);
        let status = self.update_status().cloned();
        let available = status
            .as_ref()
            .is_some_and(|s| s["update_available"] == true);
        let installable = status.as_ref().is_some_and(|s| s["installable"] == true);
        let latest = status
            .as_ref()
            .and_then(|s| s["latest"].as_str().map(str::to_owned));
        let release_url = status
            .as_ref()
            .and_then(|s| s["release_url"].as_str().map(str::to_owned))
            .unwrap_or_else(|| wks_native::features::RELEASES_URL.to_owned());
        let asset = status.as_ref().map(|s| s["asset"].clone());
        let (line, tone) = match (&status, latest.as_deref()) {
            (_, _) if checking => ("Checking…".to_owned(), p.muted),
            (Some(s), _) if s["update_available"].is_null() => (
                "Development build: version comparison is unavailable".to_owned(),
                p.muted,
            ),
            (Some(_), Some(latest)) if available => {
                (format!("Update available: {latest}"), p.accent)
            }
            (Some(_), Some(_)) => ("You're up to date".to_owned(), p.success),
            (Some(_), None) => ("No release found on this channel".to_owned(), p.muted),
            (None, _) => ("Not checked yet".to_owned(), p.muted),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(format!("Installed {installed}"))
                    .child(
                        div()
                            .px_2()
                            .rounded_full()
                            .bg(rgb(p.selected))
                            .text_color(rgb(if channel == wks_native::updates::Channel::Nightly {
                                p.busy
                            } else {
                                p.text
                            }))
                            .child(channel.label()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "update-status".into())
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(tone))
                    .child(line),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .when(available && installable, |d| {
                        d.child(
                            self.primary_button(
                                "install-update",
                                if downloading {
                                    "Downloading…"
                                } else {
                                    "Install and restart"
                                },
                                !downloading,
                            )
                            .debug_selector(|| "install-update".into())
                            .when(!downloading, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(asset) = asset.clone() {
                                        this.request(Request::DownloadUpdate { asset }, cx);
                                    }
                                }))
                            }),
                        )
                    })
                    .child(
                        self.button(
                            "check-updates",
                            if checking {
                                "Checking…"
                            } else {
                                "Check for updates"
                            },
                            !checking && !downloading,
                        )
                        .when(!checking && !downloading, |d| {
                            d.on_click(
                                cx.listener(|this, _, _, cx| this.request(Request::Updates, cx)),
                            )
                        }),
                    )
                    .child(
                        self.quiet_button(
                            "open-releases",
                            if available && !installable {
                                "Download"
                            } else {
                                "Release notes"
                            },
                            IconName::ExternalLink,
                            true,
                        )
                        .on_click(move |_, _, cx| cx.open_url(&release_url)),
                    ),
            )
            .child(self.feature_message("updates"))
            .child(self.feature_message("update-download"))
            .when(available && installable, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.disabled))
                        .child("The app closes, the installer runs silently, and the app reopens."),
                )
            })
    }
}
