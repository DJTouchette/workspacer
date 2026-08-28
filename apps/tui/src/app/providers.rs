//! Which harnesses this machine can actually launch — the app half of
//! [`crate::providers`].
//!
//! The spawn modal and the handoff picker used to cycle a hardcoded list of all
//! five harnesses whether or not their CLI existed, so four of the five choices
//! were spawn failures waiting to happen. The hub answers this
//! (`providers.checkAll`), and the desktop pickers already filter on it; this
//! is the same rule for the TUI.

use super::*;

impl App {
    /// The harnesses a provider picker should offer right now: the installed
    /// ones, plus `keep` (the form's current pick, or a handoff's source
    /// harness) so a picker can never render with its own value missing. With
    /// no detection answer this is every harness — see [`crate::providers`].
    pub fn offered_providers(&self, keep: Option<&str>) -> Vec<&'static str> {
        crate::providers::offered(self.installed_providers.as_deref(), keep)
    }

    /// Re-probe `providers.checkAll`. Fired when a provider picker opens, so
    /// installing a CLI mid-session is picked up without restarting the TUI.
    /// No-op without a bus (`--direct` claudemon mode), which leaves the list
    /// unanswered and every harness on offer.
    pub(in crate::app) fn probe_providers(&self) {
        let Some(bus) = self.bus.clone() else { return };
        tokio::spawn(crate::providers::probe(bus, self.tx.clone()));
    }
}
