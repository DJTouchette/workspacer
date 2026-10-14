//! Codex app-server extras a session row may carry and the calls that change
//! them: the thread's goal, its own name, Windows sandbox readiness, plan
//! (collaboration) mode, and reviews (`agents.review`, `agents.setGoal`,
//! `agents.clearGoal`, `agents.windowsSandboxSetup`, `agents.rename`).
//!
//! Each field is shown only when the row carries it: an older hub or a
//! provider without the feature simply never shows the control.
use serde_json::{Value, json};

/// The thread's persisted goal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Goal {
    pub objective: String,
    /// `active`, `paused`, `blocked`, `usageLimited`, `budgetLimited`,
    /// `complete`, or a newer word shown as is.
    pub status: String,
    pub token_budget: Option<i64>,
    pub tokens_used: Option<i64>,
    pub time_used_seconds: Option<i64>,
}

impl Goal {
    pub fn parse(value: &Value) -> Option<Self> {
        let objective = value["objective"].as_str()?.trim();
        if objective.is_empty() {
            return None;
        }
        let int = |names: &[&str]| names.iter().find_map(|n| value[*n].as_i64());
        Some(Self {
            objective: crate::transcript::head(objective, 4_000),
            status: crate::transcript::head(value["status"].as_str().unwrap_or("active"), 32),
            token_budget: int(&["tokenBudget", "token_budget"]),
            tokens_used: int(&["tokensUsed", "tokens_used"]),
            time_used_seconds: int(&["timeUsedSeconds", "time_used_seconds"]),
        })
    }

    pub fn status_label(&self) -> String {
        match self.status.as_str() {
            "active" => "Active".into(),
            "paused" => "Paused".into(),
            "blocked" => "Blocked".into(),
            "usageLimited" => "Usage limited".into(),
            "budgetLimited" => "Over budget".into(),
            "complete" => "Complete".into(),
            other => other.to_owned(),
        }
    }

    /// `12k of 50k tokens · 4m`, from what Codex reported.
    pub fn usage(&self) -> String {
        let tokens = |n: i64| crate::session_controls::tokens_short(n.max(0) as u64);
        let mut parts = Vec::new();
        match (self.tokens_used, self.token_budget) {
            (Some(used), Some(budget)) => {
                parts.push(format!("{} of {} tokens", tokens(used), tokens(budget)))
            }
            (Some(used), None) => parts.push(format!("{} tokens", tokens(used))),
            (None, Some(budget)) => parts.push(format!("budget {} tokens", tokens(budget))),
            (None, None) => {}
        }
        if let Some(seconds) = self.time_used_seconds.filter(|s| *s > 0) {
            parts.push(match seconds {
                s if s < 60 => format!("{s}s"),
                s if s < 3_600 => format!("{}m", s / 60),
                s => format!("{}h {}m", s / 3_600, s % 3_600 / 60),
            });
        }
        parts.join(" · ")
    }
}

/// Codex's Windows sandbox readiness (Windows hosts only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sandbox {
    pub ready: bool,
    /// `ready`, `notConfigured`, `updateRequired`, `settingUp`, …
    pub status: String,
    pub detail: Option<String>,
}

impl Sandbox {
    pub fn parse(value: &Value) -> Option<Self> {
        let ready = value["ready"].as_bool()?;
        Some(Self {
            ready,
            status: crate::transcript::head(value["status"].as_str().unwrap_or(""), 32),
            detail: value["detail"]
                .as_str()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(|d| crate::transcript::head(d, 500)),
        })
    }

    pub fn setting_up(&self) -> bool {
        self.status == "settingUp"
    }

    /// The not-ready notice, or `None` while ready.
    pub fn notice(&self) -> Option<String> {
        if self.ready {
            return None;
        }
        let why = match self.status.as_str() {
            "settingUp" => "Setting up Codex's Windows sandbox…".to_owned(),
            "updateRequired" => "Codex's Windows sandbox needs an update".to_owned(),
            _ => "Codex's Windows sandbox is not set up".to_owned(),
        };
        Some(match &self.detail {
            Some(detail) => format!("{why}: {detail}"),
            None => why,
        })
    }
}

/// What `agents.review` reviews.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewTarget {
    Uncommitted,
    BaseBranch(String),
    Commit { sha: String, title: Option<String> },
    Custom(String),
}

impl ReviewTarget {
    /// Codex's `ReviewTarget`, or why the typed value cannot be one.
    pub fn wire(&self) -> Result<Value, &'static str> {
        Ok(match self {
            Self::Uncommitted => json!({"type":"uncommittedChanges"}),
            Self::BaseBranch(branch) => {
                let branch = branch.trim();
                if branch.is_empty() {
                    return Err("Name the branch to review against.");
                }
                if branch.starts_with('-') {
                    return Err("A branch name cannot start with '-'.");
                }
                json!({"type":"baseBranch","branch":branch})
            }
            Self::Commit { sha, title } => {
                let sha = sha.trim();
                if sha.is_empty() {
                    return Err("Enter the commit to review.");
                }
                let mut target = json!({"type":"commit","sha":sha});
                if let Some(title) = title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    target["title"] = json!(title);
                }
                target
            }
            Self::Custom(instructions) => {
                let instructions = instructions.trim();
                if instructions.is_empty() {
                    return Err("Describe what the review should look at.");
                }
                json!({"type":"custom","instructions":instructions})
            }
        })
    }

    pub fn label(&self) -> String {
        match self {
            Self::Uncommitted => "uncommitted changes".into(),
            Self::BaseBranch(branch) => format!("changes against {}", branch.trim()),
            Self::Commit { sha, .. } => {
                format!("commit {}", crate::transcript::head(sha.trim(), 12))
            }
            Self::Custom(_) => "your instructions".into(),
        }
    }
}

