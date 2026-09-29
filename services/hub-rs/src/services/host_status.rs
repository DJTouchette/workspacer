//! Read the owned engines directly. Public readiness never implies that a
//! later spawn has been admitted, and an absent service remains unknown.
use crate::{Handle, Options, Status};
use anyhow::{Result, bail};
use claudemon::daemon::embedded::{Command, Status as EngineStatus};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
fn heartbeat_limit(params: &Value) -> Result<i64> {
    let value = match params.get("limit") {
        None | Some(Value::Null) => 20,
        Some(value) => value
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("heartbeat limit must be an integer"))?,
    };
    Ok(if value <= 0 { 20 } else { value.min(200) })
}
pub(crate) fn install(mut options: Options, hub: Handle) -> Options {
    // Kept as the legacy compatibility shim: ambient session capabilities no
    // longer read or rewrite per-manager grant fields.
    options=options.handler("desktop.sessionGrantReconcile",|_,_|async{Ok(json!(false))});
    let engine = options.engine.clone();
    let ready = options.mcp_ready.clone();
    let mcp = options.mcp_listen.is_some();
    options = options.handler("desktop.agentRuntimeStatus", move |caller, _| {
        let engine = engine.clone();
        let hub = hub.clone();
        let ready = ready.clone();
        async move {
            if !caller.authenticated_host {
                bail!("desktop services require the authenticated server owner's connection");
            }
            let daemon = engine
                .as_ref()
                .map(|engine| match *engine.status().borrow() {
                    EngineStatus::Starting => "starting",
                    EngineStatus::Ready(_) => "ready",
                    EngineStatus::Stopped | EngineStatus::Failed(_) => "failed",
                })
                .unwrap_or("unknown");
            let hub_phase = match *hub.status().borrow() {
                Status::Starting => "starting",
                Status::Ready { .. } => "ready",
                Status::Stopped | Status::Failed(_) => "failed",
            };
            let facade = if !mcp {
                "unknown"
            } else if ready.load(Ordering::Acquire) {
                "ready"
            } else if hub_phase == "failed" {
                "failed"
            } else {
                "starting"
            };
            Ok(json!({"claudemon":daemon,"hub":hub_phase,"facade":facade}))
        }
    });
    if let Some(engine) = options.engine.clone() {
        options = options.handler("desktop.keepWarmHeartbeats", move |caller, params| {
            let engine = engine.clone();
            async move {
                if !caller.authenticated_host {
                    bail!("desktop services require the authenticated server owner's connection");
                }
                let limit = heartbeat_limit(&params)?;
                engine
                    .request(Command::Request {
                        method: "GET".into(),
                        path: format!("/heartbeats?limit={limit}"),
                        payload: None,
                    })
                    .await
            }
        });
    }
    options
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heartbeats_have_bounded_integer_query_inputs() {
        for (value, expected) in [
            (Value::Null, 20),
            (json!({}), 20),
            (json!({"limit":-1}), 20),
            (json!({"limit":8}), 8),
            (json!({"limit":9999}), 200),
        ] {
            assert_eq!(heartbeat_limit(&value).unwrap(), expected);
        }
        for value in [
            json!({"limit":0.5}),
            json!({"limit":"8"}),
            json!({"limit":true}),
        ] {
            assert!(heartbeat_limit(&value).is_err());
        }
    }
}
