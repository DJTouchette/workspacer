//! Plugin HTTP compatibility adapter. Public manifests are allowlisted;
//! installs/reloads require the actual owner, other administration an operator.
use super::{SharedManager, manifest::Manifest};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Clone)]
struct HttpState {
    manager: SharedManager,
    policy: crate::server::policy::Policy,
    host_token: String,
    scoped_tokens: Option<std::path::PathBuf>,
    plugin_origin: String,
    examples_dir: Option<std::path::PathBuf>,
}
fn credential<'a>(headers: &'a HeaderMap, query: &'a BTreeMap<String, String>) -> Option<&'a str> {
    if let Some(header) = headers
        .get(header::AUTHORIZATION)
        .filter(|h| !h.as_bytes().is_empty())
    {
        return header
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")
            .filter(|s| !s.is_empty());
    }
    Some(query.get("token").map(String::as_str).unwrap_or(""))
}
fn host(s: &HttpState, h: &HeaderMap, q: &BTreeMap<String, String>) -> bool {
    credential(h, q).is_some_and(|token| {
        s.host_token.is_empty() || crate::auth::credential_eq(&s.host_token, &token)
    }) && s.policy.host(h, None)
        && s.policy.origin(h, None)
}
fn authorized(s: &HttpState, h: &HeaderMap, q: &BTreeMap<String, String>) -> bool {
    if !s.policy.host(h, None) || !s.policy.origin(h, None) {
        return false;
    }
    host(s, h, q)
        || credential(h, q)
            .and_then(|token| {
                s.scoped_tokens
                    .as_ref()
                    .and_then(|path| crate::auth::Store { path: path.clone() }.lookup(token))
            })
            .is_some_and(|record| record.scope() == Some(crate::auth::Scope::Operator))
}

