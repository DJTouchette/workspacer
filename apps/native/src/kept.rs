//! The one place that decides which sessions this device keeps in the sidebar
//! beyond what the hub's fleet list shows.
//!
//! The sidebar is the hub's fleet (live sessions, plus stopped ones for a
//! day, read uncurated so another client's layout does not decide) and, on top, the sessions this device was
//! in the middle of when the app closed (`Settings::kept_open`, by hub
//! identity). The controller reads kept ids the fleet left out by id, and from
//! the hub's history once the engine has forgotten them. Archive and cleared
//! children only hide rows; they never add one.
//!
//! A kept entry is stamped with when this device last saw the session open.
//! It is let go when the session ends while the app watches it, is archived,
//! cannot be produced by the hub at all, or was last seen open more than
//! [`KEEP_FOR_MS`] ago. When the hub outlives the app (attached, not
//! `--local`), one that stopped after the app last quit finished on its own
//! while you were away, and is let go rather than shown as paused.
use crate::model::Session;
use std::collections::{BTreeMap, BTreeSet};

/// How long a kept session outlives the last time this device saw it open:
/// the engine's own archive window.
pub const KEEP_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// An open session's stamp is refreshed at most this often, so a long-running
/// session does not rewrite settings on every update.
pub const REFRESH_MS: i64 = 60 * 60 * 1000;
/// The most kept sessions handed to the controller to read by id.
pub const MAX_READ: usize = 64;
/// How long after the app quit a session may still stop and count as stopped
/// by the quit (the backend joins after the window's own quit hook).
pub const CLOSE_GRACE_MS: i64 = 2 * 60 * 1000;

/// Fold one fleet view into `kept` (id → last seen open, epoch ms).
///
/// `seen_open` is this run's memory of sessions seen open: one that stops
/// after being seen open was ended on purpose (by you or its agent) and is
/// let go; one already stopped when first seen this run was stopped by the
/// app closing and stays, paused. `gone` are ids the hub could not produce.
///
/// `closed_at` is when the app last quit, given only when the hub outlives
/// the app: a kept session first seen stopped whose last activity is after
/// that (plus [`CLOSE_GRACE_MS`]) was not stopped by the quit. With no close
/// on record (a crash, an owned backend) every such session stays paused.
#[allow(clippy::too_many_arguments)]
pub fn fold(
    kept: &mut BTreeMap<String, i64>,
    sessions: &[Session],
    seen_open: &mut BTreeSet<String>,
    gone: &[String],
    archived: impl Fn(&str) -> bool,
    closed_at: Option<i64>,
    now_ms: i64,
) {
    for session in sessions {
        if session.stopped() {
            let finished_away = closed_at.is_some_and(|closed| {
                session.last_activity > closed.saturating_add(CLOSE_GRACE_MS)
            });
            if seen_open.contains(&session.id) || finished_away {
                kept.remove(&session.id);
            }
        } else {
            seen_open.insert(session.id.clone());
            let stamp = kept.entry(session.id.clone()).or_insert(now_ms);
            if now_ms - *stamp >= REFRESH_MS {
                *stamp = now_ms;
            }
        }
    }
    kept.retain(|id, &mut seen| {
        !archived(id) && !gone.contains(id) && now_ms - seen <= KEEP_FOR_MS
    });
}

