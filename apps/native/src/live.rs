//! Explicit live-provider exercise for the harness CLI. Never used by demo or tests.
use crate::{
    bus::{Client, Config, Event},
    controller::{Action, Command, Controller, View},
};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

async fn wait_for(controller: &Controller, predicate: impl Fn(&View) -> bool) -> Result<Arc<View>> {
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let view = views.borrow_and_update().clone();
            if predicate(&view) {
                return Ok(view);
            }
            if let Some(receipt) = &view.receipt
                && let Some(error) = &receipt.error
            {
                return Err(anyhow!("Live action failed: {error}"));
            }
            views.changed().await?;
        }
    })
    .await
    .map_err(|_| anyhow!("Timed out waiting for live session state"))?
}

async fn select(controller: &Controller, id: &str) -> Result<Arc<View>> {
    wait_for(controller, |v| v.sessions.iter().any(|s| s.id == id)).await?;
    controller.command(Command::Select(id.into()))?;
    wait_for(controller, |v| {
        v.selected.as_deref() == Some(id) && !v.loading
    })
    .await
}

fn answered(view: &View, session: &str, marker: &str) -> bool {
    view.selected.as_deref() == Some(session)
        && view
            .transcript
            .rows
            .iter()
            .any(|row| row.role == "Assistant" && row.text.contains(marker))
}

/// Starts exactly one disposable agent, exercises the production Controller,
/// then terminates only that session. No automatic retries of mutations.
/// `keep_open` is explicit so the native window can inspect the same session.
pub async fn run(
    mut config: Config,
    provider: String,
    cwd: PathBuf,
    keep_open: bool,
) -> Result<Value> {
    ensure!(
        cwd.is_absolute() && cwd.is_dir(),
        "Live-test cwd must be an existing absolute directory"
    );
    config.call_timeout = Duration::from_secs(60);
    let (bus, events) = Client::start(config.clone());
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match events.recv().await? {
                Event::Connected => return Ok(()),
                Event::Disconnected(reason) => return Err(anyhow!(reason)),
                Event::PowerPaused => {
                    return Err(anyhow!(
                        "Server requested a reconnect pause; probe did not wake it"
                    ));
                }
                Event::Data { .. } => {}
            }
        }
    })
    .await??;
    let started = Instant::now();
    let spawn = bus.call("agents.spawn",json!({"provider":provider,"transport":"stream","cwd":cwd,
        "label":"Native client live test","skipPermissions":false,
        "message":"This is a disposable native-client integration test. Do not use tools or modify files. Reply with exactly NATIVE_TEST_READY."})).await?;
    let id = spawn["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow!("Spawn returned no sessionId; inspect the backend before retrying"))?
        .to_owned();
    eprintln!("Disposable live session: {id}");
    let result = async {
        let controller = Controller::start(config.clone());
        let initial = select(&controller,&id).await?;
        if !answered(&initial,&id,"NATIVE_TEST_READY") {
            wait_for(&controller, |v| answered(v,&id,"NATIVE_TEST_READY")).await?;
        }
        let first_reply_ms = started.elapsed().as_millis();
        controller.command(Command::Act {session:id.clone(),action:Action::Send(
            "Do not use tools. Reply with exactly NATIVE_FOLLOWUP_OK.".into())})?;
        let reply = wait_for(&controller, |v| answered(v,&id,"NATIVE_FOLLOWUP_OK") && v.receipt.as_ref().is_some_and(|r| r.error.is_none() && r.session == id)).await?;
        let transcript_rows = reply.transcript.rows.len();
        drop(controller);
        // A new client must reconstruct the server's conversation without the
        // old client's cache. Socket-drop recovery itself is covered by tests.
        let reconnected = Controller::start(config.clone());
        let restored = select(&reconnected,&id).await?;
        if !answered(&restored,&id,"NATIVE_FOLLOWUP_OK") {
            wait_for(&reconnected, |v| answered(v,&id,"NATIVE_FOLLOWUP_OK")).await?;
        }
        Ok::<_,anyhow::Error>(json!({"session_id":id,"provider":provider,"first_reply_ms":first_reply_ms,
            "elapsed_ms":started.elapsed().as_millis(),"controller_send_verified":true,
            "assistant_reply_verified":true,"fresh_client_reseed_verified":true,"transcript_rows":transcript_rows,
            "kept_open":keep_open}))
    }.await;
    if !keep_open || result.is_err() {
        let cleanup = bus
            .call("claude.signal", json!({"sessionId":id,"signal":"SIGTERM"}))
            .await;
        if let Err(error) = cleanup {
            return Err(anyhow!(
                "Live test result: {result:?}; cleanup failed for {id}: {error}"
            ));
        }
    }
    result
}
