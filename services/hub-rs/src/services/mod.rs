//! Host-owned services, shared by embedded and standalone runtimes.
pub mod account_setup;
pub mod agent_lifecycle;
pub mod agent_ops;
pub mod agent_spawn;
pub mod analytics;
mod atomic_persist;
pub mod briefs;
pub mod capability_inventory;
pub mod config;
pub mod desktop_workflows;
pub mod discovery;
pub mod dispatch_templates;
pub mod external_claudemon;
pub mod files;
pub mod filewatch;
pub mod fleet_messages;
pub mod fleet_review;
pub mod git;
pub mod host_resources;
pub mod host_status;
pub mod html_card;
pub mod image_preview;
pub mod jobs;
pub mod kept_sessions;
pub mod launch_instructions;
pub mod layout;
pub mod library;
pub(crate) mod library_watch;
pub mod limits;
pub mod live_controls;
pub mod machine_power;
pub mod manager_replacements;
pub mod manager_requests;
pub mod message_queue;
pub mod models;
pub mod nodes;
pub mod paths;
pub mod pricing;
pub mod profile_accounts;
pub mod profiles;
pub mod progress;
pub mod project_groups;
pub mod provider_utilities;
pub mod pull_requests;
pub(crate) mod push;
pub mod quiescence;
pub mod remote_admin;
pub mod remote_dispatch;
pub mod roster;
pub mod routing;
pub mod scratch;
pub mod search;
pub mod session_archive;
pub mod session_facade;
pub mod session_open;
pub mod sessions;
pub mod snapshots;
pub mod spawn_plan;
pub(crate) mod state_paths;
pub mod stores;
pub mod task_store;
pub mod terminals;
pub mod thresholds;
pub mod ui_assets;
pub mod uploads;
pub mod usage_prefs;
pub mod wakes;
pub mod work_queue;
pub mod worker_results;
pub mod workflow_artifacts;
pub mod workflow_runtime;
pub mod workflows;
pub mod worktrees;
use crate::{Handle, Options};
use std::{path::PathBuf, sync::Arc};

pub(crate) fn local_lookup(options: &Options) -> task_store::OwnerLookup {
    let rows = options.session_snapshots.clone();
    let lifecycle = options.launch_lifecycle.clone();
    let replacements = options.replacements.clone();
    Arc::new(move |id| {
        let row = rows.read().unwrap().get(id).cloned()?;
        Some(snapshots::with_host_metadata(
            row,
            lifecycle.as_deref(),
            replacements.as_deref(),
        ))
    })
}

