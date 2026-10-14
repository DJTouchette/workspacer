//! Codex app-server extras on the bus: `agents.review`, `agents.setGoal`,
//! `agents.clearGoal`, `agents.windowsSandboxSetup` and `agents.rename`.
//!
//! Each proxies one claudemon route that hands the request to the session's
//! live codex driver (`claudemon::daemon::codex_extras_api`) and answers with
//! Codex's own verdict. `agents.rename` is the one that is not Codex-only: it
//! records the Workspacer label on the launch record (so every client shows
//! it) and, for a live Codex session, also names the Codex thread.
use super::{Sessions, segment};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

/// Longest label `agents.rename` accepts.
const MAX_NAME: usize = 200;

/// The daemon's own error text, when it sent one, instead of the whole
/// `daemon returned 502 …: {json}` transport line.
fn daemon_error(error: anyhow::Error) -> anyhow::Error {
    let text = error.to_string();
    let reason = text
        .find('{')
        .and_then(|at| {
            serde_json::from_str::<Value>(text[at..].trim_end_matches(|c| c != '}')).ok()
        })
        .and_then(|body| body["error"].as_str().map(str::to_owned));
    match reason {
        Some(reason) => anyhow!(reason),
        None => error,
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => bail!("{key} must be text"),
    }
}

/// `agents.review` `target`: Codex's `ReviewTarget`, rebuilt from only the
/// fields its type names (claudemon re-validates).
pub(super) fn review_target(target: &Value) -> Result<Value> {
    let field = |key: &str| -> Result<&str> {
        match target.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(s),
            _ => bail!("target.{key} is required text"),
        }
    };
    Ok(match target.get("type").and_then(Value::as_str) {
        Some("uncommittedChanges") => json!({"type":"uncommittedChanges"}),
        Some("baseBranch") => {
            let branch = field("branch")?;
            // Codex hands the branch to git: an option-looking name is refused.
            if branch.starts_with('-') {
                bail!("target.branch must not start with '-'");
            }
            json!({"type":"baseBranch","branch":branch})
        }
        Some("commit") => {
            let mut out = json!({"type":"commit","sha":field("sha")?});
            if let Some(title) = text(target, "title")? {
                out["title"] = json!(title);
            }
            out
        }
        Some("custom") => json!({"type":"custom","instructions":field("instructions")?}),
        _ => bail!("target.type must be uncommittedChanges, baseBranch, commit or custom"),
    })
}

impl Sessions {
    pub(super) async fn codex_call(&self, method: &str, params: Value) -> Result<Value> {
        let id = segment(params["sessionId"].as_str().unwrap_or(""))?.to_owned();
        if self.remote_proxies.read().unwrap().contains_key(&id)
            || self
                .rows
                .read()
                .unwrap()
                .get(&id)
                .is_some_and(|row| row["hub"].as_str().is_some_and(|hub| !hub.is_empty()))
        {
            bail!("{method} for a remote session runs on its owning hub");
        }
        let root = format!("/sessions/{id}");
        let (path, payload) = match method {
            "agents.review" => {
                let target = review_target(params.get("target").unwrap_or(&Value::Null))?;
                let body = json!({"target":target});
                // Inline only: detached delivery is deprecated (Codex 0.159)
                // and runs on a thread whose findings this session never sees.
                match text(&params, "delivery")? {
                    None | Some("inline") => {}
                    Some(_) => {
                        bail!("delivery must be \"inline\" (detached reviews are not supported)")
                    }
                }
                (format!("{root}/review"), body)
            }
            "agents.setGoal" => {
                let mut body = json!({});
                for key in ["objective", "status"] {
                    if let Some(value) = text(&params, key)? {
                        body[key] = json!(value);
                    }
                }
                match params.get("tokenBudget") {
                    None => {}
                    Some(Value::Null) => body["tokenBudget"] = Value::Null,
                    Some(v) if v.as_i64().is_some_and(|n| n > 0) => body["tokenBudget"] = v.clone(),
                    Some(_) => bail!("tokenBudget must be a positive integer or null"),
                }
                if body.as_object().is_some_and(|b| b.is_empty()) {
                    bail!("agents.setGoal needs an objective, a status or a tokenBudget");
                }
                (format!("{root}/goal"), body)
            }
            "agents.clearGoal" => (format!("{root}/goal/clear"), json!({})),
            "agents.windowsSandboxSetup" => {
                let mut body = json!({});
                match text(&params, "mode")? {
                    None => {}
                    Some(m @ ("elevated" | "unelevated")) => body["mode"] = json!(m),
                    Some(_) => bail!("mode must be \"elevated\" or \"unelevated\""),
                }
                (format!("{root}/windows-sandbox/setup"), body)
            }
            "agents.rename" => return self.rename(&id, &params).await,
            _ => bail!("unknown codex extra"),
        };
        let reply = self
            .request("POST", path, Some(payload))
            .await
            .map_err(daemon_error)?;
        Ok(json!({"ok":true,"result":reply["result"].clone()}))
    }

