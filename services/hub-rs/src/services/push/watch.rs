use super::{Kind, Notification};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Default)]
struct State {
    ambient: String,
    since: Option<i64>,
    checkpoints: usize,
    seen: i64,
}
#[derive(Default)]
pub(super) struct Watcher {
    states: BTreeMap<String, State>,
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn blocked(s: &str) -> bool {
    matches!(s, "waiting_approval" | "waiting_input")
}
fn working(s: &str) -> bool {
    matches!(s, "thinking" | "streaming" | "background")
}
pub(super) fn clip(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 140 {
        text
    } else {
        format!("{}…", text.chars().take(140).collect::<String>().trim_end())
    }
}
fn duration(ms: i64) -> String {
    if ms < 60_000 {
        format!("{}s", (ms + 500) / 1000)
    } else if ms < 3_600_000 {
        format!("{}m", (ms + 30_000) / 60_000)
    } else {
        let hours = ms / 3_600_000;
        let minutes = ((ms % 3_600_000) + 30_000) / 60_000;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{minutes}m")
        }
    }
}
impl Watcher {
    pub fn snapshot(&mut self, s: &Value, now: i64) -> Vec<Notification> {
        let id = text(s, "sessionId");
        if id.is_empty() || id.len() > 256 || !s.is_object() {
            return vec![];
        }
        let name = [
            text(s, "label"),
            text(s, "cwd")
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(""),
            text(s, "liveCwd")
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(""),
            "Worker",
        ]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap()
        .to_string();
        let name = clip(&name);
        let previous = self.states.remove(id);
        if s["status"] == "ended" {
            return if previous.is_some() {
                vec![Notification {
                    kind: Kind::Ended,
                    title: name,
                    body: "Session ended".into(),
                    detail: String::new(),
                    session: id.into(),
                    ran_for: 0,
                }]
            } else {
                vec![]
            };
        }
        let mut previous = previous.unwrap_or_default();
        let ambient = text(s, "ambientState");
        let prior = previous.ambient.clone();
        let prior_since = previous.since;
        if working(ambient) && (!working(&prior) || previous.since.is_none()) {
            previous.since = Some(now);
            previous.checkpoints = 0;
        }
        previous.ambient = ambient.into();
        previous.seen = now;
        let mut events = vec![];
        if working(ambient) {
            if let Some(since) = previous.since {
                let elapsed = (now - since).max(0);
                for (index, mark) in [600_000, 1_800_000]
                    .iter()
                    .enumerate()
                    .skip(previous.checkpoints)
                {
                    if elapsed < *mark {
                        break;
                    }
                    previous.checkpoints = index + 1;
                    events.push(Notification {
                        kind: Kind::Checkpoint,
                        title: format!("{name} still in flight"),
                        body: format!("{} so far", duration(elapsed)),
                        detail: String::new(),
                        session: id.into(),
                        ran_for: elapsed,
                    });
                }
            }
        }
        if blocked(ambient) && !blocked(&prior) {
            let (body, detail) = if ambient == "waiting_approval" {
                let tool = &s["pendingApproval"];
                let input = &tool["toolInput"];
                let detail = [
                    "command",
                    "file_path",
                    "path",
                    "pattern",
                    "url",
                    "description",
                ]
                .iter()
                .map(|key| text(input, key))
                .find(|value| !value.is_empty())
                .unwrap_or("");
                (
                    "Approve a tool use",
                    format!("{} {detail}", text(tool, "toolName")),
                )
            } else {
                (
                    "Answer a question",
                    text(&s["pendingQuestions"]["questions"][0], "question").into(),
                )
            };
            events.push(Notification {
                kind: Kind::Needs,
                title: format!("{name} needs you"),
                body: body.into(),
                detail: clip(&detail),
                session: id.into(),
                ran_for: 0,
            });
        } else if ambient == "idle" && (working(&prior) || blocked(&prior)) {
            if let Some(since) = prior_since {
                let elapsed = (now - since).max(0);
                let detail = s["conversation"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .rev()
                    .find(|row| {
                        row["role"] == "assistant" && !text(row, "content").trim().is_empty()
                    })
                    .map(|row| clip(text(row, "content")))
                    .unwrap_or_default();
                events.push(Notification {
                    kind: Kind::Finished,
                    title: format!("{name} landed"),
                    body: format!("Ran for {}", duration(elapsed)),
                    detail,
                    session: id.into(),
                    ran_for: elapsed,
                });
                previous.since = None;
                previous.checkpoints = 0;
            }
        }
        if self.states.len() >= 10_000 {
            if let Some(oldest) = self
                .states
                .iter()
                .min_by_key(|(_, state)| state.seen)
                .map(|(id, _)| id.clone())
            {
                self.states.remove(&oldest);
            }
        }
        self.states.insert(id.into(), previous);
        events
    }
}
