//! MCP transport adapter over the same in-memory capability runtime.
//! The catalog is the Go reference's schema, filtered to migrated handlers.
pub(crate) mod access;
pub use access::UntokenedAccess;
mod config_save;
mod conversation;
mod fleet;
mod gate;
mod help;
mod legacy_sse;
mod manager_context;
mod plugin_catalog;
#[cfg(test)]
mod presentation;
mod project_status;
mod raw_preferences;
mod respawn;
mod spawn;
mod status_summary;
mod ui;
mod wire;
mod workflow_dispatch;
mod workflows;
use crate::{
    Handle,
    auth::{Scope, Store},
    client::Client,
};
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult,
        ContentBlock as Content, Implementation, ListPromptsResult, ListResourceTemplatesResult,
        ListResourcesResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
struct Identity {
    token: String,
    scope: Scope,
    session_id: String,
    ephemeral: Option<access::Lease>,
}
#[derive(Clone)]
struct Adapter {
    upstream: Option<Arc<crate::provider_relay::UpstreamCaller>>,
    hub: Handle,
    plugins: Arc<plugin_catalog::Catalog>,
}
#[derive(Clone)]
struct Gate {
    policy: crate::server::policy::Policy,
    token: String,
    store: Option<PathBuf>,
    access: Arc<access::Policy>,
}
impl Identity {
    async fn connection(
        &self,
        hub: &Handle,
        facade: bool,
        plugin: Option<&str>,
    ) -> anyhow::Result<crate::Connection> {
        if let Some(lease) = &self.ephemeral {
            let lease = if facade {
                lease.for_facade()
            } else {
                lease.clone()
            };
            let lease = plugin
                .map(|method| lease.for_plugin(method))
                .unwrap_or(lease);
            hub.connect_ephemeral_facade(lease).await
        } else if facade {
            hub.connect_facade(self.token.clone()).await
        } else {
            hub.connect_authenticated(self.token.clone(), false).await
        }
    }
}
impl Gate {
    fn resolve(&self, token: Option<String>) -> Option<Identity> {
        let Some(token) = token else {
            let lease = self.access.guest()?;
            return Some(Identity {
                token: String::new(),
                scope: lease.scope(),
                session_id: String::new(),
                ephemeral: Some(lease),
            });
        };
        if token.is_empty() {
            return None;
        }
        if !self.token.is_empty() && crate::auth::credential_eq(&self.token, &token) {
            return Some(Identity {
                token,
                scope: Scope::Operator,
                session_id: String::new(),
                ephemeral: None,
            });
        }
        if let Some(lease) = self.access.static_credential(&token) {
            return Some(Identity {
                token,
                scope: lease.scope(),
                session_id: String::new(),
                ephemeral: Some(lease),
            });
        }
        if let Some(record) = self
            .store
            .as_ref()
            .and_then(|path| Store { path: path.clone() }.lookup(&token))
        {
            return Some(Identity {
                token,
                scope: record.scope()?,
                session_id: record
                    .label
                    .strip_prefix("session:")
                    .unwrap_or("")
                    .trim()
                    .into(),
                ephemeral: None,
            });
        }
        None
    }
}

