//! Host-owned services, shared by embedded and standalone runtimes.
pub mod machine_power;
pub mod agent_lifecycle;
pub mod agent_ops;
pub mod agent_spawn;
pub mod config;
pub mod capability_inventory;
pub mod briefs;
pub mod dispatch_templates;
pub mod discovery;
pub mod desktop_workflows;
pub mod files;
pub mod fleet_review;
pub mod image_preview;
pub mod ui_assets;
pub mod uploads;
pub mod filewatch;
pub mod fleet_messages;
pub mod git;
pub mod jobs;
pub mod host_status;
pub mod html_card;
pub mod layout;
pub mod launch_instructions;
pub mod library;
pub mod limits;
pub mod models;
pub mod live_controls;
pub mod external_claudemon;
pub mod nodes;
pub mod manager_requests;
pub mod manager_replacements;
pub mod paths;
pub mod profiles;
pub mod profile_accounts;
pub mod provider_utilities;
pub mod account_setup;
pub mod pricing;
pub mod analytics;
pub mod progress;
pub(crate) mod push;
pub mod quiescence;
pub mod remote_dispatch;
pub mod remote_admin;
pub mod routing;
pub mod search;
pub mod sessions;
pub mod session_facade;
pub mod snapshots;
pub mod spawn_plan;
pub mod stores;
pub(crate) mod state_paths;
pub mod task_store;
pub mod thresholds;
pub mod terminals;
pub mod usage_prefs;
pub mod wakes;
pub mod worktrees;
pub mod workflows;
pub mod workflow_artifacts;
pub mod workflow_runtime;
pub mod worker_results;
use crate::{Handle, Options};
use std::{path::PathBuf, sync::Arc};

pub(crate) fn local_lookup(options:&Options)->task_store::OwnerLookup{
    let rows=options.session_snapshots.clone();
    let lifecycle=options.launch_lifecycle.clone();
    let replacements=options.replacements.clone();
    Arc::new(move|id|{
        let row=rows.read().unwrap().get(id).cloned()?;
        Some(snapshots::with_host_metadata(row,lifecycle.as_deref(),replacements.as_deref()))
    })
}

