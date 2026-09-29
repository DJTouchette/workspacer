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
    flight: Option<(u64, Fetch)>,
    generation: u64,
}
/// Host-owned report transport. Concurrent callers join the same fetch; decisions
/// never consume a completed cache entry. Overview may reuse one for one minute.
pub struct UsageSampler {
    engine: Option<EmbeddedClient>,
    external: Option<Arc<super::super::external_claudemon::ExternalDaemon>>,
    hub: Option<crate::Handle>,
    cache: Mutex<Cache>,
    catalog_flight: AtomicBool,
    catalog_last: Mutex<Option<Instant>>,
}
impl UsageSampler {
    pub fn new(engine: Option<EmbeddedClient>) -> Self {
        Self {
            engine,
            external: None,
            hub: None,
            cache: Mutex::new(Cache::default()),
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
                            .and_then(|v| v["models"].as_array().cloned());
                        service.update_catalog(&p, models);
                    }
                });
            futures_util::future::join_all(requests).await;
        });
    }
    pub async fn report(&self, max_age: Duration) -> Result<Value> {
        let (generation, future) = {
            let mut cache = self.cache.lock().unwrap();
            if !max_age.is_zero() {
                if let Some((at, report)) = &cache.report {
                    if at.elapsed() < max_age {
                        return Ok(report.clone());
                    }
                }
            }
            if let Some(flight) = &cache.flight {
                flight.clone()
            } else {
                let engine = self.engine.clone();
                let external = self.external.clone();
                cache.generation += 1;
                let generation = cache.generation;
                let future = async move {
                    let result = tokio::time::timeout(Duration::from_secs(3), async {
                        if let Some(engine) = engine {
                            engine
                                .request(Command::Request {
                                    method: "GET".into(),
                                    path: "/usage/report".into(),
                                    payload: None,
                                })
                                .await
                        } else if let Some(external) = external {
                            external.usage_report().await
                        } else {
                            bail!("usage sampler has no daemon")
                        }
                    })
                    .await
                    .map_err(|_| "usage sampler deadline exceeded".to_string())?
                    .map_err(|e| e.to_string())?;
                    if !result["providers"].is_array() {
                        return Err("invalid usage report: providers absent".into());
                    }
                    Ok(result)
                }
                .boxed()
                .shared();
                cache.flight = Some((generation, future.clone()));
                (generation, future)
            }
        };
        let result = future.await;
        let mut cache = self.cache.lock().unwrap();
        if cache.flight.as_ref().is_some_and(|(g, _)| *g == generation) {
            cache.flight = None;
            if let Ok(report) = &result {
                cache.report = Some((Instant::now(), report.clone()));
            } else {
                // A cached observation is reusable only until this sampler
                // learns its upstream cannot supply a new one. Preserve each
                // waiter's own result, but never revive old quota after failure.
                cache.report = None;
            }
        }
        result.map_err(anyhow::Error::msg)
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
                if method == "routing.select" {
                    sampler.refresh_catalog(service.clone());
                }
                let report = sampler
                    .report(if method == "routing.preview" {
                        Duration::from_secs(60)
                    } else {
                        Duration::ZERO
                    })
                    .await;
                let (report, warning) = match report {
                    Ok(v) => (v, None),
                    Err(e) => (
                        json!({"providers":[]}),
                        Some(format!("usage unavailable: {e}; capacity remains unknown")),
                    ),
                };
                let now = chrono::Utc::now().timestamp();
                let mut decision = if method == "routing.preview" {
                    select(&service.matrix(), &params, &report, now)?
                } else {
                    service.select(params, &report, now)?
                };
                if let Some(warning) = warning {
                    decision["reason"]
                        .as_array_mut()
                        .unwrap()
                        .push(warning.into());
                }
                if method == "routing.select" {
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
