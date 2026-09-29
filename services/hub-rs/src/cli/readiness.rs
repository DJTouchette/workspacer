use anyhow::{Result, bail, ensure};
use std::time::Duration;

pub(super) async fn external(endpoint: &str, budget: Duration) -> Result<()> {
    // Reuse the retained adapter's URL policy before making any probe.
    let _ = crate::services::external_claudemon::ExternalDaemon::new(endpoint)?;
    let mut base = url::Url::parse(endpoint)?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let address = base.join("health")?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3));
    if base.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        builder = builder.no_proxy();
    }
    let client = builder.build()?;
    if budget.is_zero() {
        return probe(&client, address).await;
    }
    tokio::time::timeout(budget,async {
        loop {
            if probe(&client,address.clone()).await.is_ok(){return}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.map_err(|_|anyhow::anyhow!("external claudemon failed readiness: expected its maintenance health marker and ok body"))?;
    Ok(())
}
async fn probe(client: &reqwest::Client, address: url::Url) -> Result<()> {
    let mut response = client.get(address).send().await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK
            && response
                .headers()
                .get("X-Workspacer-Maintenance")
                .is_some_and(|value| value == "1"),
        "external claudemon health identity mismatch"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > 512 {
            bail!("external claudemon health response is too large")
        }
        body.extend_from_slice(&chunk);
    }
    ensure!(
        body.as_slice() == b"ok",
        "external claudemon health body mismatch"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn external_health_requires_exact_identity_and_never_follows_redirects() {
        use axum::{Router, response::IntoResponse, routing::get};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let redirected = Arc::new(AtomicUsize::new(0));
        let observed = redirected.clone();
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap();
        let target_task = tokio::spawn(async move {
            axum::serve(
                target,
                Router::new().route(
                    "/",
                    get(move || {
                        let observed = observed.clone();
                        async move {
                            observed.fetch_add(1, Ordering::SeqCst);
                            "ok"
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        for (status, marker, body, good) in [
            (200, "1", "ok", true),
            (200, "", "ok", false),
            (200, "1", "healthy", false),
            (200, "1", "ok\n", false),
            (503, "1", "ok", false),
            (302, "1", "ok", false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/health",
                        get(move || async move {
                            let mut response =
                                (axum::http::StatusCode::from_u16(status).unwrap(), body)
                                    .into_response();
                            response
                                .headers_mut()
                                .insert("X-Workspacer-Maintenance", marker.parse().unwrap());
                            response.headers_mut().insert(
                                "location",
                                format!("http://{target_addr}/").parse().unwrap(),
                            );
                            response
                        }),
                    ),
                )
                .await
                .unwrap();
            });
            assert_eq!(
                external(&format!("http://{address}"), Duration::ZERO)
                    .await
                    .is_ok(),
                good,
                "{status} {marker} {body}"
            );
            task.abort();
            let _ = task.await;
        }
        assert_eq!(redirected.load(Ordering::SeqCst), 0);
        target_task.abort();
        let _ = target_task.await;
    }
}
