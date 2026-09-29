use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};
use workspacer_hub::{
    Caller, Hub, Options,
    auth::{self, Record},
    services::{
        agent_lifecycle::Operation,
        progress::Progress,
        remote_dispatch::{
            Capabilities, Directory, Execution, Provider, Receiver, RemoteAdmission,
        },
        routing, spawn_plan,
        wakes::Wakes,
    },
    test_support,
};
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    non_repo: PathBuf,
    config: Value,
    rows: Mutex<BTreeMap<String, Value>>,
    replies: Mutex<BTreeMap<String, String>>,
    launches: Mutex<Vec<Value>>,
    last: Mutex<String>,
    receiver: Mutex<Weak<Receiver>>,
    wakes: Mutex<Weak<Wakes>>,
    progress: Mutex<Weak<Progress>>,
    replay: AtomicUsize,
    clock: AtomicI64,
}
impl Fixture {
    fn receiver(&self) -> Result<Arc<Receiver>> {
        self.receiver
            .lock()
            .unwrap()
            .upgrade()
            .context("fixture receiver unavailable")
    }
    fn lookup(&self, id: &str) -> Option<Value> {
        self.rows.lock().unwrap().get(id).cloned()
    }
    fn observe(&self, row: Value) {
        let now = self.clock.fetch_add(1, Ordering::SeqCst);
        self.rows
            .lock()
            .unwrap()
            .insert(row["sessionId"].as_str().unwrap().into(), row.clone());
        if let Some(wakes) = self.wakes.lock().unwrap().upgrade() {
            wakes.observe(&row, now);
        }
    }
}
impl Execution for Fixture {
    fn capabilities(&self) -> Operation<'_, Capabilities> {
        Box::pin(async move {
            Ok(Capabilities {
                protocol: 2,
                exact_model: true,
                executes: true,
                scope: "operator".into(),
                providers: vec![
                    Provider {
                        provider: "claude".into(),
                        found: true,
                        authenticated: Some(true),
                        note: "fake authenticated provider; no model process".into(),
                    },
                    Provider {
                        provider: "codex".into(),
                        found: false,
                        authenticated: Some(false),
                        note: "missing fixture provider".into(),
                    },
                ],
                cwds: vec![
                    Directory {
                        path: self.repo.to_string_lossy().into_owned(),
                        source: "project".into(),
                        git: true,
                    },
                    Directory {
                        path: self.non_repo.to_string_lossy().into_owned(),
                        source: "project".into(),
                        git: false,
                    },
                ],
                unsupported_reason: None,
            })
        })
    }
    fn canonical_directory<'a>(&'a self, cwd: &'a str) -> Operation<'a, String> {
        Box::pin(async move {
            let path = Path::new(cwd).canonicalize()?;
            anyhow::ensure!(
                path.starts_with(&self.root) && path.is_dir(),
                "fixture directory escaped scratch root"
            );
            Ok(path.to_string_lossy().into_owned())
        })
    }
    fn allocate<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            test_support::git(Path::new(repo), &["worktree", "add", "-b", branch, cwd]).await?;
            Ok(())
        })
    }
    fn cleanup<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            test_support::git(Path::new(repo), &["worktree", "remove", cwd]).await?;
            test_support::git(Path::new(repo), &["branch", "-D", branch]).await?;
            Ok(())
        })
    }
    fn spawn<'a>(
        &'a self,
        _caller: Caller,
        admission: RemoteAdmission,
        params: Value,
    ) -> Operation<'a, Value> {
        Box::pin(async move {
            // The production resolver forms exactly the daemon request. Only the
            // daemon's provider-process boundary is replaced by this recording sink.
            let plan = spawn_plan::resolve(
                &params,
                &self.config,
                None,
                &self.root,
                admission.session_id(),
                true,
            )?;
            anyhow::ensure!(
                plan.provider == admission.provider() && plan.request["cwd"] == admission.cwd(),
                "fixture plan changed sealed admission"
            );
            self.launches.lock().unwrap().push(plan.request.clone());
            *self.last.lock().unwrap() = admission.session_id().into();
            self.observe(json!({"sessionId":admission.session_id(),"cwd":admission.cwd(),"provider":admission.provider(),"status":"active","ambientState":"streaming","lastActivity":self.clock.load(Ordering::SeqCst),"resultSchema":params["resultSchema"],"remoteOrigin":admission.origin()}));
            plan.receipt(&json!({"session_id":admission.session_id(),"first_message_queued":true}))
        })
    }
    fn blocked<'a>(&'a self, id: &'a str) -> Operation<'a, Option<bool>> {
        Box::pin(async move { Ok(self.lookup(id).map(|row| row["pendingApproval"] == true)) })
    }
}
async fn evidence(State(state): State<Arc<Fixture>>) -> Json<Value> {
    Json(json!(state.launches.lock().unwrap().clone()))
}
async fn control(State(state): State<Arc<Fixture>>, body: Bytes) -> axum::response::Response {
    let result: Result<Value> = async {
        let command: Value = serde_json::from_slice(&body)?;
        let kind = command["kind"].as_str().unwrap_or("");
        if kind.starts_with("replay-") {
            let mode = match kind {
                "replay-known" => 0,
                "replay-unknown" => 1,
                "replay-error" => 2,
                "replay-wrong-id" => 3,
                _ => anyhow::bail!("unknown replay mode"),
            };
            state.replay.store(mode, Ordering::SeqCst);
            return Ok(json!({"ok":true}));
        }
        let id = state.last.lock().unwrap().clone();
        anyhow::ensure!(!id.is_empty(), "no fixture worker");
        state
            .replies
            .lock()
            .unwrap()
            .insert(id.clone(), command["reply"].as_str().unwrap_or("").into());
        if kind == "progress" {
            let progress = state
                .progress
                .lock()
                .unwrap()
                .upgrade()
                .context("progress service unavailable")?;
            progress
                .report(
                    json!({"callerSessionId":id,"note":"remote fixture progress"}),
                    state.clock.fetch_add(60001, Ordering::SeqCst),
                )
                .await?;
        } else {
            let mut row = state.lookup(&id).context("fixture session missing")?;
            row["ambientState"] = "streaming".into();
            row["lastActivity"] = state.clock.load(Ordering::SeqCst).into();
            state.observe(row.clone());
            row["ambientState"] = if kind == "block" {
                "waiting_approval"
            } else {
                "idle"
            }
            .into();
            row["pendingApproval"] = (kind == "block").into();
            state.observe(row);
            let now = state.clock.fetch_add(30000, Ordering::SeqCst);
            let wakes = state
                .wakes
                .lock()
                .unwrap()
                .upgrade()
                .context("wake observer unavailable")?;
            wakes.tick(now + 25000).await?;
            wakes.tick(now + 28000).await?;
        }
        Ok(json!({"ok":true}))
    }
    .await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
pub async fn run(scratch: &Path) -> Result<()> {
    anyhow::ensure!(
        std::env::var("WKS_PAIRED_CHAIN_FIXTURE").as_deref() == Ok("1"),
        "paired fixture opt-in required"
    );
    let temporary = tempfile::Builder::new()
        .prefix("paired-rust-")
        .tempdir_in(scratch)?;
    let root = temporary.path().canonicalize()?;
    let repo = root.join("remote-repo");
    let non_repo = root.join("remote-non-repo");
    std::fs::create_dir_all(repo.join("frontend"))?;
    std::fs::create_dir_all(repo.join("backend"))?;
    std::fs::create_dir(&non_repo)?;
    std::fs::write(repo.join("frontend/tracked.ts"), "fixture\n")?;
    std::fs::write(repo.join("backend/tracked.go"), "fixture\n")?;
    test_support::git(&repo, &["init", "-q"]).await?;
    test_support::git(&repo, &["add", "."]).await?;
    test_support::git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-qm",
            "fixture",
        ],
    )
    .await?;
    let mut config = workspacer_hub::services::config::defaults();
    config["agents"]["binaries"] =
        json!({"claude":root.join("never-executed-provider"),"codex":root.join("missing-codex")});
    config["projects"] =
        json!({repo.to_string_lossy().as_ref():{},non_repo.to_string_lossy().as_ref():{}});
    std::fs::write(root.join("config.yaml"), serde_yaml::to_string(&config)?)?;
    std::fs::write(
        root.join("routing.yaml"),
        "active_profile: anthropic_only\n",
    )?;
    let state = Arc::new(Fixture {
        root: root.clone(),
        repo: repo.clone(),
        non_repo: non_repo.clone(),
        config: config.clone(),
        rows: Mutex::new(BTreeMap::new()),
        replies: Mutex::new(BTreeMap::new()),
        launches: Mutex::new(vec![]),
        last: Mutex::new(String::new()),
        receiver: Mutex::new(Weak::new()),
        wakes: Mutex::new(Weak::new()),
        progress: Mutex::new(Weak::new()),
        replay: AtomicUsize::new(0),
        clock: AtomicI64::new(chrono::Utc::now().timestamp_millis()),
    });
    let token_file = root.join("tokens.json");
    auth::save(
        &token_file,
        &[
            Record {
                token: "paired-fixture-operator".into(),
                scope: "operator".into(),
                ..Default::default()
            },
            Record {
                token: "paired-fixture-view".into(),
                scope: "view".into(),
                ..Default::default()
            },
        ],
    )?;
    let mut options = Options::default();
    options.token = "paired-fixture-host".into();
    options.scoped_tokens = Some(token_file);
    options.control_plane_only = true;
    options.listen = Some("127.0.0.1:0".parse()?);
    options.config_dir = Some(root.clone());
    options.data_dir = Some(root.join("hub-state"));
    options.push_dir = Some(root.join("push"));
    options = test_support::with_execution(options, state.clone());
    let routing = Arc::new(routing::RoutingService::open(root.clone())?);
    let callbacks = state.clone();
    options = test_support::after_services(options, move |options| {
        let state = callbacks.clone();
        let route = routing.clone();
        let options = options.handler("agents.dispatchReplay", move |caller, params| {
            let state = state.clone();
            async move {
                match state.replay.load(Ordering::SeqCst) {
                    1 => Ok(json!({"state":"unknown","dispatchId":params["dispatchId"]})),
                    2 => anyhow::bail!("fixture replay transport failure"),
                    3 => Ok(json!({"state":"unknown","dispatchId":"unrelated-dispatch-nonce"})),
                    _ => state.receiver()?.replay(&caller, params).await,
                }
            }
        });
        let options = options.handler("routing.select", move |_, params| {
            let route = route.clone();
            async move {
                route.select(
                    params,
                    &json!({"providers":[]}),
                    chrono::Utc::now().timestamp(),
                )
            }
        });
        let mut options = options.handler("config.get", {
            let config = config.clone();
            move |_, _| {
                let config = config.clone();
                async move { Ok(config) }
            }
        });
        for method in [
            "sessions.snapshot",
            "sessions.snapshots",
            "agents.list",
            "sessions.conversation",
        ] {
            let state = callbacks.clone();
            options=options.handler(method,move|_,params|{let state=state.clone();async move{Ok(match method{"sessions.snapshot"=>state.lookup(params["sessionId"].as_str().unwrap_or("")).unwrap_or(Value::Null),"sessions.conversation"=>json!({"items":[{"kind":"user_message","text":"fixture task"},{"kind":"assistant_text","text":state.replies.lock().unwrap().get(params["sessionId"].as_str().unwrap_or("")).cloned().unwrap_or_default()}]}),_=>json!(state.rows.lock().unwrap().values().cloned().collect::<Vec<_>>())})}});
        }
        options
    });
    let hub = Hub::start(options)?;
    let address = hub.ready().await?.unwrap();
    let receiver = test_support::receiver(&hub.handle()).await?;
    *state.receiver.lock().unwrap() = Arc::downgrade(&receiver);
    let lookup: workspacer_hub::services::task_store::OwnerLookup = {
        let state = state.clone();
        Arc::new(move |id| state.lookup(id))
    };
    let list = {
        let state = state.clone();
        Arc::new(move || state.rows.lock().unwrap().values().cloned().collect())
            as workspacer_hub::services::wakes::Inventory
    };
    let capture = {
        let state = state.clone();
        Arc::new(move |id: String| {
            let state = state.clone();
            Box::pin(async move {
                Ok(
                    json!({"items":[{"kind":"user_message","text":"fixture task"},{"kind":"assistant_text","text":state.replies.lock().unwrap().get(&id).cloned().unwrap_or_default()}]}),
                )
            }) as futures_util::future::BoxFuture<'static, Result<Value>>
        }) as workspacer_hub::services::wakes::Capture
    };
    let wakes = test_support::wake_returns(
        Wakes::new(
            lookup.clone(),
            list,
            capture,
            Arc::new(|_, _, _| {
                Box::pin(async { anyhow::bail!("remote fixture must never deliver locally") })
            }),
            None,
            None,
        ),
        receiver.clone(),
    );
    *state.wakes.lock().unwrap() = Arc::downgrade(&wakes);
    let progress = Arc::new(test_support::progress_returns(
        Progress::new(
            lookup,
            Arc::new(|_, _| {
                Box::pin(async { anyhow::bail!("remote fixture must never deliver locally") })
            }),
        ),
        receiver,
    ));
    *state.progress.lock().unwrap() = Arc::downgrade(&progress);
    let mut old = Options::default();
    old.token = "paired-fixture-operator".into();
    old.control_plane_only = true;
    old.listen = Some("127.0.0.1:0".parse()?);
    old.push_dir = Some(root.join("old-push"));
    let old = Hub::start(old)?;
    let old_address = old.ready().await?.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let control_address = listener.local_addr()?;
    let api = Router::new()
        .route("/control", post(control))
        .route("/evidence", get(evidence))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(state);
    let server = tokio::spawn(async move { axum::serve(listener, api).await });
    println!(
        "{}",
        json!({"url":format!("http://{address}"),"oldURL":format!("http://{old_address}"),"repo":repo,"nonRepo":non_repo,"control":format!("http://{control_address}")})
    );
    super::wait_for_parent().await;
    server.abort();
    let _ = server.await;
    wakes.close();
    old.shutdown()?;
    hub.shutdown()?;
    drop(progress);
    drop(wakes);
    drop(temporary);
    Ok(())
}
