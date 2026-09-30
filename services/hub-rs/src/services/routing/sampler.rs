use super::*;
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
type Fetch = Shared<BoxFuture<'static, std::result::Result<Value, String>>>;
const CALLER_WAIT: Duration = Duration::from_secs(3);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
struct Flight {
    generation: u64,
    result: Fetch,
    cancel: tokio::task::AbortHandle,
}

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod flight_tests;
#[cfg(test)]
mod preview_tests;
#[cfg(test)]
mod report_tests;
fn claude_models(catalog: &Value) -> Option<Vec<Value>> {
    #[derive(serde::Deserialize)]
    struct Alias {
        value: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Catalog {
        aliases: Option<Vec<Option<Alias>>>,
        seen: Option<Vec<Option<String>>>,
    }
    let catalog: Catalog = serde_json::from_value(catalog.clone()).ok()?;
    let ids = catalog
        .aliases
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|a| a.value)
        .chain(catalog.seen.into_iter().flatten().flatten());
    let models: Vec<_> = ids
        .filter(|id| !id.is_empty())
        .map(|id| json!({"id":id}))
        .collect();
    (!models.is_empty()).then_some(models)
}

fn provider_models(catalog: &Value) -> Option<Vec<Value>> {
    #[derive(serde::Deserialize)]
    struct Model {
        id: Option<String>,
        #[serde(rename = "effortLevels")]
        effort_levels: Option<Vec<Option<String>>>,
    }
    // Missing/malformed envelopes are unknown, not a successful zero-model
    // answer. A present null/empty models list retains the Go empty-list rule.
    let rows: Option<Vec<Option<Model>>> =
        serde_json::from_value(catalog.get("models")?.clone()).ok()?;
    Some(
        rows.into_iter()
            .flatten()
            .flatten()
            .filter_map(|row| {
                let id = row.id.filter(|id| !id.is_empty())?;
                let mut model = json!({"id":id});
                let levels: Vec<_> = row
                    .effort_levels
                    .into_iter()
                    .flatten()
                    .map(|level| level.unwrap_or_default())
                    .collect();
                if !levels.is_empty() {
                    model["effortLevels"] = json!(levels);
                }
                Some(model)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[tokio::test]
    async fn external_usage_joins_inflight_but_decisions_do_not_reuse_completed_cache() {
        let reads = Arc::new(AtomicUsize::new(0));
        let count = reads.clone();
        let app = axum::Router::new().route(
            "/usage/report",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    let sample = count.fetch_add(1, Ordering::SeqCst) + 1;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    axum::Json(json!({"providers":[],"fixtureSample":sample}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let external = super::super::super::external_claudemon::ExternalDaemon::new(&format!(
            "http://{}",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let mut sampler = UsageSampler::new(None);
        sampler.external = Some(external.clone());
        let (a, b) = tokio::join!(
            sampler.report(Duration::ZERO),
            sampler.report(Duration::ZERO)
        );
        assert_eq!(a.unwrap()["fixtureSample"], 1);
        assert_eq!(b.unwrap()["fixtureSample"], 1);
        assert_eq!(
            sampler.report(Duration::from_secs(60)).await.unwrap()["fixtureSample"],
            1
        );
        assert_eq!(
            sampler.report(Duration::ZERO).await.unwrap()["fixtureSample"],
            2
        );
        external.close();
        assert!(sampler.report(Duration::ZERO).await.is_err());
        assert!(sampler.cache.lock().unwrap().report.is_none());
        assert!(
            sampler.report(Duration::from_secs(60)).await.is_err(),
            "a known failed refresh must not turn an old observation back into success"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        stop.send(()).unwrap();
        server.await.unwrap();
    }
    #[test]
    fn provider_catalog_requires_typed_launchable_ids_without_exposing_extra_fields() {
        assert_eq!(
            provider_models(&json!({"models":[null,{}, {"id":null}, {"id":""}]})),
            Some(vec![])
        );
        assert_eq!(provider_models(&json!({"models":null})), Some(vec![]));
        assert_eq!(
            provider_models(
                &json!({"models":[{"id":"model","effortLevels":[null,"high"],"private":"do-not-project"}]})
            ),
            Some(vec![json!({"id":"model","effortLevels":["","high"]})])
        );
        for invalid in [
            Value::Null,
            json!({}),
            json!({"models":{}}),
            json!({"models":[{"id":42}]}),
            json!({"models":[{"id":"valid"},{"id":"bad","effortLevels":[42]}]}),
        ] {
            assert!(provider_models(&invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn actual_claude_aliases_and_seen_form_catalog_but_empty_is_unknown() {
        assert_eq!(
            claude_models(
                &json!({"aliases":[{"value":"sonnet"},{"value":""}],"seen":["claude-fixture"]})
            ),
            Some(vec![json!({"id":"sonnet"}), json!({"id":"claude-fixture"})])
        );
        assert!(claude_models(&json!({"aliases":[],"seen":[]})).is_none());
        assert!(claude_models(&Value::Null).is_none());
        assert!(claude_models(&json!({"aliases":[{"value":"sonnet"}],"seen":[42]})).is_none());
    }
}
#[derive(Default)]
struct Cache {
    report: Option<(Instant, Value)>,
    flight: Option<Flight>,
    generation: u64,
}
/// Host-owned report transport. Concurrent callers join the same fetch; decisions
/// never consume a completed cache entry. Overview may reuse one for one minute.
pub struct UsageSampler {
    engine: Option<EmbeddedClient>,
    external: Option<Arc<super::super::external_claudemon::ExternalDaemon>>,
    hub: Option<crate::Handle>,
    cache: Arc<Mutex<Cache>>,
    catalog_flight: AtomicBool,
    catalog_last: Mutex<Option<Instant>>,
}
impl Drop for UsageSampler {
    fn drop(&mut self) {
        // Cancel the outstanding read when its sampler owner drops. Dropping
        // one waiter does not reach this while the service still owns it.
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(flight) = cache.flight.take() {
                flight.cancel.abort();
            }
        }
    }
}

impl UsageSampler {
    pub fn new(engine: Option<EmbeddedClient>) -> Self {
        Self {
            engine,
            external: None,
            hub: None,
            cache: Arc::new(Mutex::new(Cache::default())),
            catalog_flight: AtomicBool::new(false),
            catalog_last: Mutex::new(None),
        }
    }
    fn refresh_catalog(self: &Arc<Self>, service: Arc<RoutingService>) {
        let mut last = self.catalog_last.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
            return;
        }
        if self.catalog_flight.swap(true, Ordering::AcqRel) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        let sampler = self.clone();
        tokio::spawn(async move {
            struct Reset(Arc<UsageSampler>);
            impl Drop for Reset {
                fn drop(&mut self) {
                    self.0.catalog_flight.store(false, Ordering::Release);
                }
            }
            let _reset = Reset(sampler.clone());
            let matrix = service.matrix();
            let mut providers = std::collections::BTreeSet::new();
            for rows in matrix["profiles"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                for row in rows.as_object().into_iter().flat_map(|m| m.values()) {
                    for a in candidates(row) {
                        providers.insert(word(&a["provider"]).to_string());
                    }
                }
            }
            let cached = service.catalog();
            let requests = providers
                .into_iter()
                .filter(|p| {
                    cached[p]["state"] != "available"
                        || cached[p]["observedAt"]
                            .as_i64()
                            .is_none_or(|at| chrono::Utc::now().timestamp_millis() - at >= 600_000)
                })
                .map(|p| {
                    let engine = sampler.engine.clone();
                    let external = sampler.external.clone();
                    let hub = sampler.hub.clone();
                    let service = service.clone();
                    async move {
                        if p == "claude" {
                            // Ask the actual model owner after startup. Electron may own
                            // this method in hub-only mode; an empty response is unknown.
                            let result = async {
                                let hub = hub.ok_or_else(|| {
                                    anyhow::anyhow!("model catalog bus unavailable")
                                })?;
                                let client = crate::client::Client::connect_service(&hub).await?;
                                let result = client
                                    .call_with_timeout(
                                        "claude.listModels",
                                        json!({}),
                                        Duration::from_secs(20),
                                    )
                                    .await;
                                client.close();
                                result
                            }
                            .await;
                            service
                                .update_catalog(&p, result.ok().as_ref().and_then(claude_models));
                            return;
                        }
                        if !["codex", "copilot", "opencode", "pi"].contains(&p.as_str()) {
                            return;
                        }
                        let result = tokio::time::timeout(Duration::from_secs(20), async {
                            if let Some(engine) = engine {
                                engine
                                    .request(Command::Request {
                                        method: "GET".into(),
                                        path: format!("/providers/{p}/models"),
                                        payload: None,
                                    })
                                    .await
                            } else if let Some(external) = external {
                                external.models(&p).await
                            } else {
                                bail!("model catalog has no daemon")
                            }
                        })
                        .await;
                        let models = result
                            .ok()
                            .and_then(Result::ok)
                            .and_then(|value| provider_models(&value));
                        service.update_catalog(&p, models);
                    }
                });
            futures_util::future::join_all(requests).await;
        });
    }
    pub async fn report(&self, max_age: Duration) -> Result<Value> {
        let future = {
            let mut cache = self.cache.lock().unwrap();
            if !max_age.is_zero() {
                if let Some((at, report)) = &cache.report {
                    if at.elapsed() < max_age {
                        return Ok(report.clone());
                    }
                }
            }
            if let Some(flight) = &cache.flight {
                flight.result.clone()
            } else {
                let engine = self.engine.clone();
                let external = self.external.clone();
                let saved = self.cache.clone();
                cache.generation += 1;
                let generation = cache.generation;
                let (send, receive) =
                    tokio::sync::oneshot::channel::<std::result::Result<Value, String>>();
                let future = async move {
                    receive.await.unwrap_or_else(|_| {
                        Err("usage sampler stopped before fetch completed".into())
                    })
                }
                .boxed()
                .shared();
                // The sampler owns the read independently of any caller. A
                // cancelled/expired waiter must not discard a late observation.
                let task = tokio::spawn(async move {
                    let fetched = tokio::time::timeout(FETCH_TIMEOUT, async {
                        let report = if let Some(engine) = engine {
                            engine
                                .request(Command::Request {
                                    method: "GET".into(),
                                    path: "/usage/report".into(),
                                    payload: None,
                                })
                                .await?
                        } else if let Some(external) = external {
                            external.usage_report().await?
                        } else {
                            bail!("usage sampler has no daemon");
                        };
                        if !report["providers"].is_array() {
                            bail!("invalid usage report: providers absent");
                        }
                        Ok::<_, anyhow::Error>(report)
                    })
                    .await;
                    let result = match fetched {
                        Ok(result) => result.map_err(|error| error.to_string()),
                        Err(_) => Err("usage sampler fetch deadline exceeded".into()),
                    };
                    {
                        let mut cache = saved.lock().unwrap();
                        if cache
                            .flight
                            .as_ref()
                            .is_some_and(|flight| flight.generation == generation)
                        {
                            cache.flight = None;
                            cache.report = result
                                .as_ref()
                                .ok()
                                .map(|report| (Instant::now(), report.clone()));
                        }
                    }
                    // Publish cache first: even with no remaining waiter, the
                    // next Overview request can use this bounded observation.
                    let _ = send.send(result);
                });
                cache.flight = Some(Flight {
                    generation,
                    result: future.clone(),
                    cancel: task.abort_handle(),
                });
                future
            }
        };
        tokio::time::timeout(CALLER_WAIT, future).await
            .map_err(|_| anyhow::anyhow!("usage report did not arrive within the 3-second caller budget; shared fetch remains open for a later reader"))?
            .map_err(anyhow::Error::msg)
    }
}
pub fn install(
    mut options: crate::Options,
    routing: Arc<RoutingService>,
    engine: Option<EmbeddedClient>,
    hub: crate::Handle,
) -> crate::Options {
    let mut sampler = UsageSampler::new(engine);
    sampler.external = options.external_claudemon.clone();
    sampler.hub = Some(hub.clone());
    let sampler = Arc::new(sampler);
    for method in [
        "routing.select",
        "routing.preview",
        "usage.report",
        "routing.preferences.get",
        "routing.preferences.validate",
        "routing.preferences.save",
        "routing.preferences.reset",
    ] {
        let service = routing.clone();
        let sampler = sampler.clone();
        let publisher = hub.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            let sampler = sampler.clone();
            let publisher = publisher.clone();
            async move {
                if method.starts_with("routing.preferences.") {
                    return tokio::task::spawn_blocking(move || {
                        service.preferences(&caller, method, params)
                    })
                    .await?;
                }
                if method == "usage.report" {
                    if !params.is_null() && params != json!({}) {
                        bail!("usage.report: no parameters accepted");
                    }
                    let report = sampler
                        .report(Duration::from_secs(60))
                        .await
                        .context("usage.report unavailable")?;
                    return Ok(service.usage_report(&report, chrono::Utc::now().timestamp()));
                }
                if method == "routing.preview" {
                    super::preview::validate_request(&params)?;
                }
                if method == "routing.select" {
                    sampler.refresh_catalog(service.clone());
                }
                let report = sampler
                    .report(Duration::ZERO)
                    .await;
                let (report, warning) = match report {
                    Ok(v) => (v, None),
                    Err(e) => (
                        json!({"providers":[]}),
                        Some(format!("usage unavailable: {e}; capacity remains unknown")),
                    ),
                };
                let instant = chrono::Utc::now();
                let now = instant.timestamp();
                let usage_known = warning.is_none();
                let mut decision = if method == "routing.preview" {
                    select(&service.matrix(), &params, &report, now)
                        .map_err(|_| anyhow::anyhow!("routing preview unavailable; inspect host routing policy"))?
                } else {
                    service.select(params, &report, now)?
                };
                if let Some(warning) = warning {
                    decision["reason"]
                        .as_array_mut()
                        .unwrap()
                        .push(warning.into());
                }
                if method == "routing.preview" {
                    return Ok(super::preview::project(&decision, instant.timestamp_millis(), usage_known));
                }
                if method == "routing.select" {
                    if service.catalog_pending() {
                        decision["reason"].as_array_mut().unwrap().push(json!("the routing service still owes routing.yaml a catalog check (either none has run yet, or a provider named in it could not answer the last one)"));
                    }
                    service.log_decision(&decision);
                    // Failure to deliver during shutdown cannot invalidate an
                    // already recorded decision, matching the reference sink.
                    let _ = publisher
                        .publish_wait(crate::protocol::Event::new(
                            "routing.decision",
                            "routing",
                            super::events::projection(&decision),
                        ))
                        .await;
                }
                Ok(decision)
            }
        });
    }
    options
}
