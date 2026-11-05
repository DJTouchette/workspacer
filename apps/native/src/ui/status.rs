//! r4-truth (R4.1): the one derivation of a session's status. Every surface
//! (the header strip, the composer, the sidebar row and its group, History,
//! the Fleet tile and its counts, the command palette) reads a session's
//! state from [`derive`], with one priority order:
//!
//! Failed (unacknowledged, actionable) > Needs you > Not responding >
//! Needs you (a stalled subagent) > Working > Paused / Offline > Idle >
//! Done.
//!
//! r6-decisions (R6.2): a session whose own turn is healthy but one of
//! whose subagents has stopped answering is **Needs you**, not "Not
//! responding": the parent is fine, the user has to nudge, stop or look at
//! the child. Its reason names the child ([`stalled_child_reason`]: "Trace
//! session… not responding · 7m"). The child gets no entry of its own: the
//! parent's one status is the one source.
//!
//! Pure: `Workspace::session_status` gathers the [`Facts`] and the surfaces
//! draw what comes back. No surface decides a session's state on its own.
use super::attention::Bucket;
use super::primitives::{StatusBadge, StatusKind};
use std::collections::BTreeSet;
use wks_native::background_tasks::Task;

/// What one session's status is derived from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The hub is reachable; without it nothing below is current.
    pub connected: bool,
    /// The session has stopped (ended, or paused).
    pub stopped: bool,
    /// Stopped, but held open by the hub and resumable.
    pub paused: bool,
    /// An approval, a question or a plan waits on the user.
    pub needs_you: bool,
    /// Background work failed and no later run of it passed (nor was it
    /// dismissed): [`open_failures`] is not empty.
    pub failed: bool,
    /// It has itself gone quiet past the heartbeat threshold mid-turn.
    pub stalled: bool,
    /// One of its running subagents has gone quiet past the threshold.
    pub child_stalled: bool,
    /// Output is arriving now: never "Not responding".
    pub streaming: bool,
    /// Mid-turn or starting.
    pub working: bool,
    /// A subagent of it is at work (and answering).
    pub children_working: bool,
}

/// A session's status: its state word and glyph, and the sidebar group (and
/// Fleet section) it counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Status {
    pub kind: StatusKind,
    pub bucket: Bucket,
    /// Stopped on purpose: Idle, but it says "Ended".
    pub ended: bool,
}

impl Status {
    /// The word every surface shows for it.
    pub(crate) fn label(self) -> &'static str {
        if self.ended && self.kind == StatusKind::Idle {
            "Ended"
        } else {
            self.kind.label()
        }
    }

    /// As the shared v2 badge. r6-surfaces (R6.2): an ended session takes
    /// Ended's own glyph from the status table.
    pub(crate) fn badge(self) -> StatusBadge {
        if self.ended && self.kind == StatusKind::Idle {
            StatusBadge::new(StatusKind::Ended)
        } else {
            StatusBadge::new(self.kind)
        }
    }
}

/// The state alone, in the priority order above.
fn kind(f: &Facts) -> StatusKind {
    if !f.connected {
        StatusKind::Offline
    } else if f.stopped {
        if f.paused {
            StatusKind::Paused
        } else {
            StatusKind::Idle
        }
    } else if f.failed {
        StatusKind::Failed
    } else if f.needs_you {
        StatusKind::NeedsYou
    } else if f.stalled && !f.streaming {
        StatusKind::NotResponding
    } else if f.child_stalled {
        // R6.2: a stalled subagent asks for the user (nudge, stop, view).
        StatusKind::NeedsYou
    } else if f.working || f.streaming || f.children_working {
        StatusKind::Working
    } else {
        StatusKind::Idle
    }
}

/// The group a state counts in: everything that asks for the user (a
/// failure, a decision, a stall) is Needs you.
fn bucket_of(kind: StatusKind, stopped: bool) -> Bucket {
    match kind {
        StatusKind::Failed | StatusKind::NeedsYou | StatusKind::NotResponding => Bucket::NeedsYou,
        StatusKind::Working => Bucket::Working,
        StatusKind::Idle | StatusKind::Done if stopped => Bucket::Ended,
        _ => Bucket::Idle,
    }
}

/// The one derivation. Offline keeps the group the session's own state
/// puts it in (the list does not reshuffle while the hub is away); faults
/// it cannot see then are left out.
pub(crate) fn derive(f: Facts) -> Status {
    let kind = kind(&f);
    let bucket = if f.connected {
        bucket_of(kind, f.stopped)
    } else {
        let own = Facts {
            connected: true,
            failed: false,
            stalled: false,
            child_stalled: false,
            streaming: false,
            ..f
        };
        bucket_of(self::kind(&own), f.stopped)
    };
    Status {
        kind,
        bucket,
        ended: f.stopped && !f.paused,
    }
}

