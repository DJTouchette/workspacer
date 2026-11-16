//! Hub settings that used to live only in `config.yaml`: keep-warm
//! (`claude.keepWarm`), worktree artifact cleanup (`agents.artifactCleanup`),
//! Fleet workers' access, the Fleet Manager's provider and the hub's startup
//! checks. The hub runs all of them, so they are its shared settings, read
//! and verified on readback like child access (see `features::hub_settings`).
use super::*;
use gpui_component::{Disableable, switch::Switch};
use serde_json::{Value, json};
use wks_native::features::{
    HubChange, KEEP_WARM_PROVIDERS, MANAGER_PROVIDERS, Request, shift_daily_at,
};

const INTERVALS: [f64; 6] = [1., 2., 3., 5., 8., 12.];
const MIN_AGES: [f64; 4] = [1., 2., 6., 24.];

fn hours_label(hours: f64) -> String {
    if hours == 24. {
        "1 day".into()
    } else if hours.fract() == 0. {
        format!("{hours:.0} h")
    } else {
        format!("{hours} h")
    }
}

/// Presets plus a configured value they do not list (shown, never replaced).
fn hour_options(presets: &[f64], current: f64) -> Vec<(f64, String)> {
    let mut options: Vec<(f64, String)> = presets.iter().map(|h| (*h, hours_label(*h))).collect();
    if current.is_finite() && current >= 0. && !presets.contains(&current) {
        options.push((current, hours_label(current)));
        options.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    options
}

fn provider_label(id: &str) -> &'static str {
    match id {
        "claude" => "Claude",
        "codex" => "Codex",
        "copilot" => "Copilot",
        "opencode" => "OpenCode",
        _ => "Other",
    }
}

impl Workspace {
    fn hub_value(&self) -> Option<&Value> {
        self.extras.hub_settings.as_ref()
    }

    fn hub_writable(&self) -> bool {
        self.view.connected
            && !self.demo
            && self.hub_value().is_some()
            && !self
                .view
                .requests
                .get("hub-settings")
                .is_some_and(|s| s.loading)
    }

    fn change_hub(&mut self, change: HubChange, cx: &mut Context<Self>) {
        if !self.hub_writable() {
            return;
        }
        self.request(Request::HubSettings { set: Some(change) }, cx);
    }

    /// The status for one group: a failed write shows only under the group
    /// whose setting it was; reading and offline show under every group.
    fn hub_status(&self, prefixes: &[&str]) -> Option<(String, chrome::Tone)> {
        let state = self.view.requests.get("hub-settings");
        if let Some(state) = state.filter(|s| !s.loading)
            && let Some(error) = &state.error
        {
            let mine = match &state.request {
                Request::HubSettings { set: Some(change) } => {
                    prefixes.iter().any(|p| change.pointer.starts_with(p))
                }
                // A failed read leaves every group unknown.
                _ => true,
            };
            if mine {
                return Some((
                    format!("Couldn't reach the hub's setting: {error}"),
                    chrome::Tone::Error,
                ));
            }
        }
        match self.hub_value() {
            Some(_) => None,
            None if self.view.connected && !self.demo => {
                Some(("Reading the hub's setting…".into(), chrome::Tone::Loading))
            }
            None => Some((
                "Connect to a hub to change this.".into(),
                chrome::Tone::Info,
            )),
        }
    }

    fn hub_bool(&self, pointer: &str) -> Option<bool> {
        self.hub_value()
            .and_then(|v| v.pointer(pointer))
            .and_then(Value::as_bool)
    }

