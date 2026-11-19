//! "Diagnose": what an error on screen is sent to the hub as
//! (`desktop.diagnose`), what comes back, and the GitHub issue the user can
//! file when the hub's model judges the fault to be Workspacer's own.
//!
//! The hub owns the instructions, the harness and model (`agents.diagnose`),
//! the caps and the credential masking. This side only gathers what the user
//! is already looking at; nothing leaves the machine except that call, and the
//! issue is a pre-filled page the user reviews and submits themselves.
use crate::background_tasks::{Log, Task};
use serde_json::{Value, json};

/// Where issues for this app are filed.
pub const ISSUES: &str = "https://github.com/DJTouchette/workspacer/issues/new";
/// Matches the hub's caps; longer text is cut here so the call stays small.
const SUBJECT_CHARS: usize = 200;
const DETAILS_CHARS: usize = 24_000;
/// Log kept from the end of a task's output.
const LOG_TAIL_LINES: usize = 60;
const LOG_TAIL_CHARS: usize = 8_000;
/// A GitHub new-issue link stops working well past ~8 KB.
const ISSUE_BODY_CHARS: usize = 5_000;

/// One error as the user saw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subject {
    /// A short line naming the problem ("Background task log is empty").
    pub title: String,
    /// Everything else on screen that bears on it, as labelled lines.
    pub details: String,
    /// The harness the error came from, so the hub can answer with it.
    pub provider: Option<String>,
}

impl Subject {
    /// The `desktop.diagnose` parameters.
    pub fn params(&self) -> Value {
        json!({
            "subject": clip(self.title.trim(), SUBJECT_CHARS),
            "details": clip(self.details.trim(), DETAILS_CHARS),
            "provider": self.provider,
        })
    }

    /// A notice the app showed (a failed request, a refused action).
    pub fn notice(text: &str, screen: &str, provider: Option<&str>, app_version: &str) -> Self {
        let mut details = format!(
            "Kind: an error notice in the app\nScreen: {screen}\nNotice: {}\n",
            text.trim()
        );
        if let Some(provider) = provider {
            details.push_str(&format!("Session harness: {provider}\n"));
        }
        details.push_str(&environment(app_version));
        Self {
            title: clip(first_line(text), SUBJECT_CHARS),
            details,
            provider: provider.map(str::to_owned),
        }
    }

    /// A background task and its log as the Tasks panel shows them.
    pub fn task<S: AsRef<str>>(
        task: Option<&Task>,
        log: &Log,
        lines: &[S],
        panel_error: Option<&str>,
        provider: Option<&str>,
        app_version: &str,
    ) -> Self {
        let mut details = String::from("Kind: a background task in the Tasks panel\n");
        let mut add = |label: &str, value: &str| {
            let value = value.trim();
            if !value.is_empty() {
                details.push_str(&format!("{label}: {value}\n"));
            }
        };
        if let Some(task) = task {
            add("Task id", &task.id);
            add("Task kind", &task.task_type);
            add("Status", &task.status);
            add("Description", &task.description);
            add("Command", &task.command);
            add("Summary", &task.summary);
            add("Task error", &task.error);
            add(
                "Command redirects output to",
                task.redirect_file.as_deref().unwrap_or(""),
            );
            add(
                "CLI task log exists",
                if task.has_output { "yes" } else { "no" },
            );
        } else {
            add("Task id", &log.task);
        }
        add(
            "Panel reading",
            match log.source {
                crate::background_tasks::Source::Redirect => "the file the command redirects to",
                crate::background_tasks::Source::Task => "the CLI's task log",
            },
        );
        add(
            "Daemon is waiting because",
            log.waiting.as_deref().unwrap_or(""),
        );
        add("Panel error", panel_error.unwrap_or(""));
        add("Session harness", provider.unwrap_or(""));
        add(
            "Log",
            &format!(
                "{} lines held{}{}",
                lines.len(),
                if log.truncated {
                    ", earlier output not held"
                } else {
                    ""
                },
                if log.done { ", finished" } else { "" }
            ),
        );
        details.push_str(&environment(app_version));
        if !lines.is_empty() {
            details.push_str("\nLast lines of the log:\n");
            details.push_str(&tail(lines, LOG_TAIL_LINES, LOG_TAIL_CHARS));
            details.push('\n');
        }
        let title = match task {
            Some(task) if task.status == "failed" => {
                format!("Background task failed: {}", label(task))
            }
            Some(task) if lines.is_empty() => {
                format!("Background task shows no output: {}", label(task))
            }
            Some(task) => format!("Background task: {}", label(task)),
            None => "Background task log".into(),
        };
        Self {
            title: clip(&title, SUBJECT_CHARS),
            details,
            provider: provider.map(str::to_owned),
        }
    }
}

/// The hub's answer, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub text: String,
    pub workspacer_bug: bool,
    pub issue_title: Option<String>,
    /// Who answered ("claude · sonnet").
    pub by: String,
}

/// A `desktop.diagnose` reply as the panel shows it: the diagnosis, or the
/// plain reason there is none.
pub fn read_reply(value: &Value) -> Result<Diagnosis, String> {
    let by = match (value["provider"].as_str(), value["model"].as_str()) {
        (Some(provider), Some(model)) => format!("{provider} · {model}"),
        (Some(provider), None) => provider.to_owned(),
        _ => String::new(),
    };
    if value["ok"] != true {
        return Err(failure(value["error"].as_str().unwrap_or("error"), &by));
    }
    let text = value["text"].as_str().unwrap_or("").trim();
    if text.is_empty() {
        return Err(failure("empty", &by));
    }
    Ok(Diagnosis {
        text: text.to_owned(),
        workspacer_bug: value["workspacerBug"] == true,
        issue_title: value["issueTitle"]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
        by,
    })
}