fn denied() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error":"unauthorized"})),
    )
        .into_response()
}
fn host_denied(
    s: &HttpState,
    h: &HeaderMap,
    q: &BTreeMap<String, String>,
    action: &str,
) -> Response {
    if !authorized(s, h, q) {
        return denied();
    }
    (StatusCode::FORBIDDEN, Json(json!({"error":format!("{action} requires host authority: it runs code on the hub's own machine, so it is refused to every scoped bus token, the operator tier included. Run it from the machine that owns the hub.")}))).into_response()
}
fn enabled_value(body: &Value) -> anyhow::Result<bool> {
    match body.get("enabled") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(enabled)) => Ok(*enabled),
        _ => anyhow::bail!("enabled must be a boolean"),
    }
}
fn answer(result: anyhow::Result<Value>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":e.to_string()})),
        )
            .into_response(),
    }
}
fn public(m: Manifest) -> Value {
    let mut out = json!({"id":m.id,"name":m.name,"apiVersion":m.api_version,"disabled":m.disabled});
    if let Some(version) = m.contributions.get("version") {
        out["version"] = version.clone();
    }
    for (key, fields) in [
        ("panes", &["type", "title", "icon", "path", "scope"][..]),
        ("widgets", &["id", "title", "icon", "path", "sizes"][..]),
        ("hotkeys", &["id", "default", "command"][..]),
    ] {
        if let Some(items) = m.contributions.get(key).and_then(Value::as_array) {
            out[key] = Value::Array(
                items
                    .iter()
                    .map(|item| {
                        Value::Object(
                            fields
                                .iter()
                                .filter_map(|field| {
                                    item.get(*field).map(|v| ((*field).into(), v.clone()))
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            );
        }
    }
    out
}
async fn list(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !s.policy.host(&h, None) {
        return denied();
    }
    let manager = s.manager.lock().await;
    let manifests = manager.list();
    if authorized(&s, &h, &q) {
        answer(serde_json::to_value(manifests).map_err(Into::into))
    } else {
        Json(json!(manifests.into_iter().map(public).collect::<Vec<_>>())).into_response()
    }
}
async fn tokens(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    Json(json!(
        s.manager
            .lock()
            .await
            .plugins
            .iter()
            .filter(|(_, l)| !l.token.is_empty())
            .map(|(id, l)| (id.clone(), l.token.clone()))
            .collect::<BTreeMap<_, _>>()
    ))
    .into_response()
}
async fn settings_get(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    answer(
        s.manager
            .lock()
            .await
            .settings(q.get("pluginId").map(String::as_str).unwrap_or(""))
            .map(|values| json!({"values":values})),
    )
}
async fn mutate(
    State(s): State<HttpState>,
    Path(operation): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    if operation == "reload" && !host(&s, &h, &q) {
        return host_denied(&s, &h, &q, "plugin reload");
    }
    let mut manager = s.manager.lock().await;
    let id = body
        .get("pluginId")
        .or_else(|| body.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    answer(async {match operation.as_str(){
        "settings"=>Ok(json!({"values":manager.set_settings(id,body.get("values").and_then(Value::as_object).ok_or_else(||anyhow::anyhow!("values required"))?).await?})),
        "pane-token"=>Ok(json!({"token":manager.pane_token(id).await?})),
        "setEnabled"=>Ok(serde_json::to_value(manager.set_enabled(id,enabled_value(&body)?).await?)?),
        "reload"=>{let manifest=if let Some(dir)=body.get("dir").and_then(Value::as_str){Manifest::load(&std::path::Path::new(dir).join("plugin.json"))?}else{let m=manager.manifest(id)?;Manifest::load(&m.dir.join("plugin.json"))?};manager.add(manifest.clone()).await?;Ok(serde_json::to_value(manifest)?)},
        "remove"=>{if let Some(dir)=manager.remove(id).await?{super::install::uninstall_directory(&manager.root,&dir)?;}Ok(json!({"ok":true}))},
        _=>anyhow::bail!("unsupported plugin operation")
    }}.await)
}
async fn revoke(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    answer(
        async {
            s.manager
                .lock()
                .await
                .revoke_pane(
                    body.get("token")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("token required"))?,
                )
                .await?;
            Ok(json!({"ok":true}))
        }
        .await,
    )
}
async fn ui(
    State(s): State<HttpState>,
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !s.policy.host(&h, None) {
        return denied();
    }
    let path = match s.manager.lock().await.ui_file(&id, &path) {
        Ok(p) => p,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut own = false;
    if let Some(presented) = credential(&h, &query) {
        let hub = s.manager.lock().await.hub.clone();
        for token in [
            presented,
            query.get("busToken").map(String::as_str).unwrap_or(""),
        ] {
            if !token.is_empty()
                && hub
                    .plugin_token_matches(token.to_owned(), id.clone())
                    .await
                    .unwrap_or(false)
            {
                own = true;
                break;
            }
        }
    }
    let seed = if authorized(&s, &h, &query) || own {
        s.manager.lock().await.settings(&id).ok()
    } else {
        None
    };
    let mime = match path.extension().and_then(|x| x.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(mut bytes)) => {
            if mime.starts_with("text/html") {
                bytes = inject_sdk(&bytes, &id, seed.as_ref());
            }
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "base-uri 'none'; object-src 'none'",
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Default)]
pub struct HttpOptions {
    pub host_token: String,
    pub scoped_tokens: Option<std::path::PathBuf>,
    pub plugin_origin: String,
    pub examples_dir: Option<std::path::PathBuf>,
}
pub fn normalize_origin(raw: &str) -> anyhow::Result<String> {
    if raw.is_empty() {
        return Ok(String::new());
    }
    let url = url::Url::parse(raw)?;
    if !["http", "https"].contains(&url.scheme()) || url.host().is_none() {
        anyhow::bail!("plugin origin requires absolute HTTP(S) URL")
    };
    Ok(url.origin().ascii_serialization())
}
pub fn router(manager: SharedManager, host_token: String) -> Router {
    router_with_options(
        manager,
        HttpOptions {
            host_token,
            ..HttpOptions::default()
        },
    )
    .expect("default plugin HTTP options")
}
pub fn router_with_options(manager: SharedManager, options: HttpOptions) -> anyhow::Result<Router> {
    router_with_policy(manager, options, crate::server::policy::Policy::local())
}
pub(crate) fn router_with_policy(
    manager: SharedManager,
    options: HttpOptions,
    policy: crate::server::policy::Policy,
) -> anyhow::Result<Router> {
    let plugin_origin = normalize_origin(&options.plugin_origin)?;
    Ok(Router::new()
        .route("/plugins", get(list))
        .route("/plugins/tokens", get(tokens))
        .route("/plugins/settings", get(settings_get).post(settings_post))
        .route("/plugins/inspect", post(inspect))
        .route("/plugins/install", post(install))
        .route("/plugins/updates", get(updates))
        .route("/plugins/examples", get(examples))
        .route("/plugins/examples/install", post(example_install))
        .route("/plugins/:operation", post(mutate))
        .route("/plugins/pane-token/revoke", post(revoke))
        .route("/plugins/ui/:id/*path", get(ui))
        .route("/plugins/ui/:id/", get(ui_index))
        .route(
            "/plugins/sdk.js",
            get(|| async {
                (
                    [
                        (
                            header::CONTENT_TYPE,
                            "application/javascript; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "public, max-age=300"),
                    ],
                    include_str!("sdk.js"),
                )
            }),
        )
        .route(
            "/plugins/origin",
            get(
                |State(s): State<HttpState>| async move { Json(json!({"origin":s.plugin_origin})) },
            ),
        )
        .with_state(HttpState {
            manager,
            policy,
            host_token: options.host_token,
            scoped_tokens: options.scoped_tokens,
            plugin_origin,
            examples_dir: options.examples_dir,
        }))
}

async fn settings_post(
    state: State<HttpState>,
    query: Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    body: Json<Value>,
) -> Response {
    mutate(state, Path("settings".into()), query, headers, body).await
}
async fn inspect(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    let input = body
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    answer(
        async {
            Ok(serde_json::to_value(
                tokio::task::spawn_blocking(move || super::install::inspect(&input)).await??,
            )?)
        }
        .await,
    )
}
async fn updates(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !authorized(&s, &h, &q) {
        return denied();
    }
    let manifests = s.manager.lock().await.list();
    answer(
        async {
            Ok(
                tokio::task::spawn_blocking(move || super::install::check_updates(manifests))
                    .await?,
            )
        }
        .await,
    )
}
async fn install(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !host(&s, &h, &q) {
        return host_denied(&s, &h, &q, "plugin install");
    }
    let input = body
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let (root, runtime) = {
        let manager = s.manager.lock().await;
        (manager.root.clone(), manager.sidecar_node.clone())
    };
    let consent = match serde_json::from_value::<super::install::Consent>(body) {
        Ok(v) => v,
        Err(e) => return answer(Err(e.into())),
    };
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let publisher = s.manager.lock().await.hub.clone();
    let event_url = input.clone();
    let progress = move |stage: &str| {
        let _ = publisher.publish(crate::protocol::Event::new(
            "plugin.install.progress",
            "hub",
            json!({"url":event_url,"stage":stage}),
        ));
    };
    let prepared = match tokio::task::spawn_blocking(move || {
        super::install::prepare_cancellable_with_runtime(
            &root,
            &input,
            &consent,
            &progress,
            &cancellation,
            runtime.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            if let Some(need) = e.downcast_ref::<super::install::ConsentRequired>() {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({"needsConsent":true,"pluginId":need.plugin_id,"argv":need.argv})),
                )
                    .into_response();
            }
            return answer(Err(e));
        }
        Err(e) => return answer(Err(e.into())),
    };
    let mut manager = s.manager.lock().await;
    let old = manager.manifest(&prepared.manifest.id).ok();
    answer(
        async {
            manager.remove(&prepared.manifest.id).await?;
            let root = manager.root.clone();
            let committed = tokio::task::spawn_blocking(move || prepared.commit(&root)).await?;
            match committed {
                Ok(manifest) => {
                    manager.add(manifest.clone()).await?;
                    Ok(serde_json::to_value(manifest)?)
                }
                Err(e) => {
                    if let Some(old) = old {
                        manager.add(old).await?;
                    }
                    Err(e)
                }
            }
        }
        .await,
    )
}

async fn ui_index(
    state: State<HttpState>,
    Path(id): Path<String>,
    query: Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    ui(state, Path((id, String::new())), query, headers).await
}
fn inject_sdk(bytes: &[u8], id: &str, settings: Option<&Value>) -> Vec<u8> {
    // JSON serializers do not necessarily HTML-escape closing script tags.
    let escape = |value: Value| {
        value
            .to_string()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    };
    let mut script = format!("<script>window.__WKS_PLUGIN_ID__={};", escape(json!(id)));
    if let Some(settings) = settings {
        script.push_str(&format!(
            "window.__WKS_SETTINGS__={};",
            escape(settings.clone())
        ));
    }
    script.push_str("</script>\n<script src=\"/plugins/sdk.js\"></script>\n");
    let html = String::from_utf8_lossy(bytes);
    let index = html.to_ascii_lowercase().find("</head>").unwrap_or(0);
    let mut result = html[..index].to_owned();
    result.push_str(&script);
    result.push_str(&html[index..]);
    result.into_bytes()
}

fn example_manifests(directory: Option<&std::path::Path>) -> Vec<Manifest> {
    let Some(directory) = directory else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Manifest::load(&e.path().join("plugin.json")).ok())
        .collect()
}
async fn examples(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
) -> Response {
    if !s.policy.host(&h, None) {
        return denied();
    }
    let manifests = example_manifests(s.examples_dir.as_deref());
    if authorized(&s, &h, &q) {
        answer(serde_json::to_value(manifests).map_err(Into::into))
    } else {
        Json(json!(manifests.into_iter().map(public).collect::<Vec<_>>())).into_response()
    }
}
async fn example_install(
    State(s): State<HttpState>,
    Query(q): Query<BTreeMap<String, String>>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !host(&s, &h, &q) {
        return host_denied(&s, &h, &q, "installing a bundled example plugin");
    }
    let id = body.get("id").and_then(Value::as_str).unwrap_or("");
    let Some(manifest) = example_manifests(s.examples_dir.as_deref())
        .into_iter()
        .find(|m| m.id == id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (root, runtime) = {
        let manager = s.manager.lock().await;
        (manager.root.clone(), manager.sidecar_node.clone())
    };
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let prepared = match tokio::task::spawn_blocking(move || {
        super::install::prepare_directory_cancellable_with_runtime(
            &root,
            &manifest.dir,
            &super::install::Consent {
                allow_install_command: true,
                consented_argv: manifest.install,
            },
            &cancellation,
            runtime.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return answer(Err(e)),
        Err(e) => return answer(Err(e.into())),
    };
    let mut manager = s.manager.lock().await;
    answer(
        async {
            let old = manager.manifest(&prepared.manifest.id).ok();
            manager.remove(&prepared.manifest.id).await?;
            match prepared.commit(&manager.root) {
                Ok(m) => {
                    manager.add(m.clone()).await?;
                    Ok(serde_json::to_value(m)?)
                }
                Err(e) => {
                    if let Some(old) = old {
                        manager.add(old).await?;
                    }
                    Err(e)
                }
            }
        }
        .await,
    )
}

struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