    /// A switch bound to one boolean the snapshot reports at `read`, saved
    /// to the config at `write`.
    fn hub_switch(
        &self,
        id: &'static str,
        read: &'static str,
        write: &'static str,
        tooltip: &'static str,
        cx: &mut Context<Self>,
    ) -> Switch {
        Switch::new(id)
            .checked(self.hub_bool(read) == Some(true))
            .disabled(!self.hub_writable())
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                this.change_hub(HubChange::new(write, *checked), cx);
            }))
    }

    fn hub_note(&self, prefixes: &[&str], text: Option<(String, chrome::Tone)>) -> Option<Div> {
        let p = self.palette();
        self.hub_status(prefixes)
            .or(text)
            .map(|(text, tone)| div().child(chrome::notice_line(text, tone, p)))
    }

    fn labelled(&self, label: &'static str, control: impl IntoElement) -> Div {
        let p = self.palette();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(96.))
                    .flex_shrink_0()
                    .text_size(px(scale::META))
                    .text_color(rgb(p.muted))
                    .child(label),
            )
            .child(control)
    }

    pub(super) fn render_keep_warm(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let writable = self.hub_writable();
        let warm = self.hub_value().map(|v| v["keepWarm"].clone());
        let enabled = self.hub_bool("/keepWarm/enabled") == Some(true);
        let providers: Vec<String> = warm
            .as_ref()
            .and_then(|w| w["providers"].as_array())
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let has = |id: &str| providers.iter().any(|p| p == id);
        let chosen = match (has("claude"), has("codex")) {
            (true, true) => "both",
            (false, true) => "codex",
            _ => "claude",
        };
        let mode = warm
            .as_ref()
            .and_then(|w| w["mode"].as_str())
            .unwrap_or("auto")
            .to_owned();
        let interval = warm
            .as_ref()
            .and_then(|w| w["intervalHours"].as_f64())
            .unwrap_or(5.);
        let daily_at = warm
            .as_ref()
            .and_then(|w| w["dailyAt"].as_str())
            .unwrap_or("08:00")
            .to_owned();
        let settings_enabled = writable && enabled;

        let mode_detail = match mode.as_str() {
            "interval" => Some(
                self.labelled(
                    "Check every",
                    self.segmented_enabled(
                        "keep-warm-interval",
                        hour_options(&INTERVALS, interval),
                        interval,
                        settings_enabled,
                        |this, hours, _, cx| {
                            this.change_hub(
                                HubChange::new("/claude/keepWarm/intervalHours", hours),
                                cx,
                            )
                        },
                        cx,
                    ),
                )
                .into_any_element(),
            ),
            "daily" => {
                let earlier = shift_daily_at(&daily_at, -1);
                let later = shift_daily_at(&daily_at, 1);
                Some(
                    self.labelled(
                        "Check at",
                        div()
                            .debug_selector(|| "keep-warm-daily-at".into())
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                self.icon_button(
                                    "keep-warm-earlier",
                                    "An hour earlier",
                                    IconName::Minus,
                                    settings_enabled,
                                )
                                .when(settings_enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.change_hub(
                                            HubChange::new(
                                                "/claude/keepWarm/dailyAt",
                                                earlier.clone(),
                                            ),
                                            cx,
                                        )
                                    }))
                                }),
                            )
                            .child(
                                div()
                                    .min_w(px(52.))
                                    .flex()
                                    .justify_center()
                                    .text_size(px(scale::BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if settings_enabled {
                                        p.text
                                    } else {
                                        p.disabled
                                    }))
                                    .child(daily_at.clone()),
                            )
                            .child(
                                self.icon_button(
                                    "keep-warm-later",
                                    "An hour later",
                                    IconName::Plus,
                                    settings_enabled,
                                )
                                .when(settings_enabled, |d| {
                                    d.on_click(cx.listener(move |this, _, _, cx| {
                                        this.change_hub(
                                            HubChange::new(
                                                "/claude/keepWarm/dailyAt",
                                                later.clone(),
                                            ),
                                            cx,
                                        )
                                    }))
                                }),
                            )
                            .child(
                                div()
                                    .text_size(px(scale::CAPTION))
                                    .text_color(rgb(p.muted))
                                    .child("hub machine's local time"),
                            ),
                    )
                    .into_any_element(),
                )
            }
            _ => None,
        };

        let summary = match (enabled, mode.as_str()) {
            (false, _) => (
                "Off: windows start with your first message.".to_owned(),
                chrome::Tone::Info,
            ),
            (true, "interval") => (
                format!(
                    "On: every {} the hub starts a window for any provider whose window has lapsed.",
                    hours_label(interval)
                ),
                chrome::Tone::Success,
            ),
            (true, "daily") => (
                format!(
                    "On: at {daily_at} each day the hub starts a window for any provider whose window has lapsed."
                ),
                chrome::Tone::Success,
            ),
            (true, _) => (
                "On: the hub starts a new window as soon as one lapses.".to_owned(),
                chrome::Tone::Success,
            ),
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .debug_selector(|| "keep-warm-switch".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.hub_switch(
                        "settings-keep-warm",
                        "/keepWarm/enabled",
                        "/claude/keepWarm/enabled",
                        "Keep usage windows warm",
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(scale::META))
                            .child("Keep usage windows warm"),
                    ),
            )
            .child(self.labelled(
                "Providers",
                self.segmented_enabled(
                    "keep-warm-providers",
                    vec![
                        ("claude", "Claude".into()),
                        ("codex", "Codex".into()),
                        ("both", "Both".into()),
                    ],
                    chosen,
                    settings_enabled,
                    |this, choice, _, cx| {
                        let list: Vec<&str> = match choice {
                            "both" => KEEP_WARM_PROVIDERS.to_vec(),
                            one => vec![one],
                        };
                        this.change_hub(
                            HubChange::new("/claude/keepWarm/providers", json!(list)),
                            cx,
                        )
                    },
                    cx,
                ),
            ))
            .child(self.labelled(
                "When",
                self.segmented_enabled(
                    "keep-warm-mode",
                    vec![
                        ("auto", "When a window lapses".into()),
                        ("interval", "On an interval".into()),
                        ("daily", "Once a day".into()),
                    ],
                    match mode.as_str() {
                        "interval" => "interval",
                        "daily" => "daily",
                        _ => "auto",
                    },
                    settings_enabled,
                    |this, mode, _, cx| {
                        this.change_hub(HubChange::new("/claude/keepWarm/mode", mode), cx)
                    },
                    cx,
                ),
            ))
            .children(mode_detail)
            .children(self.hub_note(&["/claude/keepWarm"], Some(summary)))
    }

    pub(super) fn render_artifact_cleanup(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let writable = self.hub_writable();
        let cleanup = self.hub_value().map(|v| v["artifactCleanup"].clone());
        let enabled = self.hub_bool("/artifactCleanup/enabled") == Some(true);
        let min_age = cleanup
            .as_ref()
            .and_then(|c| c["minAgeHours"].as_f64())
            .unwrap_or(1.);
        let root = cleanup
            .as_ref()
            .and_then(|c| c["worktreeRoot"].as_str())
            .filter(|r| !r.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "~/.workspacer/worktrees".into());
        let summary = if enabled {
            (
                format!(
                    "On: every 15 minutes, worktrees under {root} idle for {} or more.",
                    hours_label(min_age)
                ),
                chrome::Tone::Info,
            )
        } else {
            (
                "Off: run `workspacer cleanup-agents` to clean up by hand.".to_owned(),
                chrome::Tone::Info,
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .debug_selector(|| "artifact-cleanup-switch".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.hub_switch(
                        "settings-artifact-cleanup",
                        "/artifactCleanup/enabled",
                        "/agents/artifactCleanup/enabled",
                        "Clean up stopped agents' build output",
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(scale::META))
                            .child("Clean up stopped agents' build output"),
                    ),
            )
            .child(self.labelled(
                "Idle for",
                self.segmented_enabled(
                    "artifact-cleanup-age",
                    hour_options(&MIN_AGES, min_age),
                    min_age,
                    writable && enabled,
                    |this, hours, _, cx| {
                        this.change_hub(
                            HubChange::new("/agents/artifactCleanup/minAgeHours", hours),
                            cx,
                        )
                    },
                    cx,
                ),
            ))
            .children(self.hub_note(&["/agents/artifactCleanup"], Some(summary)))
            .child(
                div()
                    .text_size(px(scale::CAPTION))
                    .text_color(rgb(p.muted))
                    .child("Only recognised output beside its project file is removed (node_modules next to package.json, target next to Cargo.toml, and similar). Worktrees, branches, source and tracked files are never touched."),
            )
    }

    pub(super) fn render_fleet_access(&self, cx: &mut Context<Self>) -> Div {
        let on = self.hub_bool("/fleetFullAccess") == Some(true);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "fleet-access-switch".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.hub_switch(
                        "settings-fleet-access",
                        "/fleetFullAccess",
                        "/agents/fleetFullAccess",
                        "Fleet workers start with full access",
                        cx,
                    )),
            )
            .children(self.hub_note(
                &["/agents/fleetFullAccess"],
                Some(if on {
                    (
                        "On: the Fleet Manager and its workers skip provider approval prompts."
                            .into(),
                        chrome::Tone::Warning,
                    )
                } else {
                    (
                        "Off: Fleet workers follow Child agents start with full access.".into(),
                        chrome::Tone::Info,
                    )
                }),
            ))
    }

    pub(super) fn render_manager_provider(&self, cx: &mut Context<Self>) -> Div {
        let current = self
            .hub_value()
            .and_then(|v| v["managerProvider"].as_str())
            .and_then(|id| MANAGER_PROVIDERS.iter().copied().find(|p| *p == id))
            .unwrap_or("claude");
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                self.segmented_enabled(
                    "manager-provider",
                    MANAGER_PROVIDERS
                        .iter()
                        .map(|id| (*id, provider_label(id).to_owned()))
                        .collect(),
                    current,
                    self.hub_writable(),
                    |this, provider, _, cx| {
                        this.change_hub(HubChange::new("/agents/managerProvider", provider), cx)
                    },
                    cx,
                ),
            )
            .children(self.hub_note(&["/agents/managerProvider"], None))
    }

    pub(super) fn render_startup_checks(&self, cx: &mut Context<Self>) -> Div {
        let row = |switch: Switch, label: &'static str, id: &'static str| {
            div()
                .debug_selector(move || id.into())
                .flex()
                .items_center()
                .gap_3()
                .child(switch)
                .child(div().text_size(px(scale::META)).child(label))
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(row(
                self.hub_switch(
                    "settings-poll-usage",
                    "/pollUsageOnBoot",
                    "/usage/pollOnBoot",
                    "Read account usage at startup",
                    cx,
                ),
                "Read account usage at startup (takes effect at the next start)",
                "poll-usage-switch",
            ))
            .child(row(
                self.hub_switch(
                    "settings-provider-check",
                    "/checkProviderOnStartup",
                    "/agents/checkProviderOnStartup",
                    "Check the Fleet Manager's provider at startup",
                    cx,
                ),
                "Check the Fleet Manager's provider at startup",
                "provider-check-switch",
            ))
            .children(self.hub_note(
                &["/usage/pollOnBoot", "/agents/checkProviderOnStartup"],
                None,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{hour_options, hours_label};

    #[test]
    fn hour_presets_keep_a_configured_value_they_do_not_list() {
        assert_eq!(hours_label(5.), "5 h");
        assert_eq!(hours_label(1.5), "1.5 h");
        assert_eq!(hours_label(24.), "1 day");
        let options = hour_options(&[1., 5.], 3.);
        assert_eq!(
            options.iter().map(|(h, _)| *h).collect::<Vec<_>>(),
            vec![1., 3., 5.]
        );
        assert_eq!(hour_options(&[1., 5.], 5.).len(), 2);
        assert_eq!(hour_options(&[1., 5.], f64::NAN).len(), 2);
    }
}
