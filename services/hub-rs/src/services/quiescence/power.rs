//! Host-power settings and baseline projection. Environment configuration alone
//! grants no action authority; the owned machine-power controller must install
//! an explicit provider and apply caller guards before advertising canStop.
use super::Reading;
use serde_json::{Value, json};
#[derive(Clone, Debug, Default)]
pub struct PowerConfig {
    pub idle_timeout_ms: i64,
    pub requested_stop: bool,
    pub wake_url: String,
    pub label: String,
}
pub fn duration_ms(value: &str) -> Option<i64> {
    let mut value = value.trim();
    if value == "0" {
        return Some(0);
    }
    let sign = if let Some(rest) = value.strip_prefix('-') {
        value = rest;
        -1.
    } else {
        if let Some(rest) = value.strip_prefix('+') {
            value = rest;
        }
        1.
    };
    if value.is_empty() {
        return None;
    }
    let number = regex::Regex::new(r"^(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)").unwrap();
    let mut total = 0.;
    while !value.is_empty() {
        let part = number.find(value)?;
        let n: f64 = part.as_str().parse().ok()?;
        value = &value[part.end()..];
        let (unit, multiplier) = [
            ("ms", 1.),
            ("us", 0.001),
            ("µs", 0.001),
            ("μs", 0.001),
            ("ns", 0.000001),
            ("h", 3_600_000.),
            ("m", 60_000.),
            ("s", 1000.),
        ]
        .into_iter()
        .find(|(unit, _)| value.starts_with(unit))?;
        value = &value[unit.len()..];
        total += n * multiplier;
    }
    total *= sign;
    if !total.is_finite() || total < i64::MIN as f64 || total > i64::MAX as f64 {
        None
    } else {
        Some(total as i64)
    }
}
impl PowerConfig {
    pub fn from_environment() -> Self {
        Self::from_get(|key| std::env::var(key).unwrap_or_default())
    }
    pub fn from_get(get: impl Fn(&str) -> String) -> Self {
        let mode = get("WKS_MACHINE_IDLE_MODE");
        let raw = get("WKS_MACHINE_IDLE_TIMEOUT");
        let duration = duration_ms(&raw).filter(|ms| *ms >= 600_000).unwrap_or(0);
        let idle_timeout_ms =
            if matches!(mode.as_str(), "" | "observe" | "stop") && raw.trim() != "off" {
                duration
            } else {
                0
            };
        let mut config = Self {
            idle_timeout_ms,
            requested_stop: mode == "stop",
            ..Self::default()
        };
        if get("WKS_MACHINE_POWER") != "fly" || get("WKS_MACHINE_WAKE") != "http" {
            return config;
        }
        let wake = get("WKS_MACHINE_WAKE_URL");
        if !wake.is_empty() {
            let Ok(url) = url::Url::parse(&wake) else {
                return config;
            };
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return config;
            }
            config.wake_url = wake;
        }
        let app = get("FLY_APP_NAME");
        let machine = get("FLY_MACHINE_ID");
        let token = get("FLY_API_TOKEN");
        let file = get("FLY_API_TOKEN_FILE");
        let token_available = !token.trim().is_empty()
            || !file.is_empty()
                && std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024)
                && std::fs::read_to_string(file).is_ok_and(|s| !s.trim().is_empty());
        if !app.trim().is_empty() && !machine.trim().is_empty() && token_available {
            config.label = app.trim().into();
        }
        config
    }
    pub fn info(&self, idle: Option<Reading>) -> Value {
        let idle = idle.map(|r| serde_json::to_value(r).unwrap()).unwrap_or(
            json!({"quiescent":false,"since":null,"blockers":[],"dwellSeconds":0,"calmSeconds":0}),
        );
        json!({"canStop":false,"wake":"http","wakeUrl":self.wake_url,"label":self.label,"stopping":false,"error":"","idleTimeoutSeconds":self.idle_timeout_ms/1000,"idleMode":if self.idle_timeout_ms>0{"observe"}else{"off"},"idle":idle})
    }
}