/// One command line, compared as typed: runs of spaces collapse.
fn command_key(task: &Task) -> String {
    task.title()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A failed task is resolved once a later run of the same command (same
/// kind) completed: "cargo test" failed, then passed.
fn resolved(failed: &Task, tasks: &[Task]) -> bool {
    let key = command_key(failed);
    let since = failed.ended_at.or(failed.started_at);
    tasks.iter().any(|later| {
        later.id != failed.id
            && later.kind == failed.kind
            && later.status == "completed"
            && command_key(later) == key
            && match (since, later.started_at.or(later.ended_at)) {
                (Some(since), Some(at)) => at >= since,
                // Times unknown: a passing run listed after it counts.
                _ => {
                    let at = |t: &Task| tasks.iter().position(|x| x.id == t.id);
                    at(later) > at(failed)
                }
            }
    })
}

/// The failures a session still owes the user: failed, not dismissed, and
/// not since passed. Newest first (by end, then start).
pub(crate) fn open_failures<'a>(
    tasks: &'a [Task],
    acknowledged: &BTreeSet<String>,
) -> Vec<&'a Task> {
    let mut failed: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.failed() && !acknowledged.contains(&t.id) && !resolved(t, tasks))
        .collect();
    failed.sort_by_key(|t| std::cmp::Reverse((t.ended_at, t.started_at)));
    failed
}

/// What a failure chip says: the command that broke and the work it belongs
/// to, "cargo test failed · Restore sessions".
pub(crate) fn failure_label(task: &Task, owner: &str, max: usize) -> String {
    let failed = wks_native::format::failed_label(&task.title(), max);
    let owner = short_title(owner, 18);
    if owner.is_empty() {
        failed
    } else {
        format!("{failed} · {owner}")
    }
}

/// r6-decisions (R6.2): why a parent with a stalled subagent needs you:
/// the subagent's name, cut short with an ellipsis, then how long it has
/// been silent ("Trace session… not responding · 7m").
pub(crate) fn stalled_child_reason(name: &str, quiet_ms: i64) -> String {
    const NAME: usize = 16;
    let name = name.trim();
    let short = short_title(name, NAME);
    let short = if short.chars().count() < name.chars().count() && !short.ends_with('…') {
        format!("{short}…")
    } else {
        short
    };
    format!(
        "{short} not responding · {}",
        super::attention::quiet_for(quiet_ms)
    )
}

