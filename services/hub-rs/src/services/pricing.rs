//! Editable software cost estimates; never provider billing or live prices.
use crate::{Caller, Options};
use anyhow::{Context, Result, bail};
use claudemon::session::{pricing, windows};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
pub struct Pricing {
    pub path: PathBuf,
    lock: Mutex<()>,
}
impl Pricing {
    pub fn new(home: PathBuf) -> Self {
        Self {
            path: home.join(".workspacer/model-rates.json"),
            lock: Mutex::new(()),
        }
    }
    pub fn overrides(&self) -> Value {
        // Compatibility: malformed/unreadable edits leave builtin estimates active.
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(|v| v.is_object() || v.is_array())
            .unwrap_or(json!({}))
    }
    pub fn defaults() -> Value {
        let mut out = json!({});
        for (prefix, rate) in pricing::builtin_rates() {
            let mut row = json!({"input":rate.input,"output":rate.output,"contextLimit":windows::window_for(prefix)});
            if let Some(n) = rate.cached_input {
                row["cachedInput"] = n.into();
            }
            out[*prefix] = row;
        }
        out
    }
    pub fn get(&self) -> Value {
        json!({"defaults":Self::defaults(),"overrides":self.overrides()})
    }
    pub fn save(&self, overrides: &Value) -> Result<Value> {
        let map = overrides
            .as_object()
            .context("Rate overrides must be an object")?;
        for row in map.values() {
            for key in ["input", "output"] {
                if !row[key].as_f64().is_some_and(|n| n.is_finite() && n >= 0.) {
                    bail!("Rates must have nonnegative finite input and output values");
                }
            }
            for key in ["cached_input", "context_limit"] {
                if row
                    .get(key)
                    .is_some_and(|v| !v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.))
                {
                    bail!("Invalid {key}");
                }
            }
        }
        let _guard = self.lock.lock().unwrap();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _file = crate::auth::StoreLock::take(&self.path.with_extension("json.rust.lock"))?;
        if map.is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        } else {
            super::config::atomic_bytes(&self.path, &serde_json::to_vec_pretty(overrides)?)?;
        }
        Ok(json!({"ok":true}))
    }
    pub fn turn_cost(&self, model: Option<&str>, usage: &Value) -> f64 {
        Self::turn_cost_with(&self.overrides(), model, usage)
    }
    pub fn turn_cost_with(overrides: &Value, model: Option<&str>, usage: &Value) -> f64 {
        let mut rates = (3., 15., None);
        let mut longest = None;
        if let Some(model) = model.filter(|m| !m.is_empty()) {
            for (prefix, r) in pricing::builtin_rates() {
                if model.starts_with(prefix) && longest.is_none_or(|n| prefix.len() > n) {
                    rates = (r.input, r.output, r.cached_input);
                    longest = Some(prefix.len());
                }
            }
            if let Some(overrides) = overrides.as_object() {
                for (prefix, r) in overrides {
                    if model.starts_with(prefix) && longest.is_none_or(|n| prefix.len() >= n) {
                        if let (Some(i), Some(o)) = (r["input"].as_f64(), r["output"].as_f64()) {
                            rates = (i, o, r["cached_input"].as_f64());
                            longest = Some(prefix.len());
                        }
                    }
                }
            }
        }
        let n = |v: &Value| {
            v.as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
                .unwrap_or(0.)
        };
        let write = n(&usage["cache_creation_input_tokens"]);
        let five = &usage["cache_creation"]["ephemeral_5m_input_tokens"];
        let hour = &usage["cache_creation"]["ephemeral_1h_input_tokens"];
        let write_cost = if five.is_null() && hour.is_null() {
            write * 2.
        } else {
            n(five) * 1.25 + n(hour) * 2. + (write - n(five) - n(hour)).max(0.) * 2.
        };
        (n(&usage["input_tokens"]) * rates.0
            + write_cost * rates.0
            + n(&usage["cache_read_input_tokens"]) * rates.2.unwrap_or(rates.0 * 0.1)
            + n(&usage["output_tokens"]) * rates.1)
            / 1_000_000.
    }
}
pub fn install(mut options: Options, service: Arc<Pricing>) -> Options {
    for method in ["desktop.pricingGetRates", "desktop.pricingSaveOverrides"] {
        let service = service.clone();
        options = options.handler(method, move |caller: Caller, params| {
            let service = service.clone();
            async move {
                if !caller.authenticated_host || !caller.trusted {
                    bail!("pricing requires an authenticated desktop user");
                }
                tokio::task::spawn_blocking(move || {
                    if method == "desktop.pricingGetRates" {
                        Ok(service.get())
                    } else {
                        service.save(&params["overrides"])
                    }
                })
                .await?
            }
        });
    }
    options
}