fn catalogs() -> &'static BTreeMap<String, Vec<Tool>> {
    static CATALOG: OnceLock<BTreeMap<String, Vec<Tool>>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/mcp-effective-tools.json"))
            .expect("generated Rust MCP catalog")
    })
}
fn method(name: &str) -> Option<&'static str> {
    if ui::topic(name).is_some() {
        return Some("__facade.ui");
    }
    if workflows::op(name).is_some() {
        return Some("fleetWorkflows.request");
    }
    Some(match name {
        "help" => "__facade.help",
        "spawn_agent" => "agents.spawn",
        "notify_when" => "agents.notifyWhen",
        "close_session" => "agents.close",
        "list_orphans" => "agents.orphans",
        "adopt_workers" => "agents.reparent",
        "brief_append" => "brief.append",
        "brief_archive" => "brief.archive",
        "brief_check" => "brief.check",
        "manager_context" => "fleetWorkflows.request",
        "dispatch_workflow_step" => "agents.spawn",
        "respawn_with" => "agents.spawn",
        "project_status" => "git.status",
        "analytics_summary" => "analytics.summary",
        "analytics_recent" => "analytics.recent",
        "create_terminal" => "terminals.create",
        "open_terminal" => "terminals.open",
        "notify" => "notifications.post",
        "list_dispatches" => "fleet.dispatches",
        "list_resumable_sessions" => "claude.sessionsForDir",
        "set_approval_gate" => "claude.gate",
        "summarize_agent_status" => "agents.summarizeStatus",
        "list_dispatch_targets" => "fleet.dispatchTargets",
        "select_dispatch_model" => "fleet.selectDispatchModel",
        "get_snapshot" => "sessions.snapshot",
        "list_agents" => "agents.list",
        "get_conversation" => "sessions.conversation",
        "list_snapshots" => "sessions.snapshots",
        "get_transcript" => "sessions.transcript",
        "get_host_cwd" => "app.getCwd",
        "read_file" => "fs.read",
        "write_file" => "fs.write",
        "list_dir" => "fs.listDir",
        "list_entries" => "fs.listEntries",
        "search_project" => "search.project",
        "get_config" => "config.get",
        "get_config_path" => "config.getPath",
        "save_config" => "config.save",
        "reload_config" => "config.reload",
        "list_profiles" => "claude.profiles.list",
        "add_profile" => "claude.profiles.add",
        "update_profile" => "claude.profiles.update",
        "remove_profile" => "claude.profiles.remove",
        "list_models" => "claude.listModels",
        "list_providers" => "providers.checkAll",
        "list_provider_models" => "providers.listModels",
        "list_layouts" => "layouts.list",
        "save_layout" => "layouts.save",
        "delete_layout" => "layouts.delete",
        "list_saved_sessions" => "sessions.list",
        "load_saved_session" => "sessions.load",
        "save_saved_session" => "sessions.save",
        "delete_saved_session" => "sessions.delete",
        "list_library" => "library.list",
        "save_library" => "library.save",
        "remove_library" => "library.remove",
        "approve" => "claude.approve",
        "signal" => "claude.signal",
        "answer" => "claude.answer",
        "send_message" => "agents.sendMessage",
        "report_progress" => "agents.reportProgress",
        "set_model" => "claude.setModel",
        "terminal_input" => "sessions.terminalInput",
        "terminal_resize" => "sessions.terminalResize",
        "select_model" => "routing.select",
        "routing_preview" => "routing.preview",
        "routing_preferences_get" => "routing.preferences.get",
        "routing_preferences_validate" => "routing.preferences.validate",
        "routing_preferences_save" => "routing.preferences.save",
        "routing_preferences_reset" => "routing.preferences.reset",
        "list_jobs" => "jobs.list",
        "job_history" => "jobs.history",
        "propose_job" => "jobs.propose",
        "run_job" => "jobs.run",
        "remove_job" => "jobs.remove",
        _ => return None,
    })
}
/// Migration diagnostics only: a mapped facade still needs its host handler
/// and behavioral tests. This performs no network or service startup.
pub fn migration_inventory() -> Value {
    let tools: Vec<_> = catalogs()
        .get("operator")
        .into_iter()
        .flatten()
        .map(|tool| json!({"name":tool.name,"method":method(&tool.name)}))
        .collect();
    let unmapped: Vec<_> = tools
        .iter()
        .filter(|tool| tool["method"].is_null())
        .map(|tool| tool["name"].clone())
        .collect();
    json!({"note":"Mapping coverage only; this is not runtime availability or behavior parity.","total":tools.len(),"mapped":tools.len()-unmapped.len(),"unmapped":unmapped,"tools":tools})
}
pub(crate) fn supports_owned_launch() -> bool {
    [
        "spawn_agent",
        "list_agents",
        "get_conversation",
        "send_message",
        "report_progress",
        "select_model",
        "list_workflows",
    ]
    .iter()
    .all(|name| method(name).is_some())
}
fn identity(context: &RequestContext<RoleServer>) -> Result<Identity, ErrorData> {
    context
        .extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<Identity>())
        .cloned()
        .filter(|identity| {
            identity
                .ephemeral
                .as_ref()
                .is_none_or(|lease| lease.valid())
        })
        .ok_or_else(|| ErrorData::invalid_request("unauthorized", None))
}
impl ServerHandler for Adapter {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("workspacer", "0.1.0"))
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        if method(name).is_none() {
            return self.plugins.tool(name).map(|entry| entry.0);
        }
        catalogs()["operator"]
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .or_else(|| self.plugins.tool(name).map(|entry| entry.0))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let identity = identity(&context)?;
        if !self.plugins.ready() {
            self.plugins
                .refresh(&self.hub)
                .await
                .map_err(|_| ErrorData::internal_error("plugin catalog unavailable", None))?;
        }
        let health = if let Some(upstream) = &self.upstream {
            if !upstream.connected() {
                return Err(ErrorData::internal_error(
                    "upstream facade unavailable",
                    None,
                ));
            }
            json!({})
        } else {
            self.hub
                .health()
                .await
                .map_err(|_| ErrorData::internal_error("hub unavailable", None))?
        };
        let registered = health["methodNames"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // MCP 2026-07-28 requires both cache hints. rmcp's list defaults omit
        // them, unlike server/discover. Catalogs depend on caller identity and
        // live registrations, so never share or retain them across requests.
        let mut result = ListToolsResult::default()
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private);
        result.tools = catalogs()
            .get(identity.scope.name())
            .into_iter()
            .flatten()
            .filter(|tool| {
                matches!(tool.name.as_ref(), "help" | "summarize_agent_status")
                    || ui::topic(&tool.name).is_some()
                    || method(&tool.name).is_some_and(|m| {
                        self.upstream.is_some() || registered.iter().any(|v| v == m)
                    })
            })
            .cloned()
            .collect();
        if identity.scope != Scope::Provider {
            result.tools.extend(self.plugins.tools());
        }
        Ok(result)
    }
    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::default()
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }
    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::default()
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::default()
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let identity = identity(&context)?;
        let allowed = catalogs()
            .get(identity.scope.name())
            .is_some_and(|tools| tools.iter().any(|tool| tool.name == request.name));
        let resolved = method(&request.name)
            .filter(|_| allowed)
            .map(|method| (method.to_owned(), false))
            .or_else(|| {
                (identity.scope != Scope::Provider)
                    .then(|| self.plugins.tool(&request.name))
                    .flatten()
                    .map(|(_, method)| (method, true))
            });
        let Some((method, plugin)) = resolved else {
            return Err(ErrorData::invalid_params(
                "unknown or unavailable tool",
                None,
            ));
        };
        let mut params = Value::Object(request.arguments.unwrap_or_default());
        if let Some(tool) = self.get_tool(&request.name) {
            let validator = jsonschema::validator_for(&Value::Object((*tool.input_schema).clone()))
                .map_err(|_| ErrorData::internal_error("tool schema is unavailable", None))?;
            if let Err(error) = validator.validate(&params) {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "invalid tool arguments: {error}"
                ))])
                .into());
            }
        }
        if !plugin {
            wire::project(&request.name, &mut params);
        }
        if request.name == "save_config" {
            if let Some(path) = config_save::invalid_wholesale(&params) {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "save_config refused: {path} is REPLACED wholesale and must be a JSON object. Nothing was written. Send the full map to retain, or {{}} to empty it."
                ))]).into());
            }
        }
        if request.name == "help" {
            let tools = self.list_tools(None, context).await?.tools;
            return Ok(CallToolResult::success(vec![Content::text(help::render(
                identity.scope.name(),
                &tools,
                params["topic"].as_str().unwrap_or(""),
            ))])
            .into());
        }
        if let Some(event) = ui::event(&request.name, &params) {
            let mut connection =
                identity
                    .connection(&self.hub, false, None)
                    .await
                    .map_err(|_| {
                        ErrorData::invalid_request("tool credential is no longer authorized", None)
                    })?;
            if !connection.recv().await.is_some_and(|hello| {
                hello.op == "hello" && matches!(hello.scope.as_str(), "triage" | "operator")
            }) {
                return Err(ErrorData::invalid_request(
                    "UI actions require triage or operator authority",
                    None,
                ));
            }
            let published = if let Some(upstream) = &self.upstream {
                match upstream.client().await {
                    Ok(client) => client.publish(event).await,
                    Err(error) => Err(error),
                }
            } else {
                self.hub.publish_wait(event).await
            };
            let result = match published {
                Ok(()) => CallToolResult::success(vec![Content::text("ok")]),
                Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
            };
            return Ok(result.into());
        }
        let progress = request.name == "report_progress";
        if progress {
            params
                .as_object_mut()
                .unwrap()
                .retain(|key, _| !key.eq_ignore_ascii_case("callerSessionId"));
            params["callerSessionId"] = identity.session_id.clone().into();
        }
        let workflow = workflows::op(&request.name).is_some();
        let spawning = request.name == "spawn_agent";
        let manager_context = request.name == "manager_context";
        let dispatching = request.name == "dispatch_workflow_step";
        let respawning = request.name == "respawn_with";
        if spawning {
            if let Err(error) = spawn::prepare(&mut params, &identity.session_id) {
                return Ok(CallToolResult::error(vec![Content::text(error.to_string())]).into());
            }
        }
        let compact = workflow && params["compact"] == true;
        if workflow {
            if let Err(error) = workflows::prepare(&request.name, &mut params, &identity.session_id)
            {
                return Ok(CallToolResult::error(vec![Content::text(error)]).into());
            }
        }
        let conversation = request.name == "get_conversation";
        let last_message = conversation && params["lastMessage"] == true;
        let text_only = conversation && params["textOnly"] == true;
        if conversation {
            params.as_object_mut().unwrap().remove("lastMessage");
            params.as_object_mut().unwrap().remove("textOnly");
        }
        let fleet = matches!(request.name.as_ref(), "list_agents" | "list_snapshots");
        let routed = matches!(
            request.name.as_ref(),
            "get_snapshot"
                | "get_transcript"
                | "get_conversation"
                | "approve"
                | "signal"
                | "answer"
                | "send_message"
                | "set_model"
                | "spawn_agent"
        );
        let method = if routed && let Some(peer) = params["hub"].as_str().filter(|s| !s.is_empty())
        {
            format!("hub:{peer}/{method}")
        } else {
            method
        };
        if routed {
            params.as_object_mut().unwrap().remove("hub");
        }
        // Scope is revalidated while opening this connection, not trusted from
        // an earlier initialize request or a user-supplied session identifier.
        let result = async {
            if let Some(lease) = &identity.ephemeral {
                anyhow::ensure!(lease.valid(), "facade access policy changed");
                let lease = if spawning || workflow || manager_context || dispatching || respawning
                {
                    lease.for_facade()
                } else {
                    lease.clone()
                };
                let bare = method
                    .strip_prefix("hub:")
                    .and_then(|name| name.split_once('/').map(|(_, method)| method))
                    .unwrap_or(&method);
                params = crate::admission::sanitize(&lease.identity()?, bare, params)?;
                if plugin {
                    anyhow::ensure!(
                        lease.for_plugin(&method).allows_plugin(&method),
                        "plugin method requires authenticated host authority"
                    );
                }
            }
            let _upstream_authorization;
            let client = if let Some(upstream) = &self.upstream {
                let mut connection = identity.connection(&self.hub, false, None).await?;
                anyhow::ensure!(
                    connection.recv().await.is_some_and(
                        |hello| hello.op == "hello" && hello.scope == identity.scope.name()
                    ),
                    "tool credential changed or is no longer authorized"
                );
                _upstream_authorization = Some(connection);
                upstream.client().await?
            } else {
                _upstream_authorization = None;
                if identity.ephemeral.is_some() {
                    Client::from_connection(
                        identity
                            .connection(
                                &self.hub,
                                spawning
                                    || workflow
                                    || manager_context
                                    || dispatching
                                    || respawning,
                                plugin.then_some(method.as_str()),
                            )
                            .await?,
                    )
                } else if spawning || workflow || manager_context || dispatching || respawning {
                    Client::from_connection(identity.connection(&self.hub, true, None).await?)
                } else {
                    let mut connection = identity.connection(&self.hub, false, None).await?;
                    if plugin || progress || method.starts_with("jobs.") {
                        // Explicit facade delegation: plugin tools use their declared
                        // namespace; progress carries the authenticated session label;
                        // jobs expose propose/run/remove but no enable/upsert tool.
                        // Re-check the current credential before using the host.
                        anyhow::ensure!(
                            connection
                                .recv()
                                .await
                                .is_some_and(|hello| hello.op == "hello"
                                    && if plugin || progress {
                                        matches!(
                                            hello.scope.as_str(),
                                            "view" | "triage" | "operator"
                                        )
                                    } else {
                                        hello.scope == "operator"
                                    }),
                            "tool credential is no longer authorized"
                        );
                        Client::connect(&self.hub).await?
                    } else {
                        Client::from_connection(connection)
                    }
                }
            };
            let value = if request.name == "summarize_agent_status" {
                status_summary::call(&client, params.clone()).await?
            } else if request.name == "set_approval_gate" {
                gate::call(&client, params.clone()).await?
            } else if request.name == "project_status" {
                project_status::call(&client, params.clone()).await?
            } else if respawning {
                respawn::call(&client, params.clone(), &identity.session_id).await?
            } else if dispatching {
                workflow_dispatch::call(&client, params.clone(), &identity.session_id).await?
            } else if manager_context {
                manager_context::call(&client, params.clone(), &identity.session_id).await?
            } else if spawning {
                spawn::call(&client, &method, params.clone()).await?
            } else {
                client
                    .call_with_timeout(
                        &method,
                        params.clone(),
                        crate::protocol::provider_timeout(
                            &method,
                            std::time::Duration::from_secs(30),
                        ),
                    )
                    .await?
            };
            let value = if fleet {
                fleet::merge(&client, &method, params, value).await
            } else {
                value
            };
            let value = if compact {
                workflows::compact(value)
            } else {
                value
            };
            Ok::<_, anyhow::Error>(conversation::reduce(value, last_message, text_only))
        }
        .await;
        let result = match result {
            Ok(value) if dispatching && value["ok"] == false => {
                CallToolResult::error(vec![Content::text(value.to_string())])
            }
            Ok(value) => CallToolResult::success(vec![Content::text(if value.is_null() {
                "ok".into()
            } else {
                value.to_string()
            })]),
            Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
        };
        Ok(result.into())
    }
}

