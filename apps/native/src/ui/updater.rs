//! Native self-update UI: background checks on the build's own channel, the
//! Settings → About card, the sidebar "Update" pill, and the installer
//! handoff once a verified download lands.
use super::*;
use wks_native::features::Request;

/// First check shortly after launch, then this often.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

impl Workspace {
    /// Called by the app after the window opens; tests never start it, so
    /// they make no network requests and read no update state.
    pub fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        // What the previous update did, reported once after the relaunch.
        if let Some(outcome) = wks_native::updates::state_dir()
            .ok()
            .and_then(|dir| wks_native::updates::take_outcome(&dir))
        {
            self.extras.update_notice = outcome_notice(&outcome);
        }
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

    /// Once the installer is downloaded and verified, hand it off.
    pub(super) fn hand_off_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.view.requests.get("update-download") else {
            return;
        };
        if state.loading || state.number <= self.extras.update_handoff {
            return;
        }
        self.extras.update_handoff = state.number;
        // A failed download already shows its error.
        if state.error.is_some() {
            return;
        }
        let (Some(installer), Some(version)) = (
            state.value["installer"].as_str(),
            state.value["version"].as_str(),
        ) else {
            self.extras.update_notice =
                "Update could not start: the download did not report its version.".into();
            cx.notify();
            return;
        };
        self.extras.update_installer = Some((installer.to_owned(), version.to_owned()));
        self.install_update(window, cx);
    }

    fn has_unsaved_edits(&self, cx: &App) -> bool {
        self.showing_pane()
            .is_some_and(|pane| pane.read(cx).dirty())
    }

    /// Start the helper, and quit only once it reports that it is waiting,
    /// through the same unsaved-edits guard as Quit. It installs after this
    /// process exits and relaunches the app; any failure keeps the app open.
    pub(super) fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((installer, version)) = self.extras.update_installer.clone() else {
            return;
        };
        if self.extras.update_handing_off {
            return;
        }
        // An accepted handoff may still own an installer operation. Recheck
        // its live process and receipt, never launch another based on a timer.
        if self.extras.update_helper_ready.is_some() {
            self.quit_for_update(&version, window, cx);
            return;
        }
        if self.has_unsaved_edits(cx) {
            self.extras.update_notice = UNSAVED_BEFORE_UPDATE.into();
            cx.notify();
            return;
        }
        let handoff = match wks_native::updates::Handoff::current(installer.into(), &version) {
            Ok(handoff) => handoff,
            Err(error) => {
                self.extras.update_notice = format!("Update could not start: {error:#}");
                cx.notify();
                return;
            }
        };
        self.extras.update_handing_off = true;
        self.extras.update_notice = "Starting the installer…".into();
        cx.notify();
        let starter = self.extras.update_starter.clone();
        let start = cx
            .background_executor()
            .spawn(async move { starter(&handoff) });
        cx.spawn_in(window, async move |this, cx| {
            let result = start.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.extras.update_handing_off = false;
                match result {
                    Ok(helper) => {
                        this.extras.update_helper_ready = Some(helper);
                        this.quit_for_update(&version, window, cx);
                    }
                    Err(error) => {
                        if error.is::<wks_native::updates::MissingInstaller>() {
                            this.extras.update_installer = None;
                        }
                        this.extras.update_notice = format!("Update could not start: {error:#}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Quit through the unsaved-edits guard, like Quit. When the editor asks
    /// first, its Save or Discard closes the app and the waiting helper
    /// installs; Keep editing leaves the app open.
    fn quit_for_update(&mut self, version: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .extras
            .update_helper_ready
            .as_mut()
            .is_some_and(|helper| helper.is_waiting().unwrap_or(false))
        {
            self.extras.update_notice = "Update could not continue: the helper no longer confirms readiness. Restart Workspacer before retrying, or run the downloaded installer manually.".into();
            cx.notify();
            return;
        }
        if self.confirm_update_close(window, cx) {
            self.extras.update_notice = format!("Closing to install {version}…");
            cx.quit();
        } else {
            self.extras.update_notice = UNSAVED_DURING_UPDATE.into();
        }
        cx.notify();
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
        let handing_off = self.extras.update_handing_off;
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
                                } else if handing_off {
                                    "Starting installer…"
                                } else {
                                    "Install and restart"
                                },
                                !downloading && !handing_off,
                            )
                            .debug_selector(|| "install-update".into())
                            .when(!downloading && !handing_off, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    // A verified download is reused, never fetched again.
                                    if this.extras.update_installer.is_some() {
                                        this.install_update(window, cx);
                                    } else if let Some(asset) = asset.clone() {
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
            .when(!self.extras.update_notice.is_empty(), |d| {
                let notice = self.extras.update_notice.clone();
                let tone = update_tone(&notice);
                d.child(
                    chrome::notice_line(notice, tone, p, "update-notice")
                        .debug_selector(|| "update-notice".into()),
                )
            })
            .when(available && installable, |d| {
                d.child(div().text_size(px(11.)).text_color(rgb(p.disabled)).child(
                    "The app closes, the installer runs silently, and the app reopens. \
                             Unsaved edits are saved or discarded first.",
                ))
            })
    }
}

const UNSAVED_BEFORE_UPDATE: &str =
    "Save or discard your unsaved edits, then choose Install and restart.";
const UNSAVED_DURING_UPDATE: &str = "Save or discard your unsaved edits to finish the update. \
     It installs when Workspacer closes.";

/// The About card line for the last update's recorded outcome.
fn outcome_notice(outcome: &serde_json::Value) -> String {
    let version = outcome["expected"].as_str().unwrap_or("the new version");
    match outcome["state"].as_str() {
        Some("succeeded") => format!("Updated to {version}."),
        _ => {
            let detail = outcome["detail"]
                .as_str()
                .filter(|d| !d.is_empty())
                .unwrap_or("The update did not finish.");
            match outcome["log"].as_str() {
                Some(log) => format!("Update to {version} failed: {detail} Log: {log}"),
                None => format!("Update to {version} failed: {detail}"),
            }
        }
    }
}

/// Unsaved-edit notices mention "saved", so the shared classifier would call
/// them successes.
fn update_tone(notice: &str) -> chrome::Tone {
    if notice.starts_with("Updated to ") {
        chrome::Tone::Success
    } else if notice.contains("failed") || notice.contains("could not") {
        chrome::Tone::Error
    } else if notice.ends_with('…') {
        chrome::Tone::Loading
    } else {
        chrome::Tone::Warning
    }
}
