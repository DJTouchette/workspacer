//! The always-on hub's external worker-node registry. Cloud operations select a
//! host-owned row; local engine health never stands in for a remote provider.
pub mod cloud;
mod exposure;
mod model;
mod supervisor;
use crate::{Caller, Handle, Options, client::Client, services::agent_lifecycle::Operation};
use anyhow::{Result, bail};
pub use model::{CloudConfig, ExitRecord, Node, State, Timings, View, load, valid_id};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
pub use supervisor::{Change, Probe, Publish, Reading, Supervisor};
struct Bus {
    hub: Handle,
    client: Client,
    probes: tokio::sync::Semaphore,
}
impl Probe for Bus {
    fn read(&self, timeout: Duration) -> Operation<'_, Reading> {
        Box::pin(async move {
            let Ok(_permit) = self.probes.try_acquire() else {
                return Ok(Reading::Unavailable);
            };
            if self.client.disconnect_reason().is_some() {
                return Ok(Reading::Unavailable);
            }
            let Some(connection) = self.hub.provider_connection("brain.info").await? else {
                return Ok(Reading::Absent);
            };
            let result = self
                .client
                .call_with_timeout("brain.info", json!({}), timeout)
                .await;
            let current = self.hub.provider_connection("brain.info").await?;
            if current != Some(connection) || self.client.disconnect_reason().is_some() {
                return Ok(Reading::Unavailable);
            }
            Ok(match result {
                Ok(value) if value.is_object() => Reading::Answered {
                    connection,
                    node: value["node"].as_str().unwrap_or("").into(),
                    last_exit: value
                        .get("lastExit")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok()),
                },
                _ => Reading::Silent(connection),
            })
        })
    }
    fn evict(&self, connection: u64) -> Operation<'_, bool> {
        Box::pin(self.hub.evict_provider("brain.info", connection))
    }
    fn close(&self) {
        self.client.close();
    }
}
pub(crate) struct Owner {
    pub task: tokio::task::JoinHandle<Result<()>>,
    pub service: Arc<Supervisor>,
}
impl Owner {
    pub fn stop(&self) {
        self.service.close();
    }
}
/// Registers nothing for the usual install with no node registry.
pub(crate) async fn install(mut options: Options, hub: Handle) -> Result<(Options, Option<Owner>)> {
    let Some(path) = options.nodes_file.clone().or_else(|| {
        options
            .config_dir
            .as_ref()
            .map(|directory| directory.join("nodes.json"))
    }) else {
        return Ok((options, None));
    };
    if path.as_os_str().is_empty() {
        return Ok((options, None));
    }
    let nodes = load(&path)?;
    if nodes.is_empty() {
        return Ok((options, None));
    }
    let exposure = exposure::file(&path);
    if exposure != exposure::Exposure::OwnerOnly {
        eprintln!(
            "nodes: credential registry file exposure is {}",
            exposure.name()
        );
    }
    let mut clients: BTreeMap<String, Arc<dyn cloud::Cloud>> = BTreeMap::new();
    for node in &nodes {
        if !node.coordinates() {
            continue;
        }
        let config = node.fly.as_ref().unwrap();
        let token = match model::token(config, || {
            std::env::var("FLY_API_TOKEN").unwrap_or_default()
        }) {
            Ok(token) => token,
            Err(_) => {
                eprintln!(
                    "nodes: {} cloud credential file unreadable; power controls disabled",
                    node.id
                );
                continue;
            }
        };
        if token.is_empty() {
            continue;
        }
        match cloud::Http::new(config, token) {
            Ok(client) => {
                clients.insert(node.id.clone(), client);
            }
            Err(_) => eprintln!(
                "nodes: {} cloud configuration invalid; power controls disabled",
                node.id
            ),
        }
    }
    // The broker starts after install; this private service client connects once
    // ready through the adapter below rather than querying an unstarted loop.
    let (bus_tx, bus_rx) = tokio::sync::watch::channel(None);
    let bus = Arc::new(LateBus(bus_rx));
    let publish_hub = hub.clone();
    let publish: Publish = Arc::new(move |change| {
        let _ = publish_hub.publish(crate::protocol::Event::new(
            "node.state_changed",
            "nodes",
            json!({"node":change.node,"previous":change.previous}),
        ));
    });
    let service = Supervisor::new(
        nodes,
        clients,
        bus,
        Timings {
            keep_failed_wakes_running: options.nodes_keep_failed_wakes_running,
            ..Timings::default()
        },
        publish,
    )?;
    for method in ["nodes.list", "nodes.wake", "nodes.sleep"] {
        let service = service.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move {
                if method == "nodes.list" {
                    return Ok(serde_json::to_value(service.list())?);
                }
                trusted(method, &caller)?;
                let id = params["id"].as_str().unwrap_or("").trim();
                if id.is_empty() {
                    bail!("node id required")
                };
                let view = if method == "nodes.wake" {
                    service.wake(id).await?
                } else {
                    service.sleep(id).await?
                };
                Ok(serde_json::to_value(view)?)
            }
        });
    }
    let running = service.clone();
    let task = tokio::spawn(async move {
        hub.ready().await?;
        let client = Client::connect_service(&hub).await?;
        bus_tx.send_replace(Some(Arc::new(Bus {
            hub,
            client,
            probes: tokio::sync::Semaphore::new(8),
        })));
        running.run().await
    });
    Ok((options, Some(Owner { task, service })))
}
fn trusted(method: &str, caller: &Caller) -> Result<()> {
    if !caller.trusted || !caller.plugin_id.is_empty() {
        bail!("{method} requires trusted operator authority; it starts billing or stops work")
    };
    Ok(())
}
struct LateBus(tokio::sync::watch::Receiver<Option<Arc<Bus>>>);
impl Probe for LateBus {
    fn read(&self, timeout: Duration) -> Operation<'_, Reading> {
        Box::pin(async move {
            let bus = self.0.borrow().clone();
            match bus {
                Some(bus) => bus.read(timeout).await,
                None => Ok(Reading::Unavailable),
            }
        })
    }
    fn evict(&self, connection: u64) -> Operation<'_, bool> {
        Box::pin(async move {
            let bus = self.0.borrow().clone();
            match bus {
                Some(bus) => bus.evict(connection).await,
                None => Ok(false),
            }
        })
    }
    fn close(&self) {
        if let Some(bus) = self.0.borrow().as_ref() {
            bus.close()
        }
    }
}
#[cfg(test)]
mod tests;