async fn authenticate(State(gate): State<Gate>, mut request: Request, next: Next) -> Response {
    let local = request
        .extensions()
        .get::<axum::extract::ConnectInfo<crate::server::policy::Socket>>()
        .and_then(|socket| socket.0.local);
    if !gate.policy.host(request.headers(), local) || !gate.policy.origin(request.headers(), local)
    {
        return (StatusCode::FORBIDDEN, "host or origin not allowed").into_response();
    }
    let query = request.uri().query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(key, _)| key == "t")
            .map(|(_, v)| v.into_owned())
    });
    let token = request
        .headers()
        .get("authorization")
        .map(|header| {
            header
                .to_str()
                .ok()
                .and_then(|h| h.strip_prefix("Bearer "))
                .unwrap_or("")
                .to_owned()
        })
        .or(query);
    let Some(identity) = gate.resolve(token) else {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    };
    request.extensions_mut().insert(identity);
    if request.method() == axum::http::Method::POST {
        let (parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request body unavailable or too large",
                )
                    .into_response();
            }
        };
        if parts.uri.path() != "/sse"
            && let Some((id, error)) = raw_preferences::validate(&bytes)
        {
            return axum::Json(json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":error}]}})).into_response();
        }
        request = Request::from_parts(parts, axum::body::Body::from(bytes));
    }
    next.run(request).await
}

