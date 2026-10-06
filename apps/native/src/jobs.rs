//! Hub jobs as the native Jobs view shows them.
//!
//! Jobs are written by agents (the `scheduled-jobs` skill and `propose_job`);
//! this client only reads them and performs the owner's few writes: approve a
//! proposal, pause/resume, run now, remove. The hub owns the spec, validation
//! and scheduling (`services/hub-rs/src/services/jobs.rs`), and every write is
//! a whole-spec `jobs.upsert` built from the row the hub listed, so a field
//! this client does not know about survives it.
//!
//! Approving a proposal is the one write that turns an agent's spec into a
//! job: clear `proposedBy` and arm it. A proposal that names `replaces` is
//! applied by the hub to that job in place (keeping its id, history and
//! on/off state), so the view shows it beside the job it would change.
use serde_json::{Value, json};

/// Fields `jobs.list` adds to the spec; never sent back.
const LIVE_FIELDS: [&str; 3] = ["nextRunAt", "running", "lastRun"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// `ok`, `error` or `skipped`.
    pub status: String,
    pub detail: String,
}

impl Run {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            started_at: value["startedAt"].as_i64()?,
            finished_at: value["finishedAt"].as_i64().filter(|&at| at > 0),
            status: value["status"].as_str().unwrap_or("").to_owned(),
            detail: value["detail"].as_str().unwrap_or("").to_owned(),
        })
    }

    pub fn label(&self) -> &'static str {
        match self.status.as_str() {
            "ok" => "ok",
            "skipped" => "skipped",
            _ => "failed",
        }
    }

    /// `"4s"`, `"2m 5s"`; empty when unfinished.
    pub fn duration(&self) -> String {
        let Some(end) = self.finished_at.filter(|&end| end > self.started_at) else {
            return String::new();
        };
        let secs = (end - self.started_at) / 1000;
        match secs {
            0 => "<1s".into(),
            1..=59 => format!("{secs}s"),
            _ => format!("{}m {}s", secs / 60, secs % 60),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Non-empty: an agent proposed this and nobody has approved it.
    pub proposed_by: String,
    /// On a proposal: the id of the job it would change.
    pub replaces: String,
    pub next_run_at: Option<i64>,
    pub running: bool,
    pub last_run: Option<Run>,
    /// The stored spec exactly as listed, without the live fields.
    pub spec: Value,
}

impl Job {
    fn parse(value: &Value) -> Option<Self> {
        let id = value["id"].as_str().filter(|id| !id.is_empty())?.to_owned();
        let mut spec = value.clone();
        let map = spec.as_object_mut()?;
        for field in LIVE_FIELDS {
            map.remove(field);
        }
        let text = |key: &str| value[key].as_str().unwrap_or("").trim().to_owned();
        Some(Self {
            id,
            name: text("name"),
            enabled: value["enabled"] == true,
            proposed_by: text("proposedBy"),
            replaces: text("replaces"),
            next_run_at: value["nextRunAt"].as_i64(),
            running: value["running"] == true,
            last_run: Run::parse(&value["lastRun"]),
            spec,
        })
    }

    pub fn proposal(&self) -> bool {
        !self.proposed_by.is_empty()
    }

    /// A proposed change to another job, rather than a new job.
    pub fn change(&self) -> bool {
        self.proposal() && !self.replaces.is_empty()
    }

    /// The `jobs.upsert` that approves this proposal.
    pub fn approval(&self) -> Value {
        let mut spec = self.spec.clone();
        spec["enabled"] = json!(true);
        if let Some(map) = spec.as_object_mut() {
            map.remove("proposedBy");
        }
        spec
    }

    /// The `jobs.upsert` that pauses (`false`) or resumes (`true`) this job.
    pub fn with_enabled(&self, enabled: bool) -> Value {
        let mut spec = self.spec.clone();
        spec["enabled"] = json!(enabled);
        spec
    }

    pub fn trigger_summary(&self) -> String {
        trigger_summary(&self.spec["trigger"])
    }

    pub fn action_summary(&self) -> String {
        action_summary(&self.spec["action"])
    }

    /// Everything the job will do, field by field: what the user reads
    /// before approving it, since there is no editor to open it in.
    pub fn details(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("Name".to_owned(), self.name.clone()),
            ("When".to_owned(), self.trigger_summary()),
        ];
        let action = &self.spec["action"];
        let text = |v: &Value, key: &str| v[key].as_str().unwrap_or("").trim().to_owned();
        match action["kind"].as_str().unwrap_or("") {
            "spawn" => {
                let spawn = &action["spawn"];
                rows.push(("Agent in".into(), text(spawn, "cwd")));
                let using: Vec<String> = ["provider", "model", "effort", "permissionMode"]
                    .iter()
                    .map(|key| text(spawn, key))
                    .filter(|v| !v.is_empty())
                    .collect();
                if !using.is_empty() {
                    rows.push(("Using".into(), using.join(" · ")));
                }
                for (ix, step) in spawn["context"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    rows.push((format!("Step {}", ix + 1), step_summary(step)));
                }
                rows.push(("Prompt".into(), text(spawn, "prompt")));
            }
            "shell" => {
                rows.push((
                    "Runs".into(),
                    format!("$ {}", text(&action["shell"], "command")),
                ));
                let cwd = text(&action["shell"], "cwd");
                if !cwd.is_empty() {
                    rows.push(("In".into(), cwd));
                }
            }
            "call" => {
                rows.push(("Calls".into(), text(&action["call"], "method")));
                let params = &action["call"]["params"];
                if !params.is_null() {
                    rows.push((
                        "Params".into(),
                        serde_json::to_string_pretty(params).unwrap_or_default(),
                    ));
                }
            }
            _ => {}
        }
        rows
    }
}