/// A title cut at a word boundary to at most `max` characters ("Restore
/// sessions on launch" → "Restore sessions"); one long word is cut with an
/// ellipsis.
pub(crate) fn short_title(title: &str, max: usize) -> String {
    let title = title.trim();
    if title.chars().count() <= max {
        return title.to_owned();
    }
    let mut out = String::new();
    for word in title.split_whitespace() {
        let next = if out.is_empty() {
            word.to_owned()
        } else {
            format!("{out} {word}")
        };
        if next.chars().count() > max {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        let cut: String = title.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Facts {
        Facts {
            connected: true,
            ..Facts::default()
        }
    }

    #[test]
    fn the_priority_order_holds() {
        let all = Facts {
            failed: true,
            needs_you: true,
            stalled: true,
            working: true,
            children_working: true,
            ..live()
        };
        assert_eq!(derive(all).kind, StatusKind::Failed);
        let f = Facts {
            failed: false,
            ..all
        };
        assert_eq!(derive(f).kind, StatusKind::NeedsYou);
        let f = Facts {
            needs_you: false,
            ..f
        };
        assert_eq!(derive(f).kind, StatusKind::NotResponding);
        let f = Facts {
            stalled: false,
            ..f
        };
        assert_eq!(derive(f).kind, StatusKind::Working);
        let f = Facts {
            working: false,
            children_working: false,
            ..f
        };
        assert_eq!(derive(f).kind, StatusKind::Idle);
        for kind in [
            StatusKind::Failed,
            StatusKind::NeedsYou,
            StatusKind::NotResponding,
        ] {
            let facts = match kind {
                StatusKind::Failed => Facts {
                    failed: true,
                    ..live()
                },
                StatusKind::NeedsYou => Facts {
                    needs_you: true,
                    ..live()
                },
                _ => Facts {
                    stalled: true,
                    working: true,
                    ..live()
                },
            };
            assert_eq!(derive(facts).bucket, Bucket::NeedsYou, "{kind:?}");
        }
    }

    #[test]
    fn streaming_is_never_not_responding() {
        let f = Facts {
            stalled: true,
            working: true,
            streaming: true,
            ..live()
        };
        assert_eq!(derive(f).kind, StatusKind::Working);
        assert_eq!(derive(f).bucket, Bucket::Working);
    }

    #[test]
    fn a_stalled_subagent_puts_its_parent_in_needs_you() {
        let f = Facts {
            child_stalled: true,
            children_working: true,
            working: true,
            ..live()
        };
        let s = derive(f);
        assert_eq!((s.kind, s.bucket), (StatusKind::NeedsYou, Bucket::NeedsYou));
        // Its own silence is the louder state.
        let s = derive(Facts { stalled: true, ..f });
        assert_eq!(s.kind, StatusKind::NotResponding);
        // A decision outranks it (the same group either way).
        let s = derive(Facts {
            needs_you: true,
            ..f
        });
        assert_eq!(s.kind, StatusKind::NeedsYou);
        // Offline keeps the group its own state puts it in.
        let away = derive(Facts {
            connected: false,
            ..f
        });
        assert_eq!(away.bucket, Bucket::Working);
        assert_eq!(
            stalled_child_reason("Trace session restore paths", 7 * 60_000),
            "Trace session… not responding · 7m"
        );
        assert_eq!(
            stalled_child_reason("Trace", 60_000),
            "Trace not responding · 1m"
        );
    }

    #[test]
    fn working_subagents_keep_the_parent_working() {
        let f = Facts {
            children_working: true,
            ..live()
        };
        let s = derive(f);
        assert_eq!((s.kind, s.bucket), (StatusKind::Working, Bucket::Working));
    }

    #[test]
    fn stopped_sessions_are_ended_or_paused_and_offline_keeps_its_group() {
        let ended = derive(Facts {
            stopped: true,
            ..live()
        });
        assert_eq!(
            (ended.kind, ended.bucket, ended.label()),
            (StatusKind::Idle, Bucket::Ended, "Ended")
        );
        let paused = derive(Facts {
            stopped: true,
            paused: true,
            ..live()
        });
        assert_eq!(
            (paused.kind, paused.bucket),
            (StatusKind::Paused, Bucket::Idle)
        );
        let away = derive(Facts {
            connected: false,
            needs_you: true,
            failed: true,
            ..Facts::default()
        });
        assert_eq!(
            (away.kind, away.bucket),
            (StatusKind::Offline, Bucket::NeedsYou)
        );
        let away = derive(Facts {
            connected: false,
            stalled: true,
            working: true,
            ..Facts::default()
        });
        assert_eq!(away.bucket, Bucket::Working);
    }

    fn task(id: &str, status: &str, command: &str, at: i64) -> Task {
        Task {
            id: id.into(),
            kind: wks_native::background_tasks::Kind::of("local_bash"),
            status: status.into(),
            description: command.into(),
            started_at: Some(at),
            ended_at: Some(at + 10),
            ..Default::default()
        }
    }

    #[test]
    fn a_failure_clears_once_the_same_command_passes() {
        let acked = BTreeSet::new();
        let failed = task("a", "failed", "cargo test --locked", 100);
        assert_eq!(
            open_failures(std::slice::from_ref(&failed), &acked).len(),
            1
        );
        // Another command passing does not clear it.
        let other = task("b", "completed", "cargo clippy", 200);
        assert_eq!(open_failures(&[failed.clone(), other], &acked).len(), 1);
        // An earlier pass does not either.
        let before = task("c", "completed", "cargo test --locked", 10);
        assert_eq!(open_failures(&[before, failed.clone()], &acked).len(), 1);
        // A later pass of the same command does.
        let after = task("d", "completed", "cargo  test --locked", 300);
        assert!(open_failures(&[failed.clone(), after], &acked).is_empty());
        // Dismissed is gone too.
        let acked: BTreeSet<String> = ["a".to_owned()].into();
        assert!(open_failures(&[failed], &acked).is_empty());
    }

    #[test]
    fn failure_chips_name_their_task() {
        let failed = task("a", "failed", "cargo test --locked -- --test-threads=1", 1);
        assert_eq!(
            failure_label(&failed, "Restore sessions on launch", 28),
            "cargo test failed · Restore sessions"
        );
        assert_eq!(failure_label(&failed, "", 28), "cargo test failed");
        assert_eq!(
            short_title("Supercalifragilistic expialidocious", 10),
            "Supercali…"
        );
    }
}
