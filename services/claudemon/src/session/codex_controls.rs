//! Codex app-server extras the daemon exposes beyond chat: review, thread
//! goals, thread naming, Windows sandbox setup and plan (collaboration) mode.
//!
//! The codex ws driver (`providers::codex_extras`) owns the protocol; this
//! module is only the seam the HTTP API and the store talk to it through, plus
//! the snapshot shapes it publishes. Every operation answers with what Codex
//! actually said (a JSON-RPC result or its error text) — never
//! fire-and-forget — so a 0.153 build that lacks a method surfaces as a plain
//! "unsupported" error instead of a hang.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// One request for the codex driver.
#[derive(Debug, Clone, PartialEq)]
pub enum CodexOp {
    /// `review/start {threadId, target, delivery?}`. `target` is Codex's own
    /// tagged `ReviewTarget`, already validated by the caller.
    Review {
        target: Value,
        delivery: Option<String>,
    },
    /// `thread/goal/set`. `token_budget`: `None` leaves it, `Some(None)`
    /// clears it, `Some(Some(n))` sets it.
    SetGoal {
        objective: Option<String>,
        status: Option<String>,
        token_budget: Option<Option<i64>>,
    },
    /// `thread/goal/clear`.
    ClearGoal,
    /// `thread/name/set`.
    SetThreadName { name: String },
    /// `windowsSandbox/setupStart {mode, cwd}` — Windows hosts only.
    WindowsSandboxSetup { mode: String },
}

/// A [`CodexOp`] plus where the driver's verdict goes: Codex's result, or the
/// reason it could not be done.
#[derive(Debug)]
pub struct CodexControl {
    pub op: CodexOp,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

/// Plan-mode state shared by the store (which flips it from
/// `/permission-mode`) and the driver (which stamps `collaborationMode` on
/// each `turn/start`).
#[derive(Debug, Default)]
pub struct CollaborationShared {
    /// `collaborationMode/list` answered with a plan preset (needs the
    /// experimental opt-in). Until then plan mode is refused.
    pub available: AtomicBool,
    /// The user selected plan mode for this session.
    pub plan: AtomicBool,
    /// A plan turn was sent, so later turns must say `default` explicitly to
    /// leave it.
    pub engaged: AtomicBool,
    /// The thread's current model and effort (Codex requires `settings.model`
    /// on every collaboration mode).
    pub model: Mutex<Option<String>>,
    pub effort: Mutex<Option<String>>,
    /// The plan preset's own `reasoning_effort` (`collaborationMode/list`),
    /// `Value::Null` = the preset clears it, absent = keep the thread's.
    pub plan_effort: Mutex<Option<Value>>,
}

impl CollaborationShared {
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }
    pub fn is_plan(&self) -> bool {
        self.plan.load(Ordering::Relaxed)
    }
    pub fn set_plan(&self, plan: bool) {
        self.plan.store(plan, Ordering::Relaxed);
    }
}

/// What the store holds for a live codex ws session.
#[derive(Clone)]
pub struct CodexControlHandle {
    pub tx: mpsc::UnboundedSender<CodexControl>,
    pub collaboration: Arc<CollaborationShared>,
}

/// Snapshot `goal` — the thread's persisted Codex goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexGoal {
    pub objective: String,
    /// `active`, `paused`, `blocked`, `usageLimited`, `budgetLimited`,
    /// `complete`, or whatever a newer Codex names.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_used_seconds: Option<i64>,
}

impl CodexGoal {
    /// Parse Codex's `ThreadGoal` object.
    pub fn from_wire(goal: &Value) -> Option<Self> {
        let objective = goal.get("objective")?.as_str()?.to_string();
        Some(Self {
            objective,
            status: goal
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("active")
                .to_string(),
            token_budget: goal.get("tokenBudget").and_then(Value::as_i64),
            tokens_used: goal.get("tokensUsed").and_then(Value::as_i64),
            time_used_seconds: goal.get("timeUsedSeconds").and_then(Value::as_i64),
        })
    }
}

/// Snapshot `windows_sandbox` — Codex's Windows sandbox readiness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxStatus {
    pub ready: bool,
    /// Codex's readiness word (`ready`, `notConfigured`, `updateRequired`),
    /// or `settingUp` while a setup this daemon started runs.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