pub(crate) fn install_config(mut options: Options, directory: PathBuf, handle: Handle) -> anyhow::Result<Options> {
    options = host_status::install(options,handle.clone());
    options = uploads::install(options);
    let scope=if options.engine.is_some(){"full"}else{"catalog"};
    let node=options.provider_relay.as_ref().map(|relay|relay.node_id.clone()).unwrap_or_else(||std::env::var("WKS_NODE_ID").unwrap_or_default().trim().to_owned());
    options=options.handler("brain.info",move|_,_|{let node=node.clone();async move{let mut info=serde_json::json!({"scope":scope,"provider":"brain"});if !node.is_empty(){info["node"]=node.into();}Ok(info)}});
    if !options.has_handler("notifications.post"){
        options=options.handler("notifications.post",|_,params|async move{
            let field=|key:&str|->anyhow::Result<&str>{match params.get(key){None|Some(serde_json::Value::Null)=>Ok(""),Some(value)=>value.as_str().ok_or_else(||anyhow::anyhow!("notification {key} must be text"))}};
            let title=field("title")?;let title=if title.is_empty(){"workspacer"}else{title};let body=field("body")?;
            eprintln!("hub: notification: {title} — {body}");Ok(serde_json::json!({"ok":true}))
        });
    }
    options = library::install(options, directory.clone(), handle.clone());
    options = stores::install(options, directory.clone());
    options = profiles::install(options, directory.clone());
    if let Some(home)=options.home_dir.clone(){options=profile_accounts::install(options,directory.clone(),home);}
    let config = Arc::new(config::Config::open(directory.join("config.yaml")));
    if let Some(home)=options.home_dir.clone(){options=provider_utilities::install(options,config.clone(),home);}
    if let Some(home)=options.home_dir.clone(){options=discovery::install(options,home.clone());options=ui_assets::install(options,home.clone(),directory.clone());options=briefs::install(options,config.clone(),home,handle.clone());}
    if let Some(home)=options.home_dir.clone(){options=fleet_review::install(options,directory.clone(),home);}
    options = worktrees::install(options, config.clone());
    let replacements=manager_replacements::ReplacementState::open(directory.join("manager-replacements.json"))?;
    replacements.recover_status()?;
    options.message_tracker=Some(manager_replacements::MessageTracker::new(replacements.clone()));
    options.replacements=Some(replacements);
    if let Some(home)=options.home_dir.clone(){
        let pricing=Arc::new(pricing::Pricing::new(home.clone()));
        options=pricing::install(options,pricing.clone());
        let analytics=Arc::new(analytics::Analytics::open(directory.join("headless-analytics.sqlite"),pricing)?);
        let (configured,watcher)=analytics::install(options,analytics,Arc::new(profiles::Profiles::new(directory.clone())),home);
        options=configured;options.analytics_watcher=Some(watcher);
    }
    let owner = local_lookup(&options);
    let definitions = Arc::new(workflows::WorkflowStore::new(directory.clone(),config.clone()));
    let tasks = Arc::new(task_store::TaskStore::open(directory.join("dispatch-history.json"))?);
    let workflow = Arc::new(workflow_runtime::WorkflowRuntime::new(definitions,tasks,owner));
    workflow.set_replacements(options.replacements.as_ref().unwrap().clone());
    if let (Some(home),Some(lifecycle),Some(worktrees),Some(routing))=(options.home_dir.clone(),options.launch_lifecycle.clone(),options.worktrees.clone(),options.routing.clone()){
        let coordinator=agent_spawn::SpawnCoordinator::new(directory.clone(),home,config.clone(),lifecycle,workflow.clone(),worktrees,routing);
        coordinator.set_replacements(options.replacements.as_ref().unwrap().clone());
        if let Some(preparation)=options.launch_preparation.clone(){coordinator.set_launch_integrations(handle.clone(),preparation);}
        options.spawn_coordinator=Some(coordinator.clone());
        if options.remote_dispatch_execution.is_none() && let Some(engine)=options.engine.clone(){
            let remote_coordinator=coordinator.clone();
            let spawn:remote_dispatch::readiness::Spawn=Arc::new(move|caller,admission,params|{
                let coordinator=remote_coordinator.clone();
                Box::pin(async move{coordinator.spawn_remote(caller,admission,params).await})
            });
            options.remote_dispatch_execution=Some(remote_dispatch::readiness::Native::new(config.clone(),engine,spawn));
        }
        options=options.handler("agents.spawn",move|caller,params|{let coordinator=coordinator.clone();async move{coordinator.spawn_for(caller,params).await}});
    }
    options = manager_requests::install(options,workflow.requests.clone());
    options.workflow_runtime = Some(workflow.clone());
    options = options.handler("fleetWorkflows.request",move |_,params| {
        let workflow=workflow.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let caller=params["callerSessionId"].as_str().unwrap_or("");
                anyhow::Ok(workflow.request(&params,caller))
            }).await?
        }
    });
    options = models::install(options, config.clone());
    for method in [
        "config.get",
        "config.reload",
        "config.getPath",
        "config.save",
        "desktop.saveConfig",
    ] {
        let service = config.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move {
                // File locking and fsync run off the executor. These operations
                // are bounded and do not interrupt the bus's read loop.
                tokio::task::spawn_blocking(move || match method {
                    "config.get" => Ok(service.get()),
                    "config.reload" => Ok(service.reload()),
                    "config.getPath" => Ok(serde_json::json!(service.path())),
                    "desktop.saveConfig" if caller.authenticated_host => {
                        let partial = params
                            .get("partial")
                            .filter(|p| p.is_object())
                            .cloned()
                            .ok_or_else(|| {
                                anyhow::anyhow!("Configuration patch must be an object")
                            })?;
                        service.save(partial, true)
                    }
                    "desktop.saveConfig" => anyhow::bail!(
                        "desktop services require the authenticated server owner's connection"
                    ),
                    _ => service.save(
                        if params.is_null() {
                            serde_json::json!({})
                        } else {
                            params
                        },
                        false,
                    ),
                })
                .await?
            }
        });
    }
    Ok(options)
}

pub(crate) fn install(
    mut options: Options,
    handle: Handle,
    directory: PathBuf,
) -> anyhow::Result<Options> {
    let layout = Arc::new(layout::Layout::open(
        Some(directory.join("layout.json")),
        handle,
    ));
    options.layout = Some(layout.clone());
    let get = layout.clone();
    let options = options
        .handler("layout.get", move |_, _| {
            let service = get.clone();
            async move { Ok(service.get()) }
        })
        .handler("layout.set", move |caller, params| {
            let service = layout.clone();
            async move { service.set(&caller, params) }
        });
    let mut prefs = usage_prefs::UsagePrefs::open(Some(
        directory.join("usage-pacing.json"),
    ));
    if let Some(routing) = options.routing.clone() { prefs = prefs.with_routing(routing); }
    let prefs = Arc::new(prefs);
    let get = prefs.clone();
    Ok(options
        .handler("usage.pacingSchedule", move |_, params| {
            let service = get.clone();
            async move { service.get(params) }
        })
        .handler("usage.setPacingSchedule", move |caller, params| {
            let service = prefs.clone();
            async move { service.set(&caller, params) }
        }))
}

pub(crate) fn atomic_json(
    path: &std::path::Path,
    value: &serde_json::Value,
    private: bool,
) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(if private {
                0o600
            } else {
                0o644
            }))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub mod live_streams;
pub(crate) mod owned_process;