/// The kept ids the controller should be able to read, most recently seen
/// open first, at most [`MAX_READ`]: past the cap the oldest drop off, never
/// an arbitrary few.
pub fn read_order(kept: &BTreeMap<String, i64>) -> Vec<String> {
    let mut ids: Vec<(&String, i64)> = kept.iter().map(|(id, &at)| (id, at)).collect();
    ids.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ids.into_iter()
        .take(MAX_READ)
        .map(|(id, _)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, state: &str) -> Session {
        Session {
            id: id.into(),
            state: state.into(),
            ..Default::default()
        }
    }

    #[test]
    fn open_sessions_are_kept_and_let_go_when_they_end_while_watched() {
        let mut kept = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let fleet = [session("a", "input"), session("b", "responding")];
        fold(&mut kept, &fleet, &mut seen, &[], |_| false, None, 1_000);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["a", "b"]);
        let fleet = [session("a", "input"), session("b", "stopped")];
        fold(&mut kept, &fleet, &mut seen, &[], |_| false, None, 2_000);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["a"]);
        // A new run first sees `a` stopped: the close stopped it, it stays.
        let mut seen = BTreeSet::new();
        fold(
            &mut kept,
            &[session("a", "stopped")],
            &mut seen,
            &[],
            |_| false,
            None,
            3_000,
        );
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn a_session_that_finished_after_the_app_quit_is_not_paused() {
        let closed = 10_000_000;
        let stopped = |id: &str, at: i64| Session {
            last_activity: at,
            ..session(id, "stopped")
        };
        let fleet = [
            stopped("by-quit", closed + CLOSE_GRACE_MS),
            stopped("while-away", closed + CLOSE_GRACE_MS + 1),
            stopped("unknown", 0),
        ];
        let start = || {
            BTreeMap::from([
                ("by-quit".to_string(), 1),
                ("while-away".to_string(), 1),
                ("unknown".to_string(), 1),
            ])
        };
        let mut kept = start();
        fold(
            &mut kept,
            &fleet,
            &mut BTreeSet::new(),
            &[],
            |_| false,
            Some(closed),
            20_000_000,
        );
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["by-quit", "unknown"]);
        // No close on record (a crash, or an app that owns its backend).
        let mut kept = start();
        fold(
            &mut kept,
            &fleet,
            &mut BTreeSet::new(),
            &[],
            |_| false,
            None,
            20_000_000,
        );
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn kept_sessions_expire_unless_seen_open_again() {
        let mut kept = BTreeMap::from([("old".to_string(), 0), ("open".to_string(), 0)]);
        let mut seen = BTreeSet::new();
        let now = KEEP_FOR_MS + 1;
        let fleet = [session("old", "stopped"), session("open", "input")];
        fold(&mut kept, &fleet, &mut seen, &[], |_| false, None, now);
        assert_eq!(kept, BTreeMap::from([("open".to_string(), now)]));
    }

    #[test]
    fn stamps_refresh_at_most_hourly() {
        let mut kept = BTreeMap::from([("a".to_string(), 0)]);
        let mut seen = BTreeSet::new();
        fold(
            &mut kept,
            &[session("a", "input")],
            &mut seen,
            &[],
            |_| false,
            None,
            REFRESH_MS - 1,
        );
        assert_eq!(kept["a"], 0);
        fold(
            &mut kept,
            &[session("a", "input")],
            &mut seen,
            &[],
            |_| false,
            None,
            REFRESH_MS,
        );
        assert_eq!(kept["a"], REFRESH_MS);
    }

    #[test]
    fn archived_and_gone_sessions_are_let_go() {
        let mut kept = BTreeMap::from([
            ("archived".to_string(), 5),
            ("gone".to_string(), 5),
            ("kept".to_string(), 5),
        ]);
        fold(
            &mut kept,
            &[],
            &mut BTreeSet::new(),
            &["gone".to_string()],
            |id| id == "archived",
            None,
            10,
        );
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["kept"]);
    }

    #[test]
    fn the_read_cap_keeps_the_most_recent_not_the_first_ids() {
        let kept: BTreeMap<String, i64> = (0..MAX_READ as i64 + 10)
            .map(|n| (format!("{:03}", 999 - n), n))
            .collect();
        let ids = read_order(&kept);
        assert_eq!(ids.len(), MAX_READ);
        // Newest stamp first; the ten oldest (highest ids here) are dropped.
        assert_eq!(ids[0], format!("{:03}", 999 - (MAX_READ as i64 + 9)));
        assert!(!ids.contains(&"999".to_string()));
    }
}