pub(crate) fn install_config(
    mut options: Options,
    directory: PathBuf,
    handle: Handle,
) -> anyhow::Result<Options> {
    options = host_status::install(options, handle.clone());
    options = uploads::install(options);
    let scope = if options.engine.is_some() {
        "full"
    } else {
        "catalog"
    };
    let node = options
        .provider_relay
        .as_ref()
        .map(|relay| relay.node_id.clone())
        .unwrap_or_else(|| {
            std::env::var("WKS_NODE_ID")
                .unwrap_or_default()
                .trim()
                .to_owned()
        });
    options = options.handler("brain.info", move |_, _| {
        let node = node.clone();
        async move {
            let mut info = serde_json::json!({"scope":scope,"provider":"brain"});
            if !node.is_empty() {
                info["node"] = node.into();
            }
            Ok(info)
        }
    });
    if !options.has_handler("notifications.post") {
        options = options.handler("notifications.post", |_, params| async move {
            let field = |key: &str| -> anyhow::Result<&str> {
                match params.get(key) {
                    None | Some(serde_json::Value::Null) => Ok(""),
                    Some(value) => value
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("notification {key} must be text")),
                }
            };
            let title = field("title")?;
            let title = if title.is_empty() {
                "workspacer"
            } else {
                title
            };
            let body = field("body")?;
            eprintln!("hub: notification: {title} — {body}");
            Ok(serde_json::json!({"ok":true}))
        });
    }
    options = library::install(options, directory.clone(), handle.clone());
    options = stores::install(options, directory.clone());
    options = profiles::install(options, directory.clone());
    if let Some(home) = options.home_dir.clone() {
        options = profile_accounts::install(options, directory.clone(), home);
    }
    let config = Arc::new(config::Config::open(directory.join("config.yaml")));
    // The work queue's `queue.*` limits live in this same shared file.
    options.shared_config = Some(config.clone());
    // The sidebar cleanup prompt's threshold (`sessions.cleanupThreshold`).
    if let Some(open) = &options.session_open {
        open.set_config(config.clone());
    }
    if let Some(home) = options.home_dir.clone() {
        options = provider_utilities::install(options, config.clone(), home);
    }
    if let Some(home) = options.home_dir.clone() {
        options = discovery::install(options, home.clone());
        options = ui_assets::install(options, home.clone(), directory.clone());
        options = briefs::install(options, config.clone(), home, handle.clone());
    }
    if let Some(home) = options.home_dir.clone() {
        options = fleet_review::install(options, directory.clone(), home);
    }
    options = worktrees::install(options, config.clone());
    let pull_requests = Arc::new(pull_requests::Service::new(
        config.clone(),
        Arc::new(pull_requests::links::Links::open(
            directory.join("pr-links.json"),
        )),
    ));
    options.pull_requests = Some(pull_requests.clone());
    options = pull_requests::install(options, pull_requests);
    let replacements =
        manager_replacements::ReplacementState::open(directory.join("manager-replacements.json"))?;
    replacements.recover_status()?;
    options.message_tracker = Some(manager_replacements::MessageTracker::new(
        replacements.clone(),
    ));
    options.replacements = Some(replacements);
    if let Some(home) = options.home_dir.clone() {
        let pricing = Arc::new(pricing::Pricing::new(home.clone()));
        options = pricing::install(options, pricing.clone());
        // Catalog-only composition leaves analytics to the desktop provider.
        // An unavailable local source must not claim and shadow that capability.
        if options.engine.is_some() {
            let analytics = Arc::new(analytics::Analytics::open(
                directory.join("headless-analytics.sqlite"),
                pricing,
            )?);
            let (configured, watcher) = analytics::install(
                options,
                analytics,
                Arc::new(profiles::Profiles::new(directory.clone())),
                home,
            );
            options = configured;
            options.analytics_watcher = Some(watcher);
        }
    }
    let owner = local_lookup(&options);
    let definitions = Arc::new(workflows::WorkflowStore::new(
        directory.clone(),
        config.clone(),
    ));
    let tasks = Arc::new(task_store::TaskStore::open(
        directory.join("dispatch-history.json"),
    )?);
    let workflow = Arc::new(workflow_runtime::WorkflowRuntime::new(
        definitions,
        tasks,
        owner,
    ));
    workflow.set_replacements(options.replacements.as_ref().unwrap().clone());
    if let (Some(home), Some(lifecycle), Some(worktrees), Some(routing)) = (
        options.home_dir.clone(),
        options.launch_lifecycle.clone(),
        options.worktrees.clone(),
        options.routing.clone(),
    ) {
        let coordinator = agent_spawn::SpawnCoordinator::new(
            directory.clone(),
            home,
            config.clone(),
            lifecycle,
            workflow.clone(),
            worktrees,
            routing,
        );
        coordinator.set_replacements(options.replacements.as_ref().unwrap().clone());
        if let Some(preparation) = options.launch_preparation.clone() {
            coordinator.set_launch_integrations(handle.clone(), preparation);
        }
        options.spawn_coordinator = Some(coordinator.clone());
        if options.remote_dispatch_execution.is_none()
            && let Some(engine) = options.engine.clone()
        {
            let remote_coordinator = coordinator.clone();
            let spawn: remote_dispatch::readiness::Spawn =
                Arc::new(move |caller, admission, params| {
                    let coordinator = remote_coordinator.clone();
                    Box::pin(
                        async move { coordinator.spawn_remote(caller, admission, params).await },
                    )
                });
            options.remote_dispatch_execution = Some(remote_dispatch::readiness::Native::new(
                config.clone(),
                engine,
                spawn,
            ));
        }
        let forks = coordinator.clone();
        options = options.handler("agents.spawn", move |caller, params| {
            let coordinator = coordinator.clone();
            async move { coordinator.spawn_for(caller, params).await }
        });
        // A fork is a launch: same coordinator, admission and journal as
        // agents.spawn, with the source's launch record as its template.
        options = options.handler("agents.fork", move |caller, params| {
            let coordinator = forks.clone();
            async move { coordinator.fork_for(caller, params).await }
        });
    }
    options = manager_requests::install(options, workflow.requests.clone());
    options.workflow_runtime = Some(workflow.clone());
    options = options.handler("fleetWorkflows.request", move |_, params| {
        let workflow = workflow.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let caller = params["callerSessionId"].as_str().unwrap_or("");
                anyhow::Ok(workflow.request(&params, caller))
            })
            .await?
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
        handle.clone(),
    ));
    let archive = Arc::new(session_archive::SessionArchive::open(
        Some(directory.join("session-archive.json")),
        Some(handle.clone()),
    ));
    // The shared open set (see `session_open`): every client's Paused
    // sessions, and the roster always lists its members. Seeded once from
    // native's legacy per-device `kept_open` settings.
    let open = Arc::new(session_open::SessionOpen::open(
        Some(directory.join("session-open.json")),
        Some(handle),
        Some(archive.clone()),
        kept_sessions::settings_path(),
    ));
    options.layout = Some(layout.clone());
    options.session_open = Some(open.clone());
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
    // Archive is shared view state, like the layout: every client of this hub
    // hides the same sessions. It never touches a session's lifecycle.
    // Archiving also lets the session go from the open set.
    let get = archive.clone();
    // Restore puts a session back on the sidebar, marked ended unless it is
    // known to be live: in the embedded projection, or observed live on the
    // bus (another provider's rows). A live one clears the mark on its next
    // snapshot anyway.
    let projection = options.session_snapshots.clone();
    let live_check = {
        let open = open.clone();
        move |id: &str| {
            open.observed_live(id)
                || projection.read().unwrap().get(id).is_some_and(|row| {
                    roster::live(row) || roster::starting(row, &roster::Ctx::default())
                })
        }
    };
    let set_archive = archive.clone();
    let set_open = open.clone();
    let set_live = live_check.clone();
    let many_archive = archive.clone();
    let many_open = open.clone();
    let many_live = live_check.clone();
    let options = options
        .handler("sessionArchive.get", move |_, _| {
            let service = get.clone();
            async move { Ok(service.get()) }
        })
        .handler("sessionArchive.set", move |_, params| {
            let service = set_archive.clone();
            let open = set_open.clone();
            let live = set_live.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let before = service.get()["version"].clone();
                    let document = service.set(params.clone())?;
                    // Only an accepted change moves the sidebar.
                    if document["version"] != before
                        && let Some(id) = params["sessionId"].as_str()
                    {
                        if params["archived"] == true {
                            open.forget(id);
                        } else {
                            open.restore(id, live(id));
                        }
                    }
                    Ok(document)
                })
                .await?
            }
        })
        // Bulk Hide/Unhide as ONE archive change (a cleanup's Undo).
        .handler("sessionArchive.setMany", move |_, params| {
            let service = many_archive.clone();
            let open = many_open.clone();
            let live = many_live.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let (document, changed) = service.set_many(params.clone())?;
                    if params["archived"] == true {
                        open.forget_many(&changed);
                    } else {
                        open.restore_many(&changed, |id| live(id));
                    }
                    Ok(document)
                })
                .await?
            }
        });
    let get = open.clone();
    let kept = open.clone();
    let snooze = open.clone();
    let cleanup_open = open.clone();
    let cleanup_archive = archive.clone();
    let options = options
        .handler("sessionOpen.get", move |_, _| {
            let service = get.clone();
            async move { Ok(service.get()) }
        })
        .handler("sessionOpen.set", move |_, params| {
            let service = open.clone();
            async move { tokio::task::spawn_blocking(move || service.set(params)).await? }
        })
        .handler("sessionOpen.snoozeCleanup", move |_, _| {
            let service = snooze.clone();
            async move { tokio::task::spawn_blocking(move || service.snooze_cleanup()).await? }
        })
        // Hide every counted ended session (optionally only those whose end
        // is at least `olderThanMs` old) as ONE archive change; never Paused
        // or live sessions. `dryRun` only reports.
        .handler("sessionOpen.cleanup", move |_, params| {
            let open = cleanup_open.clone();
            let archive = cleanup_archive.clone();
            async move {
                let older = match &params["olderThanMs"] {
                    serde_json::Value::Null => None,
                    value => Some(value.as_i64().filter(|ms| *ms >= 0).ok_or_else(|| {
                        anyhow::anyhow!(
                            "sessionOpen.cleanup: olderThanMs must be a non-negative number"
                        )
                    })?),
                };
                let dry_run = match &params["dryRun"] {
                    serde_json::Value::Null => false,
                    value => value.as_bool().ok_or_else(|| {
                        anyhow::anyhow!("sessionOpen.cleanup: dryRun must be a boolean")
                    })?,
                };
                tokio::task::spawn_blocking(move || {
                    let ids = open.cleanup_candidates(older);
                    if dry_run {
                        return Ok(serde_json::json!({"wouldHide": ids}));
                    }
                    let (_, hidden) = archive.set_ids(&ids, true)?;
                    open.forget_many(&hidden);
                    Ok(serde_json::json!({"hidden": hidden}))
                })
                .await?
            }
        })
        // Historical shape (`{sessions:[{sessionId,since}]}`, newest first),
        // now served from the hub's open set: open, unended ids.
        .handler("sessions.kept", move |_, _| {
            let service = kept.clone();
            async move { Ok(service.kept()) }
        });
    let mut prefs = usage_prefs::UsagePrefs::open(Some(directory.join("usage-pacing.json")));
    if let Some(routing) = options.routing.clone() {
        prefs = prefs.with_routing(routing);
    }
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
    // Serialized in memory and written once: serde writes token by token, and
    // straight to an unbuffered file that is a syscall each, which took about
    // a second per save of a 2 MB launch journal on Windows.
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    atomic_persist::persist(file, path)?;
    Ok(())
}

pub mod live_streams;
pub(crate) mod owned_process;
