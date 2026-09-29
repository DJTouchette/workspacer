use super::*;
use crate::{
    Handle,
    federation::Routes,
    services::{
        agent_lifecycle::{Lifecycle, Phase},
        agent_spawn::SpawnCoordinator,
        jobs,
        manager_replacements::ReplacementState,
        workflow_runtime::WorkflowRuntime,
    },
};
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
pub trait EvidenceSource: Send + Sync + 'static {
    fn read(&self) -> BoxFuture<'_, Result<Evidence>>;
}
pub struct NativeSources {
    pub engine: Option<EmbeddedClient>,
    pub hub: Handle,
    /// None is explicitly disabled, not a failed schedule read.
    pub jobs: Option<Arc<jobs::Service>>,
    pub federation: Routes,
    pub coordinator: Option<Arc<SpawnCoordinator>>,
    pub lifecycle: Option<Arc<Lifecycle>>,
    pub workflow: Option<Arc<WorkflowRuntime>>,
    pub replacements: Option<Arc<ReplacementState>>,
    pub terminals: Option<Arc<super::super::terminals::Terminals>>,
}
impl EvidenceSource for NativeSources {
    fn read(&self) -> BoxFuture<'_, Result<Evidence>> {
        Box::pin(async move {
            let mut evidence = Evidence::unknown(chrono::Utc::now().timestamp_millis());
            let sessions = async {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    async {
                        if let Some(engine) = &self.engine {
                            engine.request(Command::Request {
                                method: "GET".into(),
                                path: "/sessions?state_only=true&include_archived=true&include_empty=true"
                                    .into(),
                                payload: None,
                            }).await
                        } else {
                            // Hub-only/central deployments may receive their
                            // session service from a registered provider. This
                            // private reader never asks fleet.quiescence, so it
                            // cannot recurse into the watcher. Its connection
                            // is marked infrastructure by the owning broker.
                            let client = crate::client::Client::connect_service(&self.hub).await?;
                            let result = client.call("sessions.snapshots", json!({})).await;
                            client.close();
                            result
                        }
                    },
                )
                .await
                .map_err(|_| "session provider timed out".to_string())?
                .map_err(|e| e.to_string())
            };
            let clients = async {
                self.hub
                    .quiescence_clients()
                    .await
                    .map_err(|e| e.to_string())
            };
            let peers = async {
                let requests = self.federation.peers().into_iter().map(|peer| async move {
                    let sessions = if !peer.connected {
                        Err("federation link disconnected".into())
                    } else {
                        match tokio::time::timeout(
                            Duration::from_secs(10),
                            self.federation
                                .forward(&peer.name, "sessions.snapshots", json!({})),
                        )
                        .await
                        {
                            Ok(value) => value.map_err(|e| e.to_string()),
                            Err(_) => Err("peer session provider timed out".into()),
                        }
                    };
                    PeerSessions {
                        name: peer.name,
                        connected: peer.connected,
                        sessions,
                    }
                });
                futures_util::future::join_all(requests).await
            };
            let (sessions, clients, peers) = tokio::join!(sessions, clients, peers);
            if let (Some(terminals), Ok(rows)) = (&self.terminals, &sessions) {
                for row in rows.as_array().into_iter().flatten() {
                    if let Some(id) = row["session_id"]
                        .as_str()
                        .or_else(|| row["sessionId"].as_str())
                    {
                        terminals.confirm_shell_observation(id);
                    }
                }
                evidence
                    .operations
                    .extend(terminal_blockers(rows, &terminals.owned_shell_ids()));
                for id in terminals.uncertain_shell_ids() {
                    evidence.operations.push(Blocker::new(
                        "terminal-unknown",
                        id,
                        "Owned shell launch outcome has not been observed",
                    ));
                }
            }
            evidence.sessions = sessions;
            evidence.clients = clients;
            evidence.peers = Ok(peers);
            evidence.jobs = match &self.jobs {
                Some(jobs) => jobs.schedule().map_err(|e| e.to_string()),
                None => Ok(vec![]),
            };
            if let Some(coordinator) = &self.coordinator {
                let count = coordinator.active_count();
                if count > 0 {
                    evidence.operations.push(Blocker::new(
                        "session-working",
                        "launch-admission",
                        format!("{count} host launch preparation(s) remain active"),
                    ));
                }
            }
            if let Some(lifecycle) = &self.lifecycle {
                for (id, record) in lifecycle.records() {
                    if record.phase == Phase::Preparing {
                        evidence.operations.push(Blocker::new(
                            "session-working",
                            id,
                            "Launch preparation has not produced a terminal receipt",
                        ));
                    } else if record.revocation_pending {
                        evidence.operations.push(Blocker::new(
                            "fleet-unreadable",
                            id,
                            "Launch credential cleanup has not been acknowledged",
                        ));
                    }
                }
            }
            if let Some(workflow) = &self.workflow {
                match workflow.tasks.snapshot() {
                    Ok(history) => {
                        for task in history.tasks {
                            if task.get("dispatchReservation").is_some() {
                                evidence.operations.push(Blocker::new(
                                    "session-working",
                                    task["taskId"].as_str().unwrap_or("workflow"),
                                    "Workflow dispatch reservation is still held",
                                ));
                            }
                        }
                    }
                    Err(e) => evidence.operations.push(Blocker::new(
                        "fleet-unreadable",
                        "workflows",
                        format!("Workflow ownership evidence unavailable: {e}"),
                    )),
                }
            }
            if let Some(replacements) = &self.replacements {
                for op in replacements.records() {
                    if !matches!(
                        op["phase"].as_str(),
                        Some("complete" | "failed" | "cancelled")
                    ) || op["deliveries"].as_array().into_iter().flatten().any(|d| {
                        matches!(
                            d["status"].as_str(),
                            Some("pending" | "sending" | "uncertain")
                        )
                    }) {
                        evidence.operations.push(Blocker::new(
                            "session-working",
                            op["operationId"].as_str().unwrap_or("manager-handoff"),
                            "Manager ownership or retained delivery still requires reconciliation",
                        ));
                    }
                }
            }
            Ok(evidence)
        })
    }
}

fn terminal_blockers(rows: &Value, owned: &std::collections::BTreeSet<String>) -> Vec<Blocker> {
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["session_id"]
                .as_str()
                .or_else(|| row["sessionId"].as_str())?;
            (owned.contains(id) && row["mode"] != "stopped" && row["status"] != "ended").then(
                || {
                    Blocker::new(
                        "terminal-unknown",
                        id,
                        "Owned shell has no trustworthy foreground-work telemetry",
                    )
                },
            )
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_input_mode_is_not_idle_work_evidence() {
        let ids = std::collections::BTreeSet::from(["shell".into()]);
        assert_eq!(
            terminal_blockers(&json!([{"session_id":"shell","mode":"input"}]), &ids).len(),
            1
        );
        assert!(
            terminal_blockers(&json!([{"session_id":"shell","mode":"stopped"}]), &ids).is_empty()
        );
        assert!(
            terminal_blockers(&json!([{"session_id":"agent","mode":"input"}]), &ids).is_empty()
        );
    }
}
