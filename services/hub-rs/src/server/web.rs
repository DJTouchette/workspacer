//! Browser assets contain no credentials. Only the remote/full-app entry pages
//! require operator authority; mobile startup and static library code are public.
use axum::{
    Router,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
#[derive(Clone)]
struct Web {
    auth: super::Credentials,
    root: Option<PathBuf>,
    reads: Arc<tokio::sync::Semaphore>,
}
fn asset(body: &'static [u8], mime: &str, cache: &str) -> Response {
    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        body,
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
}
async fn remote(
    State(state): State<Web>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    if !state.auth.operator(
        &headers,
        query.get("token").map(String::as_str).unwrap_or(""),
    ) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    asset(
        include_bytes!("../../assets/web/remote.html"),
        "text/html; charset=utf-8",
        "no-cache",
    )
}
async fn app_redirect(RawQuery(query): RawQuery) -> Response {
    let location = format!(
        "/app/{}",
        query.map(|q| format!("?{q}")).unwrap_or_default()
    );
    (
        StatusCode::PERMANENT_REDIRECT,
        [(header::LOCATION, location)],
    )
        .into_response()
}
async fn app_index(
    State(state): State<Web>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    serve_app(state, "index.html".into(), headers, query).await
}
async fn app_path(
    State(state): State<Web>,
    Path(path): Path<String>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    serve_app(state, path, headers, query).await
}
fn mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
async fn serve_app(
    state: Web,
    path: String,
    headers: HeaderMap,
    query: BTreeMap<String, String>,
) -> Response {
    let Some(root) = state.root else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let relative = std::path::Path::new(&path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || path
            .split(['/', '\\'])
            .any(|part| part.starts_with('.') || part.is_empty())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let entry = path == "index.html";
    if entry
        && !state.auth.operator(
            &headers,
            query.get("token").map(String::as_str).unwrap_or(""),
        )
    {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let Ok(permit) = state.reads.try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let name = path.clone();
    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let _permit = permit;
        let target = crate::services::paths::canonicalize(&root.join(&name))?;
        anyhow::ensure!(target.starts_with(&root), "asset escaped root");
        anyhow::ensure!(
            entry || target != root.join("index.html"),
            "entry aliases require the guarded entry route"
        );
        let file = std::fs::File::open(target)?;
        anyhow::ensure!(file.metadata()?.is_file(), "asset is not a regular file");
        use std::io::Read;
        let mut body = vec![];
        file.take(32 * 1024 * 1024 + 1).read_to_end(&mut body)?;
        anyhow::ensure!(body.len() <= 32 * 1024 * 1024, "asset too large");
        Ok(body)
    })
    .await;
    match bytes {
        Ok(Ok(body)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime(&path)),
                (
                    header::CACHE_CONTROL,
                    if entry {
                        "no-cache"
                    } else {
                        "public, max-age=86400"
                    },
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            body,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
pub(crate) fn router(
    auth: super::Credentials,
    directory: Option<PathBuf>,
) -> anyhow::Result<Router> {
    let root = directory
        .and_then(|directory| crate::services::paths::canonicalize(&directory).ok())
        .filter(|directory| directory.join("index.html").is_file());
    let state = Web {
        auth,
        root,
        reads: Arc::new(tokio::sync::Semaphore::new(8)),
    };
    let mut router = Router::new()
        .route("/remote", get(remote))
        .route(
            "/m",
            get(|| async {
                asset(
                    include_bytes!("../../assets/web/mobile.html"),
                    "text/html; charset=utf-8",
                    "no-cache",
                )
            }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                asset(
                    include_bytes!("../../assets/web/manifest.webmanifest"),
                    "application/manifest+json; charset=utf-8",
                    "no-cache",
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async {
                let mut response = asset(
                    include_bytes!("../../assets/web/sw.js"),
                    "text/javascript; charset=utf-8",
                    "no-cache",
                );
                response
                    .headers_mut()
                    .insert("service-worker-allowed", "/".parse().unwrap());
                response
            }),
        );
    macro_rules! public {
        ($route:literal,$file:literal,$mime:literal) => {
            router = router.route(
                $route,
                get(|| async {
                    asset(
                        include_bytes!(concat!("../../assets/web/", $file)),
                        $mime,
                        "public, max-age=86400",
                    )
                }),
            );
        };
    }
    public!("/icon-192.png", "icon-192.png", "image/png");
    public!("/icon-512.png", "icon-512.png", "image/png");
    public!(
        "/icon-maskable-512.png",
        "icon-maskable-512.png",
        "image/png"
    );
    public!("/apple-touch-icon.png", "apple-touch-icon.png", "image/png");
    public!(
        "/xterm.js",
        "xterm.js",
        "application/javascript; charset=utf-8"
    );
    public!("/xterm.css", "xterm.css", "text/css; charset=utf-8");
    public!(
        "/addon-fit.js",
        "addon-fit.js",
        "application/javascript; charset=utf-8"
    );
    if state.root.is_some() {
        router = router
            .route("/app", get(app_redirect))
            .route("/app/", get(app_index))
            .route("/app/*path", get(app_path));
    }
    Ok(router.with_state(state))
}
