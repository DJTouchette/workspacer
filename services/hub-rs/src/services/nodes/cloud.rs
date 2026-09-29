//! The only cloud verbs are read/start/stop/wait. Endpoint, identifiers and
//! credentials are fixed from host configuration, never RPC parameters.
use super::model::CloudConfig;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Started,
    Starting,
    Replacing,
    Stopped,
    Suspended,
    Destroyed,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Status(u16),
    RateLimited,
    Timeout,
    Unavailable,
    Invalid,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(404) => write!(f, "cloud app or machine was not found"),
            Self::Status(401 | 403) => write!(f, "cloud credential was rejected"),
            Self::Status(status) => write!(f, "cloud request failed with HTTP {status}"),
            Self::RateLimited => write!(f, "cloud API is rate-limiting this machine"),
            Self::Timeout => write!(f, "cloud API did not answer in time"),
            Self::Unavailable => write!(f, "cloud API could not be reached"),
            Self::Invalid => write!(f, "cloud API returned an invalid response"),
        }
    }
}
impl std::error::Error for Failure {}
pub type Outcome<T> = std::result::Result<T, Failure>;
pub trait Cloud: Send + Sync + 'static {
    fn start(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>>;
    fn stop(
        &self,
        grace: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>>;
    fn state(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<State>> + Send + '_>>;
    fn wait(
        &self,
        state: State,
        timeout: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>>;
}
pub struct Http {
    client: reqwest::Client,
    base: url::Url,
    token: String,
    gates: tokio::sync::Mutex<std::collections::BTreeMap<&'static str, tokio::time::Instant>>,
}
impl Http {
    pub fn new(config: &CloudConfig, token: String) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            !config.app.is_empty() && !config.machine_id.is_empty() && !token.is_empty(),
            "node cloud configuration is incomplete"
        );
        anyhow::ensure!(
            !matches!(config.app.as_str(), "." | "..")
                && !matches!(config.machine_id.as_str(), "." | ".."),
            "node cloud identifiers are invalid"
        );
        let mut base = url::Url::parse(if config.base_url.is_empty() {
            "https://api.machines.dev"
        } else {
            &config.base_url
        })
        .map_err(|_| anyhow::anyhow!("node cloud endpoint is invalid"))?;
        anyhow::ensure!(
            matches!(base.scheme(), "http" | "https")
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "node cloud endpoint is invalid"
        );
        base.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("node cloud endpoint is invalid"))?
            .pop_if_empty()
            .extend(["v1", "apps", &config.app, "machines", &config.machine_id]);
        Ok(Arc::new(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base,
            token,
            gates: tokio::sync::Mutex::new(std::collections::BTreeMap::new()),
        }))
    }
    async fn request(
        &self,
        action: &'static str,
        want: Option<State>,
        timeout: Duration,
        body: Option<Value>,
    ) -> Outcome<Value> {
        let now = tokio::time::Instant::now();
        let interval = if matches!(action, "start" | "stop") {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(200)
        };
        let at = {
            let mut gates = self.gates.lock().await;
            let at = gates
                .get(action)
                .map(|last| (*last + interval).max(now))
                .unwrap_or(now);
            gates.insert(action, at);
            at
        };
        tokio::time::sleep_until(at).await;
        let mut url = self.base.clone();
        if action != "get" {
            url.path_segments_mut()
                .map_err(|_| Failure::Invalid)?
                .push(action);
        }
        if let Some(want) = want {
            let state = match want {
                State::Started => "started",
                State::Stopped => "stopped",
                _ => return Err(Failure::Invalid),
            };
            url.query_pairs_mut()
                .append_pair("state", state)
                .append_pair("timeout", &timeout.as_secs().clamp(1, 60).to_string());
        }
        let request = self
            .client
            .request(
                if matches!(action, "start" | "stop") {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::GET
                },
                url,
            )
            .bearer_auth(&self.token)
            .header("Accept", "application/json");
        let request = if let Some(body) = body {
            request.json(&body)
        } else {
            request
        };
        let request = if action == "wait" {
            request.timeout(timeout.min(Duration::from_secs(60)) + Duration::from_secs(5))
        } else {
            request
        };
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                Failure::Timeout
            } else {
                Failure::Unavailable
            }
        })?;
        let status = response.status().as_u16();
        if status == 429 {
            return Err(Failure::RateLimited);
        }
        if !(200..300).contains(&status) {
            return Err(Failure::Status(status));
        }
        if action != "get" {
            return Ok(Value::Null);
        }
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Unavailable)? {
            if bytes.len() + chunk.len() > 64 * 1024 {
                return Err(Failure::Invalid);
            }
            bytes.extend(chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Failure::Invalid)
    }
}
impl Cloud for Http {
    fn start(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async {
            self.request("start", None, Duration::ZERO, None)
                .await
                .map(|_| ())
        })
    }
    fn stop(
        &self,
        grace: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async move {
            if grace.is_zero() {
                return Err(Failure::Invalid);
            }
            self.request(
                "stop",
                None,
                Duration::ZERO,
                Some(json!({"signal":"SIGTERM","timeout":format!("{}s",grace.as_secs())})),
            )
            .await
            .map(|_| ())
        })
    }
    fn state(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<State>> + Send + '_>> {
        Box::pin(async {
            let value = self.request("get", None, Duration::ZERO, None).await?;
            Ok(match value["state"].as_str() {
                Some("started") => State::Started,
                Some("starting") => State::Starting,
                Some("replacing") => State::Replacing,
                Some("stopped") => State::Stopped,
                Some("suspended") => State::Suspended,
                Some("destroyed") => State::Destroyed,
                _ => State::Unknown,
            })
        })
    }
    fn wait(
        &self,
        state: State,
        timeout: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome<()>> + Send + '_>> {
        Box::pin(async move {
            self.request("wait", Some(state), timeout, None)
                .await
                .map(|_| ())
        })
    }
}
