//! Reuse the existing Rust model/window implementation. The Go migration must
//! not introduce a second Rust copy of model identity or provider argv rules.
pub use claudemon::session::windows::*;

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

const PROVIDERS: &[&str] = &["claude", "codex", "copilot", "opencode", "pi"];

/// Generic JSON/YAML config maps may represent an integral number as a float.
/// Match the existing config writers without accepting fractions or zero.
pub fn config_window(value: &Value) -> Result<Option<u64>> {
    if value.is_null() {
        return Ok(None);
    }
    if let Some(value) = value.as_u64().filter(|v| *v > 0) {
        return Ok(Some(value));
    }
    if let Some(value) = value
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0 && v.fract() == 0.0 && *v < (u64::MAX as f64))
    {
        return Ok(Some(value as u64));
    }
    bail!("invalid-context-window")
}

pub fn context_for_new_spawn(provider: &str, context: Option<u64>, resume: bool) -> Option<u64> {
    context.or_else(|| {
        (!resume && provider.trim().eq_ignore_ascii_case("codex"))
            .then_some(DEFAULT_CODEX_CONTEXT_WINDOW)
    })
}

pub fn foreign_manager_model(provider: &str, model: &str) -> bool {
    let model = model.trim();
    let lower = model.to_lowercase();
    let claude = matches!(
        lower.as_str(),
        "default" | "haiku" | "sonnet" | "sonnet[1m]" | "opus" | "opusplan" | "fable"
    ) || lower.starts_with("claude-");
    let codex = regex::Regex::new(r"^(gpt-|o[0-9]|codex-|gpt[0-9])")
        .unwrap()
        .is_match(&lower);
    let copilot = lower == "auto";
    let slash = regex::Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.:-]+$")
        .unwrap()
        .is_match(model);
    (claude || codex || copilot || slash)
        && !match provider {
            "claude" => claude,
            "codex" => codex,
            "copilot" => copilot,
            "opencode" | "pi" => slash,
            _ => false,
        }
}

/// Returns only the supplied manager preference maps, preserving presence and
/// explicit null context requests. Strict mode is for writes; reads discard
/// malformed entries without discarding the rest of an existing config.
pub fn manager_preferences(agents: &Value, strict: bool) -> Result<Value> {
    let mut output = Map::new();
    for (name, error) in [
        ("managerModels", "invalid-manager-model"),
        ("managerEfforts", "invalid-manager-effort"),
        ("managerContextWindows", "invalid-context-window"),
    ] {
        let raw = &agents[name];
        if raw.is_null() {
            continue;
        }
        let Some(map) = raw.as_object() else {
            if strict {
                bail!("invalid-manager-map: agents.{name} must be an object");
            } else {
                output.insert(name.into(), json!({}));
                continue;
            }
        };
        let mut result = Map::new();
        for (provider, value) in map {
            if !PROVIDERS.contains(&provider.as_str()) {
                if strict {
                    bail!("{error}: agents.{name}.{provider}");
                } else {
                    continue;
                }
            }
            if name == "managerContextWindows" {
                if !matches!(provider.as_str(), "claude" | "codex") {
                    if strict {
                        bail!("unsupported-context-window");
                    } else {
                        continue;
                    }
                }
                let window = match config_window(value) {
                    Ok(window) => window,
                    Err(error) if strict => return Err(error),
                    Err(_) => continue,
                };
                result.insert(provider.clone(), json!(window));
            } else {
                let Some(text) = value.as_str() else {
                    if strict {
                        bail!("{error}");
                    } else {
                        continue;
                    }
                };
                if name == "managerModels"
                    && !text.trim().is_empty()
                    && foreign_manager_model(provider, text)
                {
                    if strict {
                        bail!("foreign-manager-model");
                    } else {
                        continue;
                    }
                }
                result.insert(provider.clone(), json!(text.trim()));
            }
        }
        output.insert(name.into(), Value::Object(result));
    }
    let model = output
        .get("managerModels")
        .and_then(|m| m["claude"].as_str())
        .unwrap_or("")
        .to_owned();
    if !model.trim().is_empty() {
        let requested = output
            .get("managerContextWindows")
            .and_then(|m| m.get("claude"));
        let present = requested.is_some();
        let window = requested.and_then(Value::as_u64);
        let mut selection =
            normalize_model_selection(&model, window).map_err(|e| e.code().to_owned());
        if let Ok(selected) = &selection {
            let inherent = selected.model.to_ascii_lowercase().contains("fable")
                || selected.model.to_ascii_lowercase().contains("mythos");
            if selected
                .context_window
                .is_some_and(|w| !matches!(w, 200_000 | 1_000_000) || (w == 200_000 && inherent))
            {
                selection = Err("unsupported-context-window".into());
            }
        }
        if selection.is_err() && !strict {
            selection = normalize_model_selection(&model, None).map_err(|e| e.code().to_owned());
        }
        match selection {
            Ok(selection) => {
                output.get_mut("managerModels").unwrap()["claude"] = json!(selection.model);
                if present || selection.context_window.is_some() {
                    output.entry("managerContextWindows").or_insert(json!({}))["claude"] =
                        json!(selection.context_window);
                }
            }
            Err(error) if strict => bail!("{error}"),
            Err(_) => {
                output
                    .get_mut("managerModels")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("claude");
                if let Some(contexts) = output.get_mut("managerContextWindows") {
                    contexts.as_object_mut().unwrap().remove("claude");
                }
            }
        }
    }
    Ok(Value::Object(output))
}
