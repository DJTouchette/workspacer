//! Optional read-only companion link for an externally owned daemon.
//! This does not register desktop methods or acquire daemon process ownership.
use crate::{Handle, protocol::Event};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

const LIMIT: usize = 1 << 20;
pub struct ExternalDaemon {
    base: url::Url,
    http: reqwest::Client,
    closed: watch::Sender<bool>,
}
impl ExternalDaemon {
    pub fn new(url: &str) -> Result<Arc<Self>> {
        let mut base = url::Url::parse(url).context("invalid external claudemon URL")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            bail!(
                "external claudemon requires an HTTP(S) URL without credentials, query or fragment"
            );
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .build()?;
        let (closed, _) = watch::channel(false);
        Ok(Arc::new(Self { base, http, closed }))
    }
    pub fn close(&self) {
        self.closed.send_replace(true);
    }
    async fn json(&self, path: &str) -> Result<Value> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            bail!("external claudemon link is closed");
        }
        let read = async {
            let mut response = self
                .http
                .get(self.base.join(path)?)
                .send()
                .await?
                .error_for_status()?;
            if !response.status().is_success() {
                bail!("external claudemon HTTP {}", response.status());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len().saturating_add(chunk.len()) > 16 * LIMIT {
                    bail!("external claudemon response exceeds 16 MiB");
                }
                body.extend_from_slice(&chunk);
            }
            Ok(serde_json::from_slice(&body)?)
        };
        tokio::select! {biased;_=closed.changed()=>bail!("external claudemon link is closed"), result=tokio::time::timeout(Duration::from_secs(10),read)=>result.context("external claudemon read deadline exceeded")?}
    }
    pub async fn usage_report(&self) -> Result<Value> {
        self.json("usage/report").await
    }
    pub async fn models(&self, provider: &str) -> Result<Value> {
        if !["codex", "copilot", "opencode", "pi"].contains(&provider) {
            bail!("unsupported external catalog provider");
        }
        self.json(&format!("providers/{provider}/models")).await
    }
    pub async fn run(self: Arc<Self>, hub: Handle) -> Result<()> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Ok(());
        }
        tokio::select! {biased;_=closed.changed()=>return Ok(()),r=hub.ready()=>{r?;}}
        let mut backoff = Duration::from_millis(200);
        loop {
            let started = Instant::now();
            let result =
                tokio::select! {biased;_=closed.changed()=>return Ok(()),r=self.stream(&hub)=>r};
            if let Err(error) = result {
                eprintln!("external claudemon event stream ended: {error}");
            }
            if started.elapsed() >= Duration::from_secs(5) {
                backoff = Duration::from_millis(200);
            }
            tokio::select! {biased;_=closed.changed()=>return Ok(()),_=tokio::time::sleep(backoff)=>()}
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }
    async fn stream(&self, hub: &Handle) -> Result<()> {
        let mut response = self
            .http
            .get(self.base.join("events")?)
            .header("accept", "text/event-stream")
            .send()
            .await?
            .error_for_status()?;
        if !response.status().is_success() {
            bail!("external claudemon HTTP {}", response.status());
        }
        let mut parser = Sse::default();
        while let Some(chunk) = response.chunk().await? {
            for event in parser.push(&chunk)? {
                hub.publish_wait(event).await?;
            }
        }
        for event in parser.finish()? {
            hub.publish_wait(event).await?;
        }
        Ok(())
    }
}
#[derive(Default)]
struct Sse {
    line: Vec<u8>,
    name: String,
    data: Vec<u8>,
}
impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                self.line(&mut events)?;
            } else {
                if self.line.len() >= LIMIT {
                    bail!("external event line exceeds 1 MiB");
                }
                self.line.push(*byte);
            }
        }
        Ok(events)
    }
    fn line(&mut self, events: &mut Vec<Event>) -> Result<()> {
        let mut bytes = std::mem::take(&mut self.line);
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        if bytes.is_empty() {
            self.flush(events);
        } else if let Some(name) = bytes.strip_prefix(b"event:") {
            self.name = String::from_utf8_lossy(name).trim().to_string();
        } else if let Some(data) = bytes.strip_prefix(b"data:") {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            if self.data.len().saturating_add(data.len()).saturating_add(1) > LIMIT {
                bail!("external event frame exceeds 1 MiB");
            }
            if !self.data.is_empty() {
                self.data.push(b'\n');
            }
            self.data.extend_from_slice(data);
        }
        Ok(())
    }
    fn flush(&mut self, events: &mut Vec<Event>) {
        if let Some(event) = map_event(&self.name, &self.data) {
            events.push(event);
        }
        self.name.clear();
        self.data.clear();
    }
    fn finish(&mut self) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        if !self.line.is_empty() {
            self.line(&mut events)?;
        }
        self.flush(&mut events);
        Ok(events)
    }
}
#[derive(Deserialize, Default)]
struct State {
    mode: Option<String>,
    cwd: Option<String>,
}
#[derive(Deserialize)]
struct Update {
    session_id: Option<String>,
    event: Option<String>,
    state: Option<Object<State>>,
}
// Serde-derived structs also accept positional JSON arrays; the daemon event
// contract (and the Go decoder) requires JSON objects at both levels.
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct MapOnly<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for MapOnly<T> {
            type Value = Object<T>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        deserializer.deserialize_map(MapOnly(std::marker::PhantomData))
    }
}
fn map_event(name: &str, data: &[u8]) -> Option<Event> {
    if !matches!(name, "" | "session.update") {
        return None;
    }
    let Object(update): Object<Update> = serde_json::from_slice(data).ok()?;
    let session_id = update.session_id.unwrap_or_default();
    if session_id.is_empty() {
        return None;
    }
    let state = update.state.map(|state| state.0).unwrap_or_default();
    let cwd = state.cwd.unwrap_or_default();
    let mut data = json!({"sessionId":session_id,"hookEvent":update.event.unwrap_or_default(),"mode":state.mode.unwrap_or_default()});
    if !cwd.is_empty() {
        data["cwd"] = Value::String(cwd);
    }
    Some(Event::new("agent.state_changed", "claudemon", data))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_streams_back_off_and_cancellation_does_not_stop_the_daemon() {
        use axum::{Router, http::StatusCode, routing::get};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/events",
            get(move || {
                count.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "event: session.update\ndata: {\"session_id\":\"must-not-publish\"}\n\n",
                    )
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut options = crate::Options::default();
        options.control_plane_only = true;
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
        let mut events = client.events();
        client
            .topics(["agent.state_changed".into()].into())
            .await
            .unwrap();
        let daemon = ExternalDaemon::new(&url).unwrap();
        let error = daemon.stream(&hub.handle()).await.unwrap_err();
        assert!(error.to_string().contains("503"));
        attempts.store(0, Ordering::SeqCst);
        let running = tokio::spawn(daemon.clone().run(hub.handle()));
        tokio::time::sleep(Duration::from_millis(1500)).await;
        daemon.close();
        tokio::time::timeout(Duration::from_secs(3), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let count = attempts.load(Ordering::SeqCst);
        assert!(
            (2..=12).contains(&count),
            "retry loop spun or stopped retrying: {count}"
        );
        assert!(
            events.try_recv().is_err(),
            "error response was treated as an SSE stream"
        );
        assert_eq!(
            reqwest::get(format!("{url}/events"))
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn cancellation_releases_a_live_body_that_never_sends_another_frame() {
        use axum::{
            Router,
            body::{Body, Bytes},
            response::Response,
            routing::get,
        };
        use futures_util::StreamExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/events",
            get(|| async {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(
                        futures_util::stream::once(async {
                            Ok::<_, std::io::Error>(Bytes::from_static(
                                b"data: {\"session_id\":\"live-body\"}\n\n",
                            ))
                        })
                        .chain(futures_util::stream::pending::<Result<Bytes, std::io::Error>>()),
                    ))
                    .unwrap()
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut options = crate::Options::default();
        options.control_plane_only = true;
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
        let mut events = client.events();
        client
            .topics(["agent.state_changed".into()].into())
            .await
            .unwrap();
        let daemon = ExternalDaemon::new(&url).unwrap();
        let running = tokio::spawn(daemon.clone().run(hub.handle()));
        let first = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.data.unwrap()["sessionId"], "live-body");
        daemon.close();
        tokio::time::timeout(Duration::from_secs(3), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn companion_reads_and_sse_shutdown_never_own_the_external_server() {
        use axum::{
            Router,
            http::{StatusCode, header},
            response::IntoResponse,
            routing::get,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app=Router::new()
            .route("/usage/report",get(||async{axum::Json(json!({"providers":[],"measuredFixture":true}))}))
            .route("/providers/codex/models",get(||async{axum::Json(json!({"models":[]}))}))
            .route("/providers/pi/models",get(||async{(StatusCode::TEMPORARY_REDIRECT,[(header::LOCATION,"/usage/report")],axum::Json(json!({"models":[]}))).into_response()}))
            .route("/events",get(||async{([(header::CONTENT_TYPE,"text/event-stream")],"event: session.update\ndata: {\"session_id\":\"fixture\",\"event\":\"Stop\",\"state\":{\"mode\":\"input\"}}\n\n")}));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let daemon = ExternalDaemon::new(&url).unwrap();
        assert_eq!(
            daemon.usage_report().await.unwrap()["measuredFixture"],
            true
        );
        assert_eq!(daemon.models("codex").await.unwrap()["models"], json!([]));
        assert!(daemon.models("pi").await.is_err());
        assert!(daemon.models("../usage").await.is_err());
        let directory = tempfile::tempdir().unwrap();
        let routing =
            Arc::new(super::super::routing::RoutingService::open(directory.path().into()).unwrap());
        let mut options = crate::Options::default().handler("claude.listModels", |_, _| async {
            Ok(json!({"aliases":[{"value":"sonnet"}],"seen":["claude-fixture"]}))
        });
        options.control_plane_only = true;
        options.external_claudemon_url = Some(url.clone());
        options.routing = Some(routing.clone());
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
        let mut events = client.events();
        client
            .topics(["agent.state_changed".into()].into())
            .await
            .unwrap();
        client.call("usage.report", json!({})).await.unwrap();
        client
            .call("routing.select", json!({"role":"scout"}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let catalog = routing.catalog();
                if catalog["codex"]["state"] == "unavailable"
                    && catalog["claude"]["state"] == "available"
                {
                    assert!(
                        catalog["claude"]["models"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|m| m["id"] == "claude-fixture")
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.source, "claudemon");
        assert_eq!(event.data.unwrap()["sessionId"], "fixture");
        daemon.close();
        assert!(daemon.usage_report().await.is_err());
        assert!(
            reqwest::get(format!("{url}/usage/report"))
                .await
                .unwrap()
                .status()
                .is_success()
        );
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(
            reqwest::get(format!("{url}/usage/report"))
                .await
                .unwrap()
                .status()
                .is_success()
        );
        stop.send(()).unwrap();
        server.await.unwrap();
    }
    #[test]
    fn companion_url_has_no_credential_or_redirect_authority() {
        for url in [
            "file:///tmp/daemon",
            "http://user:pass@localhost/",
            "http://localhost/?token=secret",
            "http://localhost/#token",
        ] {
            assert!(ExternalDaemon::new(url).is_err(), "{url}");
        }
        assert_eq!(
            ExternalDaemon::new("http://127.0.0.1:7891")
                .unwrap()
                .base
                .as_str(),
            "http://127.0.0.1:7891/"
        );
    }
    #[test]
    fn fragmented_sse_crlf_multiline_unknown_and_eof_match_bridge() {
        let wire=b": keepalive\r\nevent: session.update\r\ndata: {\r\ndata: \"session_id\":\"s\",\"event\":\"Stop\",\"state\":{\"mode\":\"input\",\"cwd\":\"/repo\"}}\r\n\r\nevent: session.resync\ndata: {}\n\ndata: {\"session_id\":\"t\"}";
        let mut parser = Sse::default();
        let mut events = Vec::new();
        for bytes in wire.chunks(3) {
            events.extend(parser.push(bytes).unwrap());
        }
        events.extend(parser.finish().unwrap());
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].topic, "agent.state_changed");
        assert_eq!(events[0].source, "claudemon");
        assert_eq!(
            events[0].data,
            Some(json!({"sessionId":"s","hookEvent":"Stop","mode":"input","cwd":"/repo"}))
        );
        assert_eq!(
            events[1].data,
            Some(json!({"sessionId":"t","hookEvent":"","mode":""}))
        );
        assert!(map_event("session.update", b"{\"session_id\":false}").is_none());
        assert!(map_event("session.update", b"not json").is_none());
        assert!(map_event("session.update", b"{\"event\":\"Stop\"}").is_none());
        assert!(map_event("other", b"{\"session_id\":\"s\"}").is_none());
        assert!(map_event("", br#"["s","Stop",{"mode":"input"}]"#).is_none());
        assert!(map_event("", br#"{"session_id":"s","state":["input","/repo"]}"#).is_none());
        assert_eq!(
            map_event("", b"{\"session_id\":\"s\",\"event\":null,\"state\":null}")
                .unwrap()
                .data,
            Some(json!({"sessionId":"s","hookEvent":"","mode":""}))
        );
        assert!(Sse::default().push(&vec![b'x'; LIMIT + 1]).is_err());
    }
}
