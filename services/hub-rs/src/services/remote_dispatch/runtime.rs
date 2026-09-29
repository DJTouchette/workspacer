//! Host-side observation and delivery for durable origin receipts.
use super::{Delivery, Origin, OriginRecord, Update};
use crate::{
    Handle, Options,
    client::Client,
    federation::Routes,
    services::{agent_lifecycle::Operation, progress::Deliver, task_store::OwnerLookup},
};
use anyhow::Result;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
struct NativeDelivery {
    lookup: OwnerLookup,
    replacements: Option<Arc<crate::services::manager_replacements::ReplacementState>>,
    send: Deliver,
    workflow: Option<Arc<crate::services::workflow_runtime::WorkflowRuntime>>,
}
impl Delivery for NativeDelivery {
    fn recipient<'a>(&'a self, owner: &'a str) -> Operation<'a, Option<String>> {
        Box::pin(async move {
            let target = match &self.replacements {
                Some(state) => {
                    let target = state.automatic_wake_target(owner)?;
                    if state.parked_successor(&target) || state.held(&target).is_some() {
                        return Ok(None);
                    }
                    target
                }
                None => owner.into(),
            };
            Ok((self.lookup)(&target)
                .filter(|row| {
                    row["status"] != "ended"
                        && row.get("hub").is_none_or(|hub| hub.is_null() || hub == "")
                })
                .map(|_| target))
        })
    }
    fn confirmed<'a>(&'a self, record: &'a OriginRecord, seq: u64) -> Operation<'a, bool> {
        Box::pin(async move {
            Ok(record.local_session_id.as_ref().is_some_and(|session| {
                self.replacements.as_ref().is_some_and(|state| {
                    state.signature(session).as_deref()
                        == Some(&format!("paired:{}:{seq}", record.dispatch_id))
                })
            }))
        })
    }
    fn deliver<'a>(
        &'a self,
        recipient: &'a str,
        record: &'a OriginRecord,
        update: &'a Update,
    ) -> Operation<'a, ()> {
        Box::pin(async move {
            let _admission = self
                .replacements
                .as_ref()
                .map(|state| state.admit(&[recipient]))
                .transpose()?;
            // Revalidate after journaling the delivery intent and before the engine
            // mutation. A concurrent replacement must not reroute this send.
            anyhow::ensure!(
                self.recipient(recipient).await?.as_deref() == Some(recipient),
                "remote wake owner changed before delivery; outcome retained"
            );
            let entry = match &self.workflow {
                Some(workflow) => super::paired::result_entry(record, update, workflow)?,
                None => update.entry.clone(),
            };
            let signature = record.local_session_id.as_ref().map(|session| {
                (
                    session.clone(),
                    format!("paired:{}:{}", record.dispatch_id, update.seq),
                )
            });
            if let (Some(state), Some((session, signature))) = (&self.replacements, &signature) {
                if state.signature(session).as_deref() == Some(signature) {
                    return Ok(());
                }
            }
            let kind = serde_json::to_value(update.kind)?;
            let mut text = crate::services::fleet_messages::build(
                kind.as_str().unwrap(),
                &[entry],
                (self.lookup)(recipient).is_none_or(|row| row["isWakeTarget"] != true),
            )?;
            if let (Some(workflow), Some(session)) = (&self.workflow, &record.local_session_id) {
                let history = workflow.tasks.snapshot()?;
                if let Some(task) = history.tasks.iter().find(|task| {
                    task["attempts"].as_array().is_some_and(|attempts| {
                        attempts
                            .iter()
                            .any(|attempt| attempt["sessionId"] == *session)
                    })
                }) {
                    text.push_str("\n\n");
                    text.push_str(&crate::services::workflow_runtime::instructions(
                        task,
                        &history.tasks,
                    ));
                }
            }
            (self.send)(recipient.into(), text).await?;
            if let (Some(state), Some((session, signature))) = (&self.replacements, &signature) {
                state.record_signature(session, signature)?;
            }
            Ok(())
        })
    }
}
pub(crate) struct Observer {
    pub task: tokio::task::JoinHandle<Result<()>>,
    stop: tokio::sync::watch::Sender<bool>,
}
impl Observer {
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}
pub(crate) fn install(
    mut options: Options,
    hub: Handle,
    routes: Routes,
) -> Result<(Options, Option<Observer>)> {
    if !options.control_plane_only {
        options=options.handler("fleet.dispatchTargets",|_,_|async{Ok(serde_json::json!({"targets":[],"linkedButNotEnabled":[],"note":"No workers-only pairing is configured."}))});
    }
    let Some(directory) = options.config_dir.clone() else {
        return Ok((options, None));
    };
    let delivery = if let Some(delivery) = options.remote_dispatch_delivery.clone() {
        delivery
    } else if let Some(engine) = options.engine.clone() {
        Arc::new(NativeDelivery {
            lookup: crate::services::local_lookup(&options),
            replacements: options.replacements.clone(),
            send: crate::services::progress::engine_delivery(&options, engine),
            workflow: options.workflow_runtime.clone(),
        })
    } else {
        return Ok((options, None));
    };
    let origin = Origin::open_with_routing(
        directory,
        hub.clone(),
        Arc::new(routes.clone()),
        delivery,
        options.routing.clone(),
    )?;
    origin.observer_ready(false);
    options.remote_origin = Some(origin.clone());
    options = super::paired::Paired::install(options, origin.clone(), routes.clone(), hub.clone())?;
    let proxies = options.remote_proxy_snapshots.clone();
    let workflow = options.workflow_runtime.clone();
    let (stop, mut stopping) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        tokio::select! {_=stopping.changed()=>return Ok(()),ready=hub.ready()=>{ready?;}}
        let client = Client::connect_service(&hub).await?;
        let mut events = client.events();
        client
            .topics(BTreeSet::from([
                "agent.dispatch.update".into(),
                "agent.snapshot".into(),
            ]))
            .await?;
        for record in origin
            .records()
            .iter()
            .filter(|record| record.local_session_id.is_some())
        {
            let _ = super::paired::project_snapshot(
                record,
                &serde_json::json!({"status":"starting","connectionState":"offline"}),
                &proxies,
                workflow.as_deref(),
                &hub,
            )
            .await;
            let _ =
                super::paired::project_retained(record, &proxies, workflow.as_deref(), &hub).await;
            let _ = super::paired::mark_offline(record, &proxies, &hub).await;
        }
        origin.observer_ready(true);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut connected = BTreeSet::new();
        loop {
            tokio::select! {biased;
                _=stopping.changed()=>{client.close();return Ok(())}
                event=events.recv()=>match event{Ok(event)=>{if event.topic=="agent.snapshot"{if let Some(snapshot)=&event.data{if let Some(record)=origin.records().iter().find(|record|record.peer==event.hub&&record.session_id.as_ref().is_some_and(|id|snapshot["sessionId"]==*id)){let _=super::paired::project_snapshot(record,snapshot,&proxies,workflow.as_deref(),&hub).await;}}}else {if let Err(error)=origin.update(&event).await{eprintln!("remote dispatch receipt retained: {error}");}if let Some(record)=origin.records().iter().find(|record|record.peer==event.hub&&event.data.as_ref().is_some_and(|data|data["dispatchId"]==record.dispatch_id)){let _=super::paired::project_retained(record,&proxies,workflow.as_deref(),&hub).await;}}},Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{},Err(_)=>anyhow::bail!("remote dispatch observer disconnected")},
                _=tick.tick()=>{
                    let peers=routes.peers();let active:BTreeSet<String>=peers.into_iter().filter(|peer|peer.connected).map(|peer|peer.name).collect();
                    for peer in &active{if !connected.contains(peer)||origin.records().iter().any(|record|record.peer==*peer&&record.state=="open"){if let Err(error)=origin.reconcile(peer).await{eprintln!("remote dispatch reconciliation deferred: {error}");}}}
                    for peer in active.difference(&connected){for record in origin.records().iter().filter(|record|record.peer==*peer&&record.local_session_id.is_some()){if let Some(session)=&record.session_id{if let Ok(snapshot)=routes.forward(peer,"sessions.snapshot",serde_json::json!({"sessionId":session})).await{let _=super::paired::project_snapshot(record,&snapshot,&proxies,workflow.as_deref(),&hub).await;}}}}
                for peer in connected.difference(&active){for record in origin.records().iter().filter(|record|record.peer==*peer&&record.local_session_id.is_some()){let _=super::paired::mark_offline(record,&proxies,&hub).await;}}
                connected=active;
                }
            }
        }
    });
    Ok((options, Some(Observer { task, stop })))
}
