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
    let negative = value.starts_with('-');
    if value.starts_with(['-', '+']) {
        value = &value[1..];
    }
    if value == "0" {
        return Some(0);
    }
    if value.is_empty() {
        return None;
    }
    // Go time.Duration is a signed nanosecond count, not a millisecond count.
    // Keep its overflow checks exact before converting to our output unit.
    const LIMIT: u64 = 1 << 63;
    let mut total = 0u64;
    while !value.is_empty() {
        let count = value.bytes().take_while(u8::is_ascii_digit).count();
        let mut whole = 0u64;
        for digit in value[..count].bytes() {
            whole = whole.checked_mul(10)?.checked_add((digit - b'0') as u64)?;
            if whole > LIMIT {
                return None;
            }
        }
        value = &value[count..];
        let mut fraction = 0u64;
        let mut scale = 1f64;
        let mut fraction_digits = 0;
        if let Some(rest) = value.strip_prefix('.') {
            fraction_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let mut overflow = false;
            // Match ParseDuration's fractional precision policy: ignore excess
            // digits after its integer accumulator fills, and use float only
            // for the fractional unit conversion (never the duration bound).
            for digit in rest[..fraction_digits].bytes() {
                if overflow {
                    continue;
                }
                if fraction > (LIMIT - 1) / 10 {
                    overflow = true;
                    continue;
                }
                let next = fraction * 10 + (digit - b'0') as u64;
                if next > LIMIT {
                    overflow = true;
                    continue;
                }
                fraction = next;
                scale *= 10.;
            }
            value = &rest[fraction_digits..];
        }
        if count == 0 && fraction_digits == 0 {
            return None;
        }
        let unit_end = value
            .bytes()
            .position(|byte| byte == b'.' || byte.is_ascii_digit())
            .unwrap_or(value.len());
        let unit = match &value[..unit_end] {
            "ns" => 1,
            "us" | "µs" | "μs" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return None,
        };
        value = &value[unit_end..];
        let nanos = whole
            .checked_mul(unit)?
            .checked_add((fraction as f64 * (unit as f64 / scale)) as u64)?;
        total = total.checked_add(nanos)?;
        if total > LIMIT {
            return None;
        }
    }
    if !negative && total == LIMIT {
        return None;
    }
    let millis = (total / 1_000_000) as i64;
    Some(if negative { -millis } else { millis })
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

#[cfg(test)]
mod duration_tests {
    use super::*;

    #[test]
    fn go_signed_nanosecond_bound_precedes_millisecond_conversion() {
        for (input, expected) in [
            ("0", Some(0)),
            ("+0", Some(0)),
            ("-0", Some(0)),
            ("1h2m3.5s", Some(3_723_500)),
            ("-1h2m3.5s", Some(-3_723_500)),
            ("+.5m", Some(30_000)),
            ("1.s", Some(1000)),
            ("1000us", Some(1)),
            ("1000µs", Some(1)),
            ("1000μs", Some(1)),
            ("0.999999ms0.000001ms", Some(1)),
            (".333333333333333333333h", Some(1_200_000)),
            ("9223372036854775807ns", Some(9_223_372_036_854)),
            ("9223372036854775808ns", None),
            ("-9223372036854775808ns", Some(-9_223_372_036_854)),
            ("-9223372036854775809ns", None),
            ("9223372036854775807ns1ns", None),
            ("-9223372036854775807ns1ns", Some(-9_223_372_036_854)),
            ("2562047h47m16.854775807s", Some(9_223_372_036_854)),
            ("2562047h47m16.854775808s", None),
            ("-2562047h47m16.854775808s", Some(-9_223_372_036_854)),
            ("2562048h", None),
            ("18446744073709551616ns", None),
            ("1m-1s", None),
            (".s", None),
            ("1", None),
            ("1d", None),
            ("1 s", None),
        ] {
            assert_eq!(duration_ms(input), expected, "{input}");
        }
        let config = PowerConfig::from_get(|key| match key {
            "WKS_MACHINE_IDLE_MODE" => "stop".into(),
            "WKS_MACHINE_IDLE_TIMEOUT" => "2562048h".into(),
            _ => String::new(),
        });
        assert_eq!(config.idle_timeout_ms, 0);
        assert_eq!(config.info(None)["idleMode"], "off");
    }
}
