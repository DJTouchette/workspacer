//! One authenticated JSON request against a provider's API, retried once
//! with fresh credentials when the provider says the token is no good.
use super::{credentials::Credentials, remote::Kind};
use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::time::Duration;

pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("workspacer")
        .build()
        .expect("reqwest client with static settings")
}

pub(crate) async fn json(
    credentials: &Credentials,
    kind: Kind,
    host: &str,
    request: impl Fn() -> reqwest::RequestBuilder,
) -> Result<Value> {
    for attempt in 0..2 {
        let auth = credentials.get(kind, host).await?;
        let response = request()
            .header("Authorization", auth.header())
            .send()
            .await
            .map_err(|error| anyhow!("{} request failed: {error}", kind.label()))?;
        let status = response.status();
        if (status == reqwest::StatusCode::UNAUTHORIZED
            // Azure DevOps answers an expired token with a sign-in redirect.
            || status == reqwest::StatusCode::NON_AUTHORITATIVE_INFORMATION)
            && attempt == 0
        {
            credentials.forget(kind, host);
            continue;
        }
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{} returned {status}: {}", kind.label(), message(&text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&text)
            .map_err(|_| anyhow!("{} returned a response that is not JSON", kind.label()));
    }
    bail!(
        "{} refused the credentials from `{}`; sign in again",
        kind.label(),
        super::credentials::cli(kind)
    )
}

/// The provider's own error message when it sent one, else a short excerpt.
fn message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(200).collect())
}