/// One change to the thread's goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoalChange {
    /// Set or replace the objective; `token_budget` `None` leaves the budget.
    Set {
        objective: String,
        token_budget: Option<i64>,
    },
    /// `active` / `paused` / `complete`.
    Status(String),
    Clear,
}

impl GoalChange {
    pub fn wire(&self, session: &str) -> (&'static str, Value) {
        match self {
            Self::Set {
                objective,
                token_budget,
            } => {
                let mut params = json!({"sessionId":session,"objective":objective.trim()});
                if let Some(budget) = token_budget {
                    params["tokenBudget"] = json!(budget);
                }
                ("agents.setGoal", params)
            }
            Self::Status(status) => (
                "agents.setGoal",
                json!({"sessionId":session,"status":status}),
            ),
            Self::Clear => ("agents.clearGoal", json!({"sessionId":session})),
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Set {
                objective,
                token_budget,
            } => {
                !objective.trim().is_empty()
                    && objective.len() <= 4_000
                    && token_budget.is_none_or(|b| b > 0)
            }
            Self::Status(status) => matches!(status.as_str(), "active" | "paused" | "complete"),
            Self::Clear => true,
        }
    }
}

/// A typed token budget: empty is none, otherwise a positive whole number
/// (`50000`, `50k`, `1.5m`).
pub fn parse_budget(text: &str) -> Result<Option<i64>, &'static str> {
    let text = text.trim().to_ascii_lowercase().replace([',', '_', ' '], "");
    if text.is_empty() {
        return Ok(None);
    }
    let (number, scale) = match text.chars().last() {
        Some('k') => (&text[..text.len() - 1], 1_000.),
        Some('m') => (&text[..text.len() - 1], 1_000_000.),
        _ => (text.as_str(), 1.),
    };
    match number.parse::<f64>() {
        Ok(n) if n.is_finite() && n * scale >= 1. && n * scale < 1e15 => {
            Ok(Some((n * scale).round() as i64))
        }
        _ => Err("Enter the token budget as a whole number, such as 50000 or 50k."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goals_and_sandbox_parse_only_what_codex_sent() {
        let goal = Goal::parse(&json!({"objective":"Ship the parser","status":"active",
            "tokenBudget":50000,"tokensUsed":12000,"timeUsedSeconds":245}))
        .unwrap();
        assert_eq!(goal.usage(), "12k of 50k tokens · 4m");
        assert_eq!(goal.status_label(), "Active");
        assert!(Goal::parse(&json!({"objective":"  "})).is_none());
        assert!(Goal::parse(&Value::Null).is_none());
        let sandbox = Sandbox::parse(&json!({"ready":false,"status":"notConfigured"})).unwrap();
        assert_eq!(
            sandbox.notice().as_deref(),
            Some("Codex's Windows sandbox is not set up")
        );
        assert!(
            Sandbox::parse(&json!({"ready":true,"status":"ready"}))
                .unwrap()
                .notice()
                .is_none()
        );
    }

    #[test]
    fn review_targets_carry_only_codex_fields() {
        assert_eq!(
            ReviewTarget::Uncommitted.wire().unwrap(),
            json!({"type":"uncommittedChanges"})
        );
        assert_eq!(
            ReviewTarget::BaseBranch(" main ".into()).wire().unwrap(),
            json!({"type":"baseBranch","branch":"main"})
        );
        assert!(ReviewTarget::BaseBranch("--all".into()).wire().is_err());
        assert_eq!(
            ReviewTarget::Commit {
                sha: "abc123".into(),
                title: None
            }
            .wire()
            .unwrap(),
            json!({"type":"commit","sha":"abc123"})
        );
        assert!(ReviewTarget::Custom(" ".into()).wire().is_err());
    }

    #[test]
    fn goal_changes_name_their_call() {
        let set = GoalChange::Set {
            objective: "Ship it".into(),
            token_budget: Some(50_000),
        };
        assert_eq!(
            set.wire("s"),
            (
                "agents.setGoal",
                json!({"sessionId":"s","objective":"Ship it","tokenBudget":50000})
            )
        );
        assert_eq!(GoalChange::Clear.wire("s").0, "agents.clearGoal");
        assert!(!GoalChange::Status("blocked".into()).valid());
        assert_eq!(parse_budget("50k"), Ok(Some(50_000)));
        assert_eq!(parse_budget(""), Ok(None));
        assert!(parse_budget("-3").is_err());
        assert!(parse_budget("lots").is_err());
    }
}