    /// Name a session: the Workspacer label (launch record) and, for a live
    /// Codex session, its thread (`thread/name/set`) — so `codex resume` and
    /// the Codex TUI show the same name.
    async fn rename(&self, id: &str, params: &Value) -> Result<Value> {
        let name = text(params, "name")?
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| anyhow!("agents.rename requires {{ sessionId, name }}"))?;
        if name.len() > MAX_NAME || name.chars().any(char::is_control) {
            bail!("name must be one line of at most {MAX_NAME} bytes");
        }
        let row = self.rows.read().unwrap().get(id).cloned();
        let Some(row) = row else {
            bail!("unknown session");
        };
        let lifecycle = self
            .lifecycle
            .as_ref()
            .filter(|l| l.contains(id))
            .ok_or_else(|| anyhow!("this session has no Workspacer launch record to name"))?;
        lifecycle.note_label(id, name).await?;
        let codex = row["provider"] == "codex" && !super::super::agent_ops::ended(&row);
        let thread = if codex {
            match self
                .request(
                    "POST",
                    format!("/sessions/{id}/thread-name"),
                    Some(json!({"name":name})),
                )
                .await
            {
                Ok(_) => json!(true),
                Err(error) => json!(daemon_error(error).to_string()),
            }
        } else {
            Value::Null
        };
        let _mutation = self.mutations.lock().await;
        let row = self.rows.read().unwrap().get(id).cloned();
        if let Some(row) = row {
            self.publish(row).await?;
        }
        let mut reply = json!({"ok":true,"label":name});
        match thread {
            Value::Bool(true) => reply["codexThreadNamed"] = json!(true),
            Value::String(error) => {
                reply["codexThreadNamed"] = json!(false);
                reply["codexThreadError"] = json!(error);
            }
            _ => {}
        }
        Ok(reply)
    }

    /// A hub automatic title for a Codex session with no thread name of its
    /// own also names the Codex thread. Best effort.
    pub(super) async fn name_codex_thread(&self, id: &str) {
        let row = self.rows.read().unwrap().get(id).cloned();
        let Some(row) = row.map(|r| self.enrich(r)) else {
            return;
        };
        if row["provider"] != "codex"
            || super::super::agent_ops::ended(&row)
            || row["thread_name"].as_str().is_some_and(|n| !n.is_empty())
        {
            return;
        }
        let Some(title) = row["autoTitle"]["title"].as_str().filter(|t| !t.is_empty()) else {
            return;
        };
        if let Err(error) = self
            .request(
                "POST",
                format!("/sessions/{id}/thread-name"),
                Some(json!({"name":title})),
            )
            .await
        {
            eprintln!("title: codex thread not named for {id}: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_targets_are_rebuilt_from_their_own_fields() {
        assert_eq!(
            review_target(&json!({"type":"commit","sha":"abc","title":"t","x":1})).unwrap(),
            json!({"type":"commit","sha":"abc","title":"t"})
        );
        assert_eq!(
            review_target(&json!({"type":"uncommittedChanges","branch":"x"})).unwrap(),
            json!({"type":"uncommittedChanges"})
        );
        for bad in [
            json!(null),
            json!({"type":"baseBranch"}),
            json!({"type":"baseBranch","branch":"-c core.pager=x"}),
            json!({"type":"custom","instructions":" "}),
            json!({"type":"commit","sha":"a","title":3}),
            json!({"type":"nope"}),
        ] {
            assert!(review_target(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn codex_snapshot_fields_get_aliases_and_the_thread_name_ranks_below_labels() {
        use super::super::snapshots::{compat, with_auto_title};
        let row = compat(json!({
            "session_id":"s","mode":"input","thread_name":"Codex name",
            "windows_sandbox":{"ready":false,"status":"notConfigured"},
            "collaboration_mode":"plan","goal":{"objective":"o","status":"active"},
            "autoTitle":{"title":"Hub title"}
        }));
        assert_eq!(row["threadName"], "Codex name");
        assert_eq!(row["windowsSandbox"]["status"], "notConfigured");
        assert_eq!(row["collaborationMode"], "plan");
        assert_eq!(row["goal"]["objective"], "o");
        // Unnamed: the Codex thread's own name beats the hub's auto title.
        assert_eq!(with_auto_title(row.clone())["label"], "Codex name");
        let mut untitled = row.clone();
        untitled["thread_name"] = Value::Null;
        assert_eq!(with_auto_title(untitled)["label"], "Hub title");
        // A Workspacer label always wins.
        let mut named = row;
        named["label"] = json!("Mine");
        assert_eq!(with_auto_title(named)["label"], "Mine");
    }

    #[test]
    fn daemon_errors_are_unwrapped_to_their_reason() {
        let raw = anyhow!(
            "daemon returned 502 Bad Gateway: {{\"error\":\"this Codex build does not support thread/goal/set\",\"ok\":false}}; execution outcome is unknown"
        );
        assert_eq!(
            daemon_error(raw).to_string(),
            "this Codex build does not support thread/goal/set"
        );
        assert_eq!(daemon_error(anyhow!("plain")).to_string(), "plain");
    }
}
