//! A session's hub-owned outbound message queue, as its snapshot row reports
//! it, and the `sessions.queue*` changes the native client asks for.
//!
//! The hub owns the queue (`services::message_queue` in hub-rs): it sends the
//! head entry once a turn ends, never over an open approval or question, and
//! pauses on a refused send. A send whose delivery is unknown is never
//! retried; it is reported as `queueUncertain` until a person dismisses it.
//! Every client (native, web, phone) edits the same queue, so this module
//! only mirrors the row and names the calls; it keeps no queue of its own.
use serde_json::{Value, json};

/// Entries shown per session (the hub holds at most 50).
pub const MAX_ENTRIES: usize = 50;
/// Bytes of one entry's text the hub accepts and the client keeps.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Characters of an uncertain entry's text the hub repeats on the row. A
/// preview this long may have been cut, so it is never sent again as is.
pub const UNCERTAIN_PREVIEW_CHARS: usize = 200;
/// Bytes of a pause reason kept.
const MAX_ERROR_BYTES: usize = 2_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub text: String,
    /// Epoch milliseconds.
    pub created_at: Option<i64>,
}

/// A send that may or may not have reached the agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uncertain {
    pub id: String,
    /// The hub's preview of the text (the first 200 characters).
    pub text: String,
    /// Epoch milliseconds.
    pub at: Option<i64>,
}

impl Uncertain {
    /// The preview is the whole message: short enough that nothing was cut.
    pub fn complete(&self) -> bool {
        self.text.chars().count() < UNCERTAIN_PREVIEW_CHARS
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queue {
    pub entries: Vec<Entry>,
    /// The hub stopped sending (a refused send or an uncertain one) until a
    /// person changes the queue.
    pub paused: bool,
    pub error: Option<String>,
    pub uncertain: Option<Uncertain>,
}

impl Queue {
    /// Fold a snapshot row in. The hub puts `queuedMessages` on every row of
    /// a session that ever had a queue (empty once drained) and omits
    /// `queueError` / `queueUncertain` when they clear, so a row carrying the
    /// list is the whole state. Rows without it (sessions that never queued,
    /// state-only reconciliations) leave the last state alone.
    pub fn merge(&mut self, row: &Value) {
        let Some(list) = row.get("queuedMessages") else {
            return;
        };
        self.entries = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let id = entry["id"].as_str().filter(|id| !id.is_empty())?;
                Some(Entry {
                    id: crate::transcript::head(id, 128),
                    text: crate::transcript::head(entry["text"].as_str()?, MAX_TEXT_BYTES),
                    created_at: entry["createdAt"].as_i64(),
                })
            })
            .take(MAX_ENTRIES)
            .collect();
        self.paused = row["queuePaused"].as_bool().unwrap_or(false);
        self.error = row["queueError"]
            .as_str()
            .map(str::trim)
            .filter(|error| !error.is_empty())
            .map(|error| crate::transcript::head(error, MAX_ERROR_BYTES));
        self.uncertain = row.get("queueUncertain").and_then(|value| {
            let id = value["id"].as_str().filter(|id| !id.is_empty())?;
            Some(Uncertain {
                id: crate::transcript::head(id, 128),
                text: crate::transcript::head(value["text"].as_str().unwrap_or(""), 4 * 1024),
                at: value["at"].as_i64(),
            })
        });
    }

    /// Something for the queue panel to show.
    pub fn visible(&self) -> bool {
        !self.entries.is_empty() || self.paused || self.uncertain.is_some()
    }
}

/// One change to a session's queue: a `sessions.queue*` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add(String),
    /// Replace an entry's text. With `expected` (the text the edit started
    /// from), the hub refuses the change when the entry was changed
    /// elsewhere meanwhile, so an edit never overwrites someone else's.
    Update {
        id: String,
        text: String,
        expected: Option<String>,
    },
    /// Removes an entry, or acknowledges the uncertain one by its id.
    Remove(String),
    Move {
        id: String,
        index: usize,
    },
    /// Sends the entry now, interrupting or setting aside current work
    /// (`delivery: "now"`). It also lifts a pause.
    SendNow(String),
    Clear,
    /// Lifts a pause (`sessions.queueResume`). Nothing else resumes sending:
    /// adding, editing or reordering a paused queue keeps it paused.
    Resume,
    /// "Send again" for an uncertain send whose preview is the whole text:
    /// queue the text anew, then dismiss the uncertain record. Two calls, in
    /// that order, so a refused add never loses the record.
    Resend {
        uncertain: String,
        text: String,
    },
}

impl Op {
    /// The bus call for this change. `Resend` names its first call (the
    /// add); the backend follows it with the dismissal.
    pub fn wire(&self, session: &str) -> (&'static str, Value) {
        match self {
            Self::Add(text) | Self::Resend { text, .. } => (
                "sessions.queueAdd",
                json!({"sessionId":session,"text":text}),
            ),
            Self::Update { id, text, expected } => {
                let mut params = json!({"sessionId":session,"id":id,"text":text});
                if let Some(expected) = expected {
                    params["expectedText"] = json!(expected);
                }
                ("sessions.queueUpdate", params)
            }
            Self::Remove(id) => ("sessions.queueRemove", json!({"sessionId":session,"id":id})),
            Self::Move { id, index } => (
                "sessions.queueMove",
                json!({"sessionId":session,"id":id,"index":index}),
            ),
            Self::SendNow(id) => (
                "sessions.queueSendNow",
                json!({"sessionId":session,"id":id,"delivery":"now"}),
            ),
            Self::Clear => ("sessions.queueClear", json!({"sessionId":session})),
            Self::Resume => ("sessions.queueResume", json!({"sessionId":session})),
        }
    }