/// `jobs.list`, proposals first (they are waiting on the user), otherwise in
/// the hub's order.
pub fn parse_list(value: &Value) -> Vec<Job> {
    let mut jobs: Vec<Job> = value["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Job::parse)
        .collect();
    jobs.sort_by_key(|job| !job.proposal());
    jobs
}

/// `jobs.history`, newest first as the hub keeps it.
pub fn parse_runs(value: &Value) -> Vec<Run> {
    value["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Run::parse)
        .collect()
}

pub fn trigger_summary(trigger: &Value) -> String {
    match trigger["kind"].as_str().unwrap_or("") {
        "interval" => {
            let minutes = trigger["everyMinutes"].as_i64().unwrap_or(0);
            if minutes > 0 && minutes % 60 == 0 {
                format!("every {}h", minutes / 60)
            } else {
                format!("every {minutes}m")
            }
        }
        "daily" => {
            let at = trigger["at"].as_str().unwrap_or("?");
            let days: Vec<&str> = trigger["days"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| DAYS.get(d.as_u64()? as usize).copied())
                .collect();
            if days.is_empty() {
                format!("daily {at}")
            } else {
                format!("daily {at} · {}", days.join(" "))
            }
        }
        "once" => match trigger["once"].as_str() {
            Some(at) => format!("once, {at}"),
            None => "once".into(),
        },
        "manual" => "manual".into(),
        other => other.to_owned(),
    }
}

pub fn action_summary(action: &Value) -> String {
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or("?").to_owned();
    match action["kind"].as_str().unwrap_or("") {
        "spawn" => {
            let steps = action["spawn"]["context"].as_array().map_or(0, Vec::len);
            let pre = match steps {
                0 => String::new(),
                1 => "1 step → ".into(),
                n => format!("{n} steps → "),
            };
            format!("{pre}agent in {}", text(&action["spawn"], "cwd"))
        }
        "call" => format!("call {}", text(&action["call"], "method")),
        "shell" => format!("$ {}", text(&action["shell"], "command")),
        other => other.to_owned(),
    }
}

fn step_summary(step: &Value) -> String {
    let run = if step["kind"] == "call" {
        format!("call {}", step["call"]["method"].as_str().unwrap_or("?"))
    } else {
        format!("$ {}", step["shell"]["command"].as_str().unwrap_or("?"))
    };
    let pattern = step["skipUnlessMatch"].as_str().unwrap_or("");
    let guards: Vec<String> = [
        (step["skipIfEmpty"] == true).then(|| "skip if empty".to_owned()),
        (!pattern.is_empty()).then(|| format!("skip unless /{pattern}/")),
        (step["ignoreExitCode"] == true).then(|| "exit code ignored".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if guards.is_empty() {
        run
    } else {
        format!("{run}  ({})", guards.join(", "))
    }
}

/// `"in 5m"`, `"in 3h"`, `"due now"` for a time at or after `now` (ms).
pub fn until(at: i64, now: i64) -> String {
    let minutes = (at - now) / 60_000;
    match minutes {
        ..1 => "due now".into(),
        1..=59 => format!("in {minutes}m"),
        _ if minutes < 24 * 60 => format!("in {}h", (minutes + 30) / 60),
        _ => format!("in {}d", (minutes + 12 * 60) / (24 * 60)),
    }
}

/// `"just now"`, `"5m ago"`, `"3h ago"` for a time at or before `now` (ms).
pub fn since(at: i64, now: i64) -> String {
    let minutes = (now - at) / 60_000;
    match minutes {
        ..1 => "just now".into(),
        1..=59 => format!("{minutes}m ago"),
        _ if minutes < 24 * 60 => format!("{}h ago", (minutes + 30) / 60),
        _ => format!("{}d ago", (minutes + 12 * 60) / (24 * 60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Value {
        json!({"jobs":[
            {"id":"a","name":"Nightly tests","enabled":true,"createdAt":1,"updatedAt":2,
             "trigger":{"kind":"daily","at":"07:00","days":[1,2,3,4,5]},
             "action":{"kind":"spawn","spawn":{"cwd":"/work/api","prompt":"Triage:\n{{output}}",
                "provider":"claude","context":[{"kind":"shell","shell":{"command":"make test"},
                "ignoreExitCode":true,"skipUnlessMatch":"FAIL"}]}},
             "futureField":{"kept":true},
             "nextRunAt":5_000_000,"running":true,
             "lastRun":{"jobId":"a","startedAt":1000,"finishedAt":6000,"status":"skipped","detail":"no match"}},
            {"id":"p","name":"Nightly tests","enabled":false,"proposedBy":"helper","replaces":"a",
             "trigger":{"kind":"interval","everyMinutes":120},
             "action":{"kind":"shell","shell":{"command":"make test","cwd":"/work/api"}}},
            {"id":"","name":"no id is not a row"}
        ]})
    }

    #[test]
    fn list_puts_proposals_first_and_keeps_the_spec_without_live_fields() {
        let jobs = parse_list(&listed());
        assert_eq!(jobs.len(), 2);
        let (change, job) = (&jobs[0], &jobs[1]);
        assert!(change.proposal() && change.change());
        assert!(!job.proposal());
        assert_eq!(job.next_run_at, Some(5_000_000));
        assert!(job.running);
        let run = job.last_run.as_ref().unwrap();
        assert_eq!((run.label(), run.duration().as_str()), ("skipped", "5s"));
        for field in LIVE_FIELDS {
            assert!(job.spec.get(field).is_none(), "{field} is not spec");
        }
        assert_eq!(job.spec["futureField"], json!({"kept":true}));
    }

    #[test]
    fn writes_are_whole_specs_with_one_change() {
        let jobs = parse_list(&listed());
        let approval = jobs[0].approval();
        assert!(approval.get("proposedBy").is_none());
        assert_eq!(approval["enabled"], true);
        assert_eq!(
            approval["replaces"], "a",
            "the hub applies it to the target"
        );
        assert_eq!(approval["id"], "p");
        let paused = jobs[1].with_enabled(false);
        assert_eq!(paused["enabled"], false);
        assert_eq!(paused["futureField"], json!({"kept":true}));
        assert!(paused.get("nextRunAt").is_none());
    }

    #[test]
    fn summaries_and_details_read_like_the_desktop() {
        let jobs = parse_list(&listed());
        let job = &jobs[1];
        assert_eq!(job.trigger_summary(), "daily 07:00 · Mon Tue Wed Thu Fri");
        assert_eq!(job.action_summary(), "1 step → agent in /work/api");
        assert_eq!(jobs[0].trigger_summary(), "every 2h");
        assert_eq!(jobs[0].action_summary(), "$ make test");
        let details = job.details();
        let get = |label: &str| {
            details
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Agent in"), Some("/work/api"));
        assert_eq!(get("Using"), Some("claude"));
        assert_eq!(
            get("Step 1"),
            Some("$ make test  (skip unless /FAIL/, exit code ignored)")
        );
        assert_eq!(get("Prompt"), Some("Triage:\n{{output}}"));
        assert_eq!(jobs[0].details()[2], ("Runs".into(), "$ make test".into()));
        assert_eq!(trigger_summary(&json!({"kind":"manual"})), "manual");
        assert_eq!(
            trigger_summary(&json!({"kind":"interval","everyMinutes":45})),
            "every 45m"
        );
        assert_eq!(
            action_summary(&json!({"kind":"call","call":{"method":"x.y"}})),
            "call x.y"
        );
    }

    #[test]
    fn history_and_relative_times() {
        let runs = parse_runs(&json!({"runs":[
            {"startedAt":10,"finishedAt":130_010,"status":"error","detail":"boom"},
            {"status":"ok"}
        ]}));
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].label(), runs[0].duration().as_str()),
            ("failed", "2m 10s")
        );
        let min = 60_000;
        assert_eq!(until(0, 0), "due now");
        assert_eq!(until(5 * min, 0), "in 5m");
        assert_eq!(until(150 * min, 0), "in 3h");
        assert_eq!(until(3 * 24 * 60 * min, 0), "in 3d");
        assert_eq!(since(0, 30_000), "just now");
        assert_eq!(since(0, 90 * min), "2h ago");
    }
}
