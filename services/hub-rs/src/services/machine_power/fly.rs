use super::*;
use anyhow::Context;
use reqwest::{Client, Method};
use std::path::PathBuf;
struct Fly {
    http: Client,
    base: url::Url,
    app: String,
    id: String,
    token: String,
    gate: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}
impl Fly {
    fn new(base: &str, app: String, id: String, token: String) -> Result<Self> {
        anyhow::ensure!(
            !app.is_empty() && !id.is_empty() && !token.is_empty(),
            "machine power identity is incomplete"
        );
        anyhow::ensure!(
            [&app, &id]
                .into_iter()
                .all(|s| !matches!(s.as_str(), "." | "..")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))),
            "machine power identity has invalid characters"
        );
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base: url::Url::parse(base)?,
            app,
            id,
            token,
            gate: tokio::sync::Mutex::new(None),
        })
    }
    async fn request(&self, stop: bool) -> Result<()> {
        // One serialized stop/read lane prevents repeated explicit retries from
        // overwhelming this machine's provider control plane.
        let mut gate = self.gate.lock().await;
        if let Some(last) = *gate {
            let delay = if stop {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(200)
            };
            tokio::time::sleep_until(last + delay).await;
        }
        *gate = Some(tokio::time::Instant::now());
        drop(gate);
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("machine power endpoint invalid"))?;
            path.clear()
                .extend(["v1", "apps", &self.app, "machines", &self.id]);
            if stop {
                path.push("stop");
            }
        }
        let request = self
            .http
            .request(if stop { Method::POST } else { Method::GET }, url)
            .bearer_auth(&self.token)
            .header("Accept", "application/json");
        let request = if stop {
            request.json(&json!({"signal":"SIGTERM","timeout":"45s"}))
        } else {
            request
        };
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("machine power provider request failed"))?;
        anyhow::ensure!(
            response.status().is_success(),
            "machine power provider refused request"
        );
        if !stop {
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| anyhow::anyhow!("machine power provider read failed"))?
            {
                anyhow::ensure!(
                    bytes.len() + chunk.len() <= 64 * 1024,
                    "machine power provider response too large"
                );
                bytes.extend_from_slice(&chunk);
            }
            if !bytes.is_empty() {
                let row: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    anyhow::anyhow!("machine power provider returned invalid state")
                })?;
                anyhow::ensure!(
                    row.is_object(),
                    "machine power provider returned invalid state"
                );
            }
        }
        Ok(())
    }
}
impl PowerProvider for Fly {
    fn check(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.request(false))
    }
    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.request(true))
    }
}
/// Call only from an explicitly owned standalone launcher. Merely linking the
/// hub into a GUI or library never grants cloud-machine shutdown authority.
pub fn standalone_from_environment() -> Result<Option<Arc<dyn PowerProvider>>> {
    configured(|key| std::env::var(key).unwrap_or_default())
}
fn configured(get: impl Fn(&str) -> String) -> Result<Option<Arc<dyn PowerProvider>>> {
    if get("WKS_MACHINE_POWER") != "fly" || get("WKS_MACHINE_WAKE") != "http" {
        return Ok(None);
    }
    let wake = get("WKS_MACHINE_WAKE_URL");
    if !wake.is_empty() {
        let url = url::Url::parse(&wake).context("machine wake URL invalid")?;
        anyhow::ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "machine wake URL must be credential-free HTTPS"
        );
    }
    let app = get("FLY_APP_NAME").trim().to_string();
    let id = get("FLY_MACHINE_ID").trim().to_string();
    let mut token = get("FLY_API_TOKEN").trim().to_string();
    if token.is_empty() {
        let path = get("FLY_API_TOKEN_FILE");
        if !path.is_empty() {
            let path = PathBuf::from(path);
            let metadata = std::fs::metadata(&path)
                .map_err(|_| anyhow::anyhow!("machine power credential file unreadable"))?;
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= 64 * 1024,
                "machine power credential file invalid"
            );
            token = std::fs::read_to_string(path)
                .map_err(|_| anyhow::anyhow!("machine power credential file unreadable"))?
                .trim()
                .into();
        }
    }
    if app.is_empty() || id.is_empty() || token.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(Fly::new(
        "https://api.machines.dev",
        app,
        id,
        token,
    )?)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_authority_requires_explicit_host_optins_and_credentials() {
        let values = std::collections::BTreeMap::from([
            ("WKS_MACHINE_POWER", "fly"),
            ("WKS_MACHINE_WAKE", "http"),
            ("FLY_APP_NAME", "app"),
            ("FLY_MACHINE_ID", "id"),
            ("FLY_API_TOKEN", "fixture-secret"),
        ]);
        assert!(
            configured(|k| values.get(k).unwrap_or(&"").to_string())
                .unwrap()
                .is_some()
        );
        for absent in [
            "WKS_MACHINE_POWER",
            "WKS_MACHINE_WAKE",
            "FLY_APP_NAME",
            "FLY_MACHINE_ID",
            "FLY_API_TOKEN",
        ] {
            assert!(
                configured(|k| if k == absent {
                    String::new()
                } else {
                    values.get(k).unwrap_or(&"").to_string()
                })
                .unwrap()
                .is_none()
            );
        }
        assert!(
            configured(|k| if k == "WKS_MACHINE_WAKE_URL" {
                "https://example.test/?secret=x".into()
            } else {
                values.get(k).unwrap_or(&"").to_string()
            })
            .is_err()
        );
    }
    #[tokio::test]
    async fn fly_self_stop_uses_fixed_host_coordinates_and_grace() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0u8; 2048];
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..at]);
                        let len = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= at + 4 + len {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{\"state\":\"started\"}").await.unwrap();
            }
            requests
        });
        let provider = Fly::new(
            &base,
            "own-app".into(),
            "own-id".into(),
            "fixture-secret".into(),
        )
        .unwrap();
        provider.check().await.unwrap();
        provider.stop().await.unwrap();
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /v1/apps/own-app/machines/own-id "));
        assert!(requests[1].starts_with("POST /v1/apps/own-app/machines/own-id/stop "));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-secret")
        );
        let body = requests[1].split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            json!({"signal":"SIGTERM","timeout":"45s"})
        );
    }
}