fn failure(reason: &str, by: &str) -> String {
    let who = if by.is_empty() { "The model" } else { by };
    match reason {
        "busy" => "Two diagnoses are already running; try again in a moment.".into(),
        "timeout" => format!("{who} did not answer in time."),
        "missing" => format!("{who} is not installed on the hub's machine."),
        "unauthenticated" => format!("{who} is not signed in on the hub's machine."),
        "limited" => format!("{who} is out of usage right now."),
        "unsupported" => {
            format!("{who} cannot answer here; pick another harness in Settings → Diagnose.")
        }
        "network-error" => format!("{who} could not reach its service."),
        "empty" => format!("{who} did not give a usable answer."),
        other => format!("The diagnosis failed ({other})."),
    }
}

/// The issue as Markdown: what the user saw, then the diagnosis. Also what
/// Copy puts on the clipboard.
pub fn report(subject: &Subject, diagnosis: &Diagnosis) -> String {
    format!(
        "## What I saw\n{}\n\n```text\n{}\n```\n\n## Diagnosis ({})\n{}\n",
        subject.title.trim(),
        subject.details.trim(),
        if diagnosis.by.is_empty() {
            "model"
        } else {
            &diagnosis.by
        },
        diagnosis.text.trim()
    )
}

/// A pre-filled new-issue page. The body is cut to keep the link working;
/// the user reviews (and can edit out paths) before submitting.
pub fn issue_url(subject: &Subject, diagnosis: &Diagnosis) -> String {
    let title = diagnosis
        .issue_title
        .clone()
        .unwrap_or_else(|| subject.title.clone());
    let mut body = report(subject, diagnosis);
    if body.chars().count() > ISSUE_BODY_CHARS {
        body = clip(&body, ISSUE_BODY_CHARS);
        body.push_str("\n\n_(cut to fit the link; use Copy in the app for the full report)_\n");
    }
    url::Url::parse_with_params(
        ISSUES,
        [
            ("title", title.as_str()),
            ("body", body.as_str()),
            ("labels", "bug,diagnosed"),
        ],
    )
    .map(String::from)
    .unwrap_or_else(|_| ISSUES.into())
}

fn environment(app_version: &str) -> String {
    format!(
        "App: Workspacer native {app_version} on {} {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn label(task: &Task) -> String {
    let text = if task.description.trim().is_empty() {
        task.command.trim()
    } else {
        task.description.trim()
    };
    clip(first_line(text), 120)
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or("").trim()
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn tail<S: AsRef<str>>(lines: &[S], count: usize, chars: usize) -> String {
    let start = lines.len().saturating_sub(count);
    let joined = lines[start..]
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<&str>>()
        .join("\n");
    let total = joined.chars().count();
    if total <= chars {
        joined
    } else {
        joined.chars().skip(total - chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_tasks::{Kind, Source};

    fn shell() -> Task {
        Task {
            id: "b1".into(),
            kind: Kind::Shell,
            task_type: "local_bash".into(),
            status: "running".into(),
            description: "Build test projects and run tests".into(),
            command: "log=$(mktemp); dotnet test > \"$log\" 2>&1".into(),
            redirect_file: Some("mktemp file".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_task_subject_carries_the_command_the_wait_and_the_log_tail() {
        let mut log = Log::new("s1", "b1", Source::Redirect);
        log.waiting = Some("2 temp files were created".into());
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let subject = Subject::task(
            Some(&shell()),
            &log,
            &lines,
            None,
            Some("claude"),
            "0.200.0",
        );
        assert_eq!(
            subject.title,
            "Background task: Build test projects and run tests"
        );
        for needle in [
            "Command: log=$(mktemp); dotnet test",
            "Daemon is waiting because: 2 temp files were created",
            "Command redirects output to: mktemp file",
            "line 99",
            "App: Workspacer native 0.200.0",
        ] {
            assert!(subject.details.contains(needle), "{needle}");
        }
        assert!(!subject.details.contains("line 39\n"), "only the tail");
        assert_eq!(subject.params()["provider"], "claude");

        let empty = Subject::task::<String>(Some(&shell()), &log, &[], None, None, "x");
        assert!(empty.title.starts_with("Background task shows no output"));
    }

    #[test]
    fn replies_read_as_a_diagnosis_or_a_plain_reason() {
        let ok = read_reply(&json!({"ok":true,"provider":"claude","model":"sonnet",
            "text":"## What happened\nx","workspacerBug":true,"issueTitle":"Blank log"}))
        .unwrap();
        assert_eq!(ok.by, "claude · sonnet");
        assert!(ok.workspacer_bug);
        assert_eq!(ok.issue_title.as_deref(), Some("Blank log"));
        assert_eq!(
            read_reply(&json!({"ok":false,"provider":"codex","error":"unauthenticated"})),
            Err("codex is not signed in on the hub's machine.".into())
        );
        assert!(read_reply(&json!({"ok":true,"text":"  "})).is_err());
    }

    #[test]
    fn the_issue_link_is_prefilled_and_bounded() {
        let subject = Subject::notice("Couldn't load history: hub timed out", "History", None, "1");
        let diagnosis = Diagnosis {
            text: "y".repeat(20_000),
            workspacer_bug: true,
            issue_title: Some("History times out".into()),
            by: "claude · sonnet".into(),
        };
        let link = issue_url(&subject, &diagnosis);
        assert!(link.starts_with(ISSUES));
        let parsed = url::Url::parse(&link).unwrap();
        let get = |key: &str| {
            parsed
                .query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(get("title").as_deref(), Some("History times out"));
        let body = get("body").unwrap();
        assert!(body.contains("Couldn't load history"));
        assert!(body.contains("cut to fit the link"));
        assert!(link.len() < 20_000);
    }
}
