//! Which agent CLIs exist on the host — the TUI's side of the same detection
//! the desktop pickers use.
//!
//! Workspacer speaks five harnesses (claude/codex/copilot/opencode/pi) and most
//! machines have one or two installed; a spawn modal that cycles through all
//! five is cycling through four spawn failures. The hub already answers this:
//! `providers.checkAll` (services/hub/cmd/brain/providers.go, the Go twin of
//! the desktop's agentProviders.ts) returns one `{provider, found, ...}` row per
//! harness, honouring the `agents.binaries` overrides.
//!
//! Two rules, matching the desktop (renderer lib/providerAvailability.ts):
//!
//!  - **Fail open.** No answer — `--direct` claudemon mode, an older hub, a
//!    scope that can't call it — leaves the list `None`, which means "offer
//!    everything". Hiding a harness on ignorance would remove a working backend
//!    with no way for the user to see why.
//!  - **Never vanish what's in use.** A caller may pin one provider (the
//!    session being handed off) that stays listed even when it isn't detected.

use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppMsg;
use crate::bus::BusClient;

/// Every harness the spawn modal / handoff picker knows how to launch.
pub const ALL_PROVIDERS: &[&str] = &["claude", "codex", "copilot", "opencode", "pi"];

/// Provider ids reported as `found` by a `providers.checkAll` answer.
pub fn installed_from_rows(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| r.get("found").and_then(|f| f.as_bool()).unwrap_or(false))
                .filter_map(|r| r.get("provider").and_then(|p| p.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The harnesses a picker should offer.
///
/// `installed` is `None` when detection has not answered (offer everything).
/// `keep` is a provider that must stay listed regardless — the source harness
/// of a handoff, or whatever the form currently has selected.
pub fn offered<'a>(installed: Option<&[String]>, keep: Option<&str>) -> Vec<&'a str> {
    let Some(found) = installed else {
        return ALL_PROVIDERS.to_vec();
    };
    let offered: Vec<&str> = ALL_PROVIDERS
        .iter()
        .copied()
        .filter(|p| found.iter().any(|f| f == p) || keep == Some(p))
        .collect();
    // An answer that found nothing at all is not a usable picker — treat it the
    // same as no answer rather than rendering an empty modal.
    if offered.is_empty() {
        return ALL_PROVIDERS.to_vec();
    }
    offered
}

/// Ask the hub which CLIs are installed and post the answer back.
///
/// Silent on failure: a hub that doesn't serve the capability, or a scope that
/// can't reach it, leaves the app's list untouched (still `None` = offer all).
pub async fn probe(bus: BusClient, tx: UnboundedSender<AppMsg>) {
    if let Ok(v) = bus.call("providers.checkAll", json!({})).await {
        let _ = tx.send(AppMsg::InstalledProviders(installed_from_rows(&v)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_found_rows() {
        let v = json!([
            { "provider": "claude", "found": true, "resolvedPath": "/usr/bin/claude" },
            { "provider": "codex", "found": false, "resolvedPath": null },
            { "provider": "pi", "found": true, "resolvedPath": "/usr/bin/pi" },
        ]);
        assert_eq!(installed_from_rows(&v), vec!["claude", "pi"]);
    }

    #[test]
    fn no_answer_offers_everything() {
        assert_eq!(offered(None, None), ALL_PROVIDERS.to_vec());
    }

    #[test]
    fn hides_what_is_not_installed() {
        let found = vec!["claude".to_string(), "codex".to_string()];
        assert_eq!(offered(Some(&found), None), vec!["claude", "codex"]);
    }

    #[test]
    fn keeps_the_pinned_provider_even_when_missing() {
        let found = vec!["claude".to_string()];
        assert_eq!(offered(Some(&found), Some("opencode")), vec!["claude", "opencode"]);
    }

    #[test]
    fn an_empty_answer_is_treated_as_no_answer() {
        // A hub that answers with nothing found would otherwise render a modal
        // with no provider at all, which cannot be recovered from on screen.
        assert_eq!(offered(Some(&[]), None), ALL_PROVIDERS.to_vec());
    }
}