pub(crate) async fn serve(
    listener: tokio::net::TcpListener,
    hub: Handle,
    token: String,
    store: Option<PathBuf>,
    ready: Arc<AtomicBool>,
    trusted_hosts: Vec<String>,
    upstream: Option<Arc<crate::provider_relay::UpstreamCaller>>,
    access: Arc<access::Policy>,
) -> anyhow::Result<()> {
    struct ReadyGuard(Arc<AtomicBool>);
    impl Drop for ReadyGuard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _ready_guard = ReadyGuard(ready.clone());
    let address = listener.local_addr()?;
    let policy = crate::server::policy::Policy::new(address.ip(), &trusted_hosts)?;
    let plugins = Arc::new(plugin_catalog::Catalog::with_readiness(
        ready,
        upstream.clone(),
    ));
    let adapter = Adapter {
        hub: hub.clone(),
        plugins: plugins.clone(),
        upstream: upstream.clone(),
    };
    let legacy = legacy_sse::LegacySse::new(
        adapter.clone(),
        Gate {
            token: token.clone(),
            store: store.clone(),
            policy: policy.clone(),
            access: access.clone(),
        },
    );
    let service = StreamableHttpService::new(
        move || Ok(adapter.clone()),
        Arc::new(LocalSessionManager::default()),
        // Our outer socket-aware policy and authenticated gate validate Host
        // and Origin for all MCP transports. RMCP's static loopback-only Host
        // allowlist would independently reject an explicitly trusted proxy.
        StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .merge(legacy.router())
        .layer(middleware::from_fn_with_state(
            Gate {
                token,
                store,
                policy: policy.clone(),
                access,
            },
            authenticate,
        ));
    // Keep the existing facade identity and report the actual hub endpoint;
    // the transport field identifies how this adapter reaches that hub.
    let catalog_hub = hub.clone();
    let health_catalog = plugins.clone();
    let health=Router::new().route("/health",axum::routing::get(move ||{let hub=hub.clone();let upstream=upstream.clone();let plugins=health_catalog.clone();async move{
        let launch_ready=hub.health().await.ok().is_some_and(|health|health["launchReady"]==true);
        let hub_bus_url=if let Some(upstream)=&upstream{Some(upstream.url().to_owned())}else{match *hub.status().borrow(){crate::Status::Ready{address:Some(address),..}=>{let address=crate::net_address::dial_addr(address);Some(format!("ws://{address}/bus"))},_=>None}};
        axum::Json(json!({"status":"ok","service":"workspacer-mcp-facade","implementation":"rust","hubConnected":upstream.as_ref().map(|upstream|upstream.connected()).unwrap_or_else(||matches!(*hub.status().borrow(),crate::Status::Ready {..})),"pluginCatalogReady":plugins.ready(),"launchReady":launch_ready,"listenAddr":address.to_string(),"hubBusUrl":hub_bus_url,"hubTransport":if upstream.is_some(){"websocket"}else{"embedded"},"hubUrl":hub_bus_url.as_deref().unwrap_or("in-process"),"migrationComplete":false})).into_response()
    }}));
    let router = router.merge(health).layer(middleware::from_fn_with_state(
        policy,
        crate::server::policy::guard,
    ));
    let result = tokio::select! {
        result = axum::serve(listener, router.into_make_service_with_connect_info::<crate::server::policy::Socket>()) => result.map_err(anyhow::Error::from),
        result = plugins.run(catalog_hub) => result.and_then(|_|Err(anyhow::anyhow!("plugin catalog observer stopped"))),
    };
    legacy.shutdown().await;
    result
}
