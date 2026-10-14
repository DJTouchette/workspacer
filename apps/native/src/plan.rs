//! The agent's task list: Claude's TodoWrite and Task tools (TaskCreate,
//! TaskUpdate, TaskList) and Codex's plan tool all reach the client as the
//! same last-write-wins plan — a `plan` conversation item and, where the
//! daemon reports it, the session row's `plan`.
use serde_json::Value;

/// Steps kept from one plan.
pub const MAX_STEPS: usize = 64;
/// Bytes of one step's text kept.
const MAX_STEP_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl Status {
    /// The wire vocabulary, read as defensively as the daemon writes it.
    pub fn from_wire(status: &str) -> Self {
        match status {
            "in_progress" | "inprogress" | "in-progress" | "inProgress" => Self::InProgress,
            "completed" | "complete" | "done" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub content: String,
    pub status: Status,
    /// Present-tense label for the step while it runs (Claude's `activeForm`).
    pub active_form: Option<String>,
}

impl Step {
    /// What the step says now: its active form while it runs.
    pub fn label(&self) -> &str {
        match (&self.active_form, self.status) {
            (Some(active), Status::InProgress) if !active.trim().is_empty() => active,
            _ => &self.content,
        }
    }
}

/// A plan as a session row reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Epoch milliseconds of the last rewrite, when the daemon knew it.
    pub updated_at: Option<i64>,
}

impl Plan {
    /// The row's `plan` (`{steps, updatedAt}`); `None` for an empty one.
    pub fn from_row(value: &Value) -> Option<Self> {
        let steps = parse(value);
        (!steps.is_empty()).then(|| Self {
            steps,
            updated_at: value["updatedAt"]
                .as_str()
                .and_then(crate::timing::parse_timestamp),
        })
    }
}

/// Steps from a plan's `steps` array (or a bare plan object carrying one).
pub fn parse(value: &Value) -> Vec<Step> {
    value
        .as_array()
        .or_else(|| value.get("steps").and_then(Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(|step| {
            let content = step["content"]
                .as_str()
                .or_else(|| step["step"].as_str())
                .map(str::trim)
                .filter(|c| !c.is_empty())?;
            Some(Step {
                content: crate::transcript::head(content, MAX_STEP_BYTES),
                status: Status::from_wire(step["status"].as_str().unwrap_or("")),
                active_form: step["activeForm"]
                    .as_str()
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(|a| crate::transcript::head(a, MAX_STEP_BYTES)),
            })
        })
        .take(MAX_STEPS)
        .collect()
}

/// `(completed, total)`.
pub fn progress(steps: &[Step]) -> (usize, usize) {
    (
        steps
            .iter()
            .filter(|s| s.status == Status::Completed)
            .count(),
        steps.len(),
    )
}

/// The step being worked on, if any.
pub fn active(steps: &[Step]) -> Option<&Step> {
    steps.iter().find(|s| s.status == Status::InProgress)
}

/// The plan to pin for a session: the newest `Plan` row of its transcript,
/// unless the row's own plan is newer (or the transcript window holds none).
/// Both come from the same daemon, so a tie goes to the transcript, which
/// updates first while the conversation streams.
pub fn current(
    transcript: &crate::model::Transcript,
    row: Option<&Plan>,
) -> Option<Vec<Step>> {
    let latest = transcript
        .rows
        .iter()
        .rev()
        .find(|r| r.role == "Plan" && r.tool.is_none())
        .and_then(|r| {
            let steps = parse(&serde_json::from_str::<Value>(&r.text).ok()?);
            Some((steps, r.timestamp_ms))
        });
    match (latest, row) {
        (Some((steps, at)), Some(row))
            if matches!((at, row.updated_at), (Some(a), Some(b)) if b > a) || steps.is_empty() =>
        {
            Some(row.steps.clone())
        }
        (Some((steps, _)), _) => Some(steps),
        (None, Some(row)) => Some(row.steps.clone()),
        (None, None) => None,
    }
    .filter(|steps| !steps.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConversationSnapshot, Item, Transcript};
    use serde_json::json;

    #[test]
    fn steps_read_the_shared_plan_shape_and_name_the_running_one() {
        let steps = parse(&json!({"steps":[
            {"content":"Explore","status":"completed"},
            {"content":"Build","status":"in_progress","activeForm":"Building"},
            {"content":"  ","status":"pending"},
            {"content":"Ship","status":"done"}
        ]}));
        assert_eq!(steps.len(), 3, "a blank step is dropped");
        assert_eq!(progress(&steps), (2, 3));
        assert_eq!(active(&steps).map(Step::label), Some("Building"));
        assert_eq!(steps[0].label(), "Explore");
    }

    #[test]
    fn the_newest_plan_wins_between_transcript_and_row() {
        let mut transcript = Transcript::default();
        transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "plan".into(),
                steps: json!([{"content":"From the transcript","status":"pending"}]),
                timestamp: Some("2026-10-01T10:00:00Z".into()),
                ..Default::default()
            }],
        });
        let older = Plan {
            steps: parse(&json!([{"content":"Older row"}])),
            updated_at: Some(0),
        };
        let pinned = current(&transcript, Some(&older)).unwrap();
        assert_eq!(pinned[0].content, "From the transcript");
        let newer = Plan::from_row(&json!({
            "steps":[{"content":"Newer row"}],
            "updatedAt":"2026-10-01T11:00:00Z"
        }))
        .unwrap();
        assert_eq!(
            current(&transcript, Some(&newer)).unwrap()[0].content,
            "Newer row"
        );
        assert_eq!(
            current(&Transcript::default(), Some(&older)).unwrap()[0].content,
            "Older row",
            "a plan written before the window still pins"
        );
        assert!(current(&Transcript::default(), None).is_none());
    }
}