    /// The text this change puts on the queue, if any.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Add(text) | Self::Update { text, .. } | Self::Resend { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Changes that put new text on the queue need a live session; tidying
    /// (remove, reorder, clear) also works on a stopped one, whose queue the
    /// hub keeps until it resumes or is cleared.
    pub fn needs_live_session(&self) -> bool {
        matches!(
            self,
            Self::Add(_) | Self::Update { .. } | Self::SendNow(_) | Self::Resend { .. }
        )
    }

    /// What the notice says when the hub refuses this change (`error` is
    /// its reason). A call that timed out says its outcome is unknown, and
    /// nothing is retried for the user.
    pub fn failure(&self, error: &str) -> String {
        if error.contains("outcome unknown") || error.contains("outcome is unknown") {
            let what = match self {
                Self::Add(_) => "Queueing the message",
                Self::Resend { .. } => "Queueing the message again",
                Self::SendNow(_) => "Sending the queued message",
                _ => "The queue change",
            };
            return format!(
                "{what} timed out; its outcome is unknown. Check the queue and the conversation before trying again."
            );
        }
        // The backend's own wording for a half-done resend already says
        // what happened.
        if error.starts_with("Queued again") {
            return error.to_owned();
        }
        let what = match self {
            Self::Add(_) => "Could not queue the message",
            Self::Resend { .. } => {
                "Could not queue the message again; the may-have-been-sent notice stays"
            }
            Self::SendNow(_) => "Could not send the queued message",
            Self::Resume => "Could not resume the queue",
            _ => "Queue change failed",
        };
        format!("{what}: {error}")
    }

    /// What the notice says when the change went through; most changes show
    /// in the panel itself and say nothing.
    pub fn success(&self) -> &'static str {
        match self {
            Self::Resend { .. } => "Queued again",
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_with_the_list_carry_the_whole_queue_and_others_keep_it() {
        let mut queue = Queue::default();
        queue.merge(&json!({"sessionId":"a","label":"no queue yet"}));
        assert!(
            !queue.visible(),
            "a session that never queued shows nothing"
        );
        queue.merge(&json!({
            "queuedMessages":[
                {"id":"q1","text":"first","createdAt":1},
                {"id":"","text":"no id"},
                {"id":"q2","text":"second","createdAt":2}
            ],
            "queuePaused":true,
            "queueError":"refused: session busy",
            "queueUncertain":{"id":"q0","text":"maybe","at":3}
        }));
        assert_eq!(
            queue
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["q1", "q2"]
        );
        assert!(queue.paused);
        assert_eq!(queue.error.as_deref(), Some("refused: session busy"));
        assert_eq!(queue.uncertain.as_ref().map(|u| u.id.as_str()), Some("q0"));
        queue.merge(&json!({"sessionId":"a","mode":"responding"}));
        assert_eq!(queue.entries.len(), 2, "a sparse row keeps the queue");
        // The hub omits the error and uncertain keys once they clear.
        queue.merge(&json!({"queuedMessages":[],"queuePaused":false}));
        assert_eq!(queue, Queue::default());
        assert!(!queue.visible());
    }

    #[test]
    fn calls_name_the_hub_methods_and_send_now_interrupts() {
        let (method, params) = Op::Move {
            id: "q2".into(),
            index: 0,
        }
        .wire("a");
        assert_eq!(method, "sessions.queueMove");
        assert_eq!(params, json!({"sessionId":"a","id":"q2","index":0}));
        let (method, params) = Op::SendNow("q1".into()).wire("a");
        assert_eq!(method, "sessions.queueSendNow");
        assert_eq!(params["delivery"], "now");
        let (method, params) = Op::Resend {
            uncertain: "q0".into(),
            text: "again".into(),
        }
        .wire("a");
        assert_eq!(method, "sessions.queueAdd", "a resend queues first");
        assert_eq!(params, json!({"sessionId":"a","text":"again"}));
        assert!(!Op::Clear.needs_live_session());
        assert!(Op::Add("x".into()).needs_live_session());
    }

    #[test]
    fn notices_name_the_change_and_never_contradict_themselves() {
        let resend = Op::Resend {
            uncertain: "q0".into(),
            text: "again".into(),
        };
        assert_eq!(resend.success(), "Queued again");
        assert_eq!(
            resend.failure("message queue is full (50 entries)"),
            "Could not queue the message again; the may-have-been-sent notice stays: message queue is full (50 entries)"
        );
        let half = "Queued again, but the may-have-been-sent notice was not dismissed (busy)";
        assert_eq!(resend.failure(half), half);
        let unknown =
            resend.failure("Request timed out; outcome unknown. Check the session before retrying");
        assert!(
            unknown.starts_with("Queueing the message again timed out; its outcome is unknown")
        );
        assert!(!unknown.contains("Could not"));
        assert_eq!(Op::Clear.success(), "");
        let update = Op::Update {
            id: "q1".into(),
            text: "mine".into(),
            expected: Some("start".into()),
        };
        assert_eq!(update.wire("a").1["expectedText"], "start");
        assert_eq!(Op::Resume.wire("a").0, "sessions.queueResume");
    }

    #[test]
    fn only_a_short_uncertain_preview_is_the_whole_message() {
        let short = Uncertain {
            text: "short".into(),
            ..Default::default()
        };
        assert!(short.complete());
        let cut = Uncertain {
            text: "é".repeat(UNCERTAIN_PREVIEW_CHARS),
            ..Default::default()
        };
        assert!(!cut.complete());
    }
}
