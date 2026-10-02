//! Server message timestamps and observed turn durations; never infer a finish
//! from the timestamp on the first streamed assistant fragment.
use crate::model::{Session, Transcript};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

pub fn history_path(settings: &Path, scope: &str, session: &str) -> PathBuf {
    settings
        .with_file_name("native-turn-timings")
        .join(format!("{:x}", Sha256::digest(scope.as_bytes())))
        .join(format!("{:x}.json", Sha256::digest(session.as_bytes())))
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn parse_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.timestamp_millis())
}

pub fn timestamp_label(timestamp: Option<i64>, now: i64, twelve_hour: bool) -> String {
    let Some(time) = timestamp.and_then(DateTime::<Utc>::from_timestamp_millis) else {
        return "Time unavailable".into();
    };
    let time = time.with_timezone(&Local);
    let today =
        DateTime::<Utc>::from_timestamp_millis(now).map(|t| t.with_timezone(&Local).date_naive());
    time.format(match (today == Some(time.date_naive()), twelve_hour) {
        (true, false) => "%H:%M:%S",
        (false, false) => "%b %d · %H:%M:%S",
        (true, true) => "%-I:%M:%S %p",
        (false, true) => "%b %d · %-I:%M:%S %p",
    })
    .to_string()
}

pub fn duration_label(milliseconds: i64) -> String {
    if milliseconds < 1000 {
        return format!("{}ms", milliseconds.max(0));
    }
    let seconds = milliseconds / 1000;
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedTurn {
    pub started_ms: i64,
    pub ended_ms: i64,
    pub stopped: bool,
}

impl CompletedTurn {
    pub fn label(&self) -> String {
        format!(
            "{} {}",
            if self.stopped {
                "Stopped after"
            } else {
                "Took"
            },
            duration_label(self.ended_ms - self.started_ms)
        )
    }
}

#[derive(Clone, Debug)]
struct ActiveTurn {
    started_ms: i64,
    resolved_start: bool,
    interrupted: bool,
    uncertain: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TurnClock {
    active: Option<ActiveTurn>,
    pub completed: VecDeque<CompletedTurn>,
    last_idle_ms: Option<i64>,
    uncertain_completion: bool,
}

impl TurnClock {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        anyhow::ensure!(
            path.metadata()?.len() <= 1024 * 1024,
            "Timing history is too large"
        );
        let mut completed: VecDeque<CompletedTurn> = serde_json::from_slice(&std::fs::read(path)?)?;
        completed.retain(|turn| turn.ended_ms >= turn.started_ms);
        while completed.len() > 64 {
            completed.pop_front();
        }
        Ok(Self {
            completed,
            ..Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut completed = Self::load(path)?.completed;
        for turn in &self.completed {
            if let Some(old) = completed
                .iter_mut()
                .find(|old| old.started_ms == turn.started_ms)
            {
                *old = turn.clone();
            } else {
                completed.push_back(turn.clone());
            }
        }
        completed
            .make_contiguous()
            .sort_by_key(|turn| turn.started_ms);
        while completed.len() > 64 {
            completed.pop_front();
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temp, serde_json::to_vec(&completed)?)?;
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
    }

    pub fn update(
        &mut self,
        session: &Session,
        transcript: Option<&Transcript>,
        connected: bool,
        now: i64,
    ) {
        if !connected {
            if let Some(active) = &mut self.active {
                active.uncertain = true;
            }
            return;
        }
        let waiting = session.approval.is_some()
            || session.questions.is_some()
            || matches!(
                session.state.as_str(),
                "approval" | "question" | "waiting_approval" | "waiting_input"
            );
        if session.working() || (waiting && !session.stopped()) {
            let active = self.active.get_or_insert(ActiveTurn {
                started_ms: now,
                resolved_start: false,
                interrupted: false,
                uncertain: false,
            });
            if !active.resolved_start
                && let Some(transcript) = transcript
            {
                let eligible = |row: &&std::sync::Arc<crate::model::Row>| {
                    row.role == "You"
                        && row.timestamp_ms.is_some_and(|t| {
                            t <= now && self.last_idle_ms.is_none_or(|idle| t >= idle)
                        })
                };
                let candidate = if self.last_idle_ms.is_some() {
                    transcript.rows.iter().find(eligible)
                } else {
                    transcript.rows.iter().rev().find(eligible)
                };
                // Some provider histories omit user timestamps. A recorded
                // assistant/tool event after that user message is still valid
                // evidence of when work was underway; never assign that time
                // to the user message itself.
                let timestamp = candidate.and_then(|row| row.timestamp_ms).or_else(|| {
                    let ix = transcript.rows.iter().rposition(|row| row.role == "You")?;
                    if transcript.rows[ix].timestamp_ms.is_some() {
                        return None;
                    }
                    transcript
                        .rows
                        .iter()
                        .skip(ix + 1)
                        .filter_map(|row| row.timestamp_ms)
                        .find(|t| *t <= now && self.last_idle_ms.is_none_or(|idle| *t >= idle))
                });
                if let Some(timestamp) = timestamp {
                    active.started_ms = active.started_ms.min(timestamp);
                }
                active.resolved_start = timestamp.is_some();
            }
        } else if session.stopped()
            || matches!(session.state.as_str(), "input" | "idle" | "background")
        {
            if let Some(active) = self.active.take() {
                self.uncertain_completion = active.uncertain;
                if !active.uncertain {
                    self.completed.push_back(CompletedTurn {
                        started_ms: active.started_ms,
                        ended_ms: now.max(active.started_ms),
                        stopped: active.interrupted || session.stopped(),
                    });
                    while self.completed.len() > 64 {
                        self.completed.pop_front();
                    }
                }
            }
            self.last_idle_ms = Some(now);
        }
    }

    pub fn interrupt(&mut self) {
        if let Some(active) = &mut self.active {
            active.interrupted = true;
        }
    }

    pub fn elapsed_label(&self, now: i64) -> Option<String> {
        self.active
            .as_ref()
            .map(|active| duration_label(now - active.started_ms))
    }

    pub fn latest_completion(&self) -> Option<String> {
        (!self.uncertain_completion)
            .then(|| self.completed.back().map(CompletedTurn::label))
            .flatten()
    }

    pub fn latest_completion_for(&self, transcript: &Transcript) -> Option<String> {
        let turn = self.completed.back()?;
        let latest_user = transcript
            .rows
            .iter()
            .rev()
            .find(|row| row.role == "You")
            .and_then(|row| row.timestamp_ms);
        if latest_user.is_some_and(|timestamp| timestamp > turn.ended_ms) {
            return None;
        }
        self.latest_completion()
    }

    pub fn message_labels(&self, transcript: &Transcript) -> HashMap<u64, String> {
        self.completed
            .iter()
            .filter_map(|turn| {
                transcript
                    .rows
                    .iter()
                    .rev()
                    .find(|row| {
                        row.role == "Assistant"
                            && row
                                .timestamp_ms
                                .is_some_and(|t| t >= turn.started_ms && t <= turn.ended_ms)
                    })
                    .map(|row| (row.key, turn.label()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConversationSnapshot, Item};
    fn transcript() -> Transcript {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: vec![
                Item {
                    kind: "user_message".into(),
                    text: "Work".into(),
                    timestamp: Some("2026-09-27T12:00:00Z".into()),
                    ..Default::default()
                },
                Item {
                    kind: "assistant_text".into(),
                    text: "Done".into(),
                    timestamp: Some("2026-09-27T12:01:12Z".into()),
                    ..Default::default()
                },
            ],
        });
        t
    }

    #[test]
    fn timestamps_parse_offsets_and_durations_use_minutes_seconds() {
        assert_eq!(
            parse_timestamp("2026-09-27T06:00:00-06:00"),
            parse_timestamp("2026-09-27T12:00:00Z")
        );
        assert_eq!(parse_timestamp("invalid"), None);
        assert_eq!(duration_label(72_999), "1m 12s");
        assert_eq!(duration_label(-100), "0ms");
        assert_eq!(duration_label(999), "999ms");
        assert_eq!(duration_label(1000), "1s");
        let now = now_ms();
        assert!(
            timestamp_label(Some(now), now, true).ends_with("AM")
                || timestamp_label(Some(now), now, true).ends_with("PM")
        );
        assert!(!timestamp_label(Some(now), now, false).contains("AM"));
        assert!(timestamp_label(Some(now - 172_800_000), now, true).contains(" · "));
        assert_eq!(timestamp_label(None, now, true), "Time unavailable");
        assert_eq!(duration_label(3_661_000), "61m 01s");
    }

    #[test]
    fn timer_does_not_restart_for_queued_messages_or_approval_and_freezes_on_done() {
        let base = parse_timestamp("2026-09-27T12:00:00Z").unwrap();
        let mut t = transcript();
        let mut clock = TurnClock::default();
        let mut session = Session {
            state: "responding".into(),
            ..Default::default()
        };
        clock.update(&session, Some(&t), true, base + 20_000);
        assert_eq!(clock.elapsed_label(base + 25_000).as_deref(), Some("25s"));
        t.delta(
            crate::model::Delta {
                seq: 3,
                items: vec![Item {
                    kind: "user_message".into(),
                    text: "Queued".into(),
                    timestamp: Some("2026-09-27T12:00:30Z".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            false,
        );
        session.state = "approval".into();
        clock.update(&session, Some(&t), true, base + 30_000);
        session.state = "responding".into();
        clock.update(&session, Some(&t), true, base + 40_000);
        session.state = "input".into();
        clock.update(&session, Some(&t), true, base + 73_000);
        clock.update(&session, Some(&t), true, base + 100_000);
        assert_eq!(clock.latest_completion().as_deref(), Some("Took 1m 13s"));
        assert!(clock.elapsed_label(base + 200_000).is_none());
        assert_eq!(clock.message_labels(&t).len(), 1);
    }

    #[test]
    fn attaching_with_an_unstamped_prompt_uses_recorded_work_without_faking_message_time() {
        let base = parse_timestamp("2026-09-27T12:00:00Z").unwrap();
        let mut t = transcript();
        std::sync::Arc::make_mut(&mut t.rows[0]).timestamp_ms = None;
        let mut clock = TurnClock::default();
        let session = Session {
            state: "working".into(),
            ..Default::default()
        };
        clock.update(&session, None, true, base + 100_000);
        clock.update(&session, Some(&t), true, base + 101_000);
        assert_eq!(clock.elapsed_label(base + 102_000).as_deref(), Some("30s"));
        assert!(t.rows[0].timestamp_ms.is_none());
    }

    #[test]
    fn completed_durations_survive_reload_and_are_scoped_to_the_hub() {
        let root = std::env::temp_dir().join(format!(
            "native-timing-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = history_path(
            &root.join("settings.json"),
            "ws://localhost:7895/bus",
            "../session",
        );
        assert_ne!(
            path,
            history_path(
                &root.join("settings.json"),
                "ws://other:7895/bus",
                "../session"
            )
        );
        let mut clock = TurnClock::default();
        clock.completed.push_back(CompletedTurn {
            started_ms: 1000,
            ended_ms: 73_000,
            stopped: false,
        });
        clock.save(&path).unwrap();
        let loaded = TurnClock::load(&path).unwrap();
        assert_eq!(loaded.latest_completion().as_deref(), Some("Took 1m 12s"));
        assert!(loaded.active.is_none());
        loaded.save(&path).unwrap();
        assert_eq!(TurnClock::load(&path).unwrap().completed.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_idle_history_and_disconnects_do_not_invent_completion_durations() {
        let base = parse_timestamp("2026-09-27T12:00:00Z").unwrap();
        let t = transcript();
        let mut clock = TurnClock::default();
        let mut session = Session {
            state: "input".into(),
            ..Default::default()
        };
        clock.update(&session, Some(&t), true, base + 100_000);
        assert!(clock.latest_completion().is_none());
        session.state = "working".into();
        clock.update(&session, Some(&t), true, base + 110_000);
        assert_eq!(clock.elapsed_label(base + 112_000).as_deref(), Some("2s"));
        clock.update(&session, None, false, base + 120_000);
        session.state = "idle".into();
        clock.update(&session, Some(&t), true, base + 200_000);
        assert!(clock.latest_completion().is_none());
    }
}
