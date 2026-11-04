//! When a queue-started agent gives its slot back.
//!
//! The hub's roster publishes `agent.snapshot` on every session change (the
//! projection in `snapshots::compat`). The fields read here:
//!
//! - `status == "ended"` / `mode == "stopped"`: the session is over.
//! - `ambientState`: `streaming` (claudemon mode `responding`), `background`
//!   (idle prompt with background tasks still running), `waiting_approval`
//!   (`approval`), `waiting_input` (`question`), `idle` (`input`). Absent while
//!   the provider is still starting (mode `unknown`).
//!
//! A spawned session sits at its idle prompt before the queued prompt reaches
//! it, so `idle` releases the slot only after the agent has been seen working.
//! Waiting on a person (approval or a question) releases at once: an agent
//! that waits for the user does not hold a slot. When no event is seen (a
//! lagged subscription, or the turn ran between the spawn returning and the
//! watch being armed) the 60 s re-check (`recheck`) is the backstop.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Signal {
    /// Nothing to decide yet.
    Wait,
    /// The agent is working on its prompt.
    Working,
    /// The slot is released; the item ends `done` or `failed` with `detail`.
    Release { failed: bool, detail: String },
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}

/// Classify one snapshot row of the watched session.
pub fn classify(row: &Value, seen_working: bool) -> Signal {
    if text(row, "status") == "ended" || text(row, "mode") == "stopped" {
        let error = ["error", "lastError"]
            .iter()
            .map(|key| text(row, key).trim())
            .find(|e| !e.is_empty());
        return match error {
            Some(error) => Signal::Release {
                failed: true,
                detail: format!("session ended with an error: {error}"),
            },
            None => Signal::Release {
                failed: false,
                detail: "session ended".into(),
            },
        };
    }
    match text(row, "ambientState") {
        "streaming" | "thinking" | "background" => Signal::Working,
        "waiting_approval" => Signal::Release {
            failed: false,
            detail: "waiting for approval".into(),
        },
        "waiting_input" => Signal::Release {
            failed: false,
            detail: "waiting for input".into(),
        },
        "idle" if seen_working => Signal::Release {
            failed: false,
            detail: "turn finished".into(),
        },
        _ => Signal::Wait,
    }
}

/// How many consecutive re-checks may find the session idle-without-work, or
/// missing, before the slot is released anyway.
pub const RECHECK_LIMIT: u32 = 2;

/// The periodic re-check of one watched session. `row` is the
/// `sessions.snapshot` answer (`None` when the session is not reported).
/// `strikes` counts consecutive inconclusive re-checks; the new count is
/// returned with the signal.
pub fn recheck(row: Option<&Value>, seen_working: bool, strikes: u32) -> (Signal, u32) {
    let Some(row) = row.filter(|row| row.is_object()) else {
        let strikes = strikes + 1;
        return if strikes >= RECHECK_LIMIT {
            (
                Signal::Release {
                    failed: false,
                    detail: "session is no longer reported".into(),
                },
                strikes,
            )
        } else {
            (Signal::Wait, strikes)
        };
    };
    match classify(row, seen_working) {
        Signal::Wait if text(row, "ambientState") == "idle" => {
            // Idle at two checks a minute apart without any observed work:
            // the turn ran unseen (or the prompt never arrived). Either way
            // the agent is not using the machine.
            let strikes = strikes + 1;
            if strikes >= RECHECK_LIMIT {
                (
                    Signal::Release {
                        failed: false,
                        detail: "idle at re-check".into(),
                    },
                    strikes,
                )
            } else {
                (Signal::Wait, strikes)
            }
        }
        Signal::Wait => (Signal::Wait, 0),
        signal => (signal, 0),
    }
}
