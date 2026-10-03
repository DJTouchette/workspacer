//! Native launch choices. Catalogs come from the connected hub, never local CLIs.
use anyhow::{Result, ensure};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    #[default]
    Ask,
    AcceptEdits,
    Plan,
    FullAccess,
}

impl Permission {
    pub fn choices(provider: &str) -> &'static [Self] {
        if provider == "claude" {
            &[Self::Ask, Self::AcceptEdits, Self::Plan, Self::FullAccess]
        } else {
            &[Self::Ask, Self::FullAccess]
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask to approve",
            Self::AcceptEdits => "Accept edits",
            Self::Plan => "Plan mode",
            Self::FullAccess => "Full access",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Ask => "The agent asks when a tool needs approval.",
            Self::AcceptEdits => "Allow file edits; other tools may still need approval.",
            Self::Plan => "Explore and plan before making changes.",
            Self::FullAccess => "Run tools without approval prompts.",
        }
    }

    pub fn wire(self, provider: &str) -> Result<&'static str> {
        ensure!(
            Self::choices(provider).contains(&self),
            "Permission mode is unavailable for this provider"
        );
        Ok(match (provider, self) {
            ("claude", Self::Ask) => "default",
            ("claude", Self::FullAccess) => "bypassPermissions",
            (_, Self::Ask) => "ask",
            (_, Self::FullAccess) => "yolo",
            (_, Self::AcceptEdits) => "acceptEdits",
            (_, Self::Plan) => "plan",
        })
    }
}

pub fn absolute_directory(cwd: &str) -> bool {
    let cwd = cwd.trim();
    let bytes = cwd.as_bytes();
    !cwd.contains('\0')
        && (cwd.starts_with('/')
            || cwd.starts_with("\\\\")
            || (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\')))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub windows: Vec<u64>,
    pub is_default: bool,
    /// Reasoning-effort ids this model accepts, as its provider reports them
    /// (Codex `model/list`); empty when the catalog does not say.
    pub efforts: Vec<String>,
    /// The level a launch runs at when no effort is sent, when reported.
    pub default_effort: Option<String>,
}

/// Claude Code's `--effort` ladder. Claude reports no per-model list; this is
/// the launch flag's vocabulary (the desktop's `CLAUDE_EFFORT_LEVELS`), not the
/// wider `/effort` command's, so a launch never sends a level the flag rejects.
pub const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// Codex's ladder until the live catalog says what the chosen model accepts.
pub const CODEX_FALLBACK_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// The effort levels a launch may request for this provider and model choice.
/// `model` is the catalog row for the chosen ID; `None` for Provider default
/// or a custom ID. Provider default uses the catalog's default model when one
/// is marked, since that is the model the launch will actually run.
pub fn effort_levels(
    provider: &str,
    model: Option<&ModelChoice>,
    catalog: &[ModelChoice],
) -> Vec<String> {
    if provider == "claude" {
        return CLAUDE_EFFORTS.iter().map(|s| (*s).to_owned()).collect();
    }
    let row = model.or_else(|| catalog.iter().find(|m| m.is_default));
    match row {
        Some(row) if !row.efforts.is_empty() => row.efforts.clone(),
        _ => CODEX_FALLBACK_EFFORTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    }
}

pub fn effort_label(id: &str) -> String {
    match id {
        "none" => "None".into(),
        "minimal" => "Minimal".into(),
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra high".into(),
        "max" => "Max".into(),
        "ultra" => "Ultra".into(),
        other => other.to_owned(),
    }
}

impl ModelChoice {
    pub fn picker_label(&self) -> String {
        let label = if self.label.eq_ignore_ascii_case(&self.id) {
            self.label.clone()
        } else {
            format!("{} · {}", self.label, self.id)
        };
        if self.is_default {
            format!("{label} (default)")
        } else {
            label
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogKey {
    pub provider: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub key: CatalogKey,
    pub loading: bool,
    pub models: Vec<ModelChoice>,
    pub error: Option<String>,
}

/// Effort ids are short ASCII words; anything else is not sent to a provider.
pub fn valid_effort(effort: &str) -> bool {
    !effort.is_empty()
        && effort.len() <= 32
        && effort
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether a launch error says the agent may have started anyway: the hub's
/// "launch admission may have executed … inspect its outcome before retrying"
/// and "do not retry blindly", or this client's own outcome-unknown receipts.
/// Such a failure is not a clean refusal; starting again could launch twice.
pub fn uncertain_outcome(error: &str) -> bool {
    let error = error.to_lowercase();
    [
        "may have executed",
        "outcome unknown",
        "outcome is unknown",
        "do not retry",
    ]
    .iter()
    .any(|phrase| error.contains(phrase))
}

pub fn parse_models(provider: &str, value: Value) -> Result<Vec<ModelChoice>> {
    let rows = if provider == "claude" {
        value.get("aliases")
    } else {
        Some(&value)
    }
    .and_then(Value::as_array)
    .ok_or_else(|| anyhow::anyhow!("Hub returned an invalid model catalog"))?;
    let mut models: Vec<ModelChoice> = Vec::new();
    for row in rows.iter().take(500) {
        let raw = if provider == "claude" {
            row["model"].as_str().or_else(|| row["value"].as_str())
        } else {
            row["id"].as_str()
        };
        let Some(raw) = raw.filter(|id| !id.trim().is_empty() && !id.starts_with('<')) else {
            continue;
        };
        let id = if provider == "claude" {
            raw.strip_suffix("[1m]").unwrap_or(raw)
        } else {
            raw
        };
        let window = if provider == "claude" {
            row["contextWindow"]
                .as_u64()
                .filter(|w| *w > 0)
                .or_else(|| raw.ends_with("[1m]").then_some(1_000_000))
        } else {
            None
        };
        if let Some(existing) = models.iter_mut().find(|m| m.id == id) {
            if let Some(window) = window
                && !existing.windows.contains(&window)
            {
                existing.windows.push(window);
            }
            continue;
        }
        // Claude labels currently infer versions from historical transcripts.
        // Show the actual family alias instead of claiming that inferred version.
        let label = if provider == "claude" {
            let mut chars = id.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        } else {
            row["label"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_owned()
        };
        let efforts = row["effortLevels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|e| valid_effort(e))
            .map(str::to_owned)
            .collect();
        models.push(ModelChoice {
            id: id.into(),
            label,
            windows: window.into_iter().collect(),
            is_default: row["default"].as_bool().unwrap_or(false),
            efforts,
            default_effort: row["defaultEffort"]
                .as_str()
                .filter(|e| valid_effort(e))
                .map(Into::into),
        });
    }
    for model in &mut models {
        model.windows.sort_unstable();
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uncertain_outcomes_are_told_apart_from_clean_refusals() {
        for error in [
            "launch admission may have executed for session s: daemon returned 503; inspect its outcome before retrying",
            "spawn failed; launch admission may have executed for session s; reservation retained, do not retry blindly",
            "Disconnected; outcome unknown",
            "Spawn returned no session ID; outcome unknown. Refresh sessions before retrying",
        ] {
            assert!(uncertain_outcome(error), "{error}");
        }
        for error in [
            "Choose a project folder first.",
            "provider executable not found",
            "",
        ] {
            assert!(!uncertain_outcome(error), "{error}");
        }
    }

    #[test]
    fn claude_families_group_windows_without_inventing_versions_from_history() {
        let models = parse_models(
            "claude",
            json!({"aliases": [
            {"model":"opus","label":"Opus 4.99","contextWindow":200000},
            {"model":"opus","label":"Opus 4.99","contextWindow":1000000},
            {"value":"sonnet[1m]","label":"Sonnet"},
            {"model":"opus","contextWindow":1000000},
            {"model":"<synthetic>"}
        ],"seen":["claude-old-model"]}),
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0],
            ModelChoice {
                id: "opus".into(),
                label: "Opus".into(),
                windows: vec![200000, 1000000],
                is_default: false,
                ..Default::default()
            }
        );
        assert_eq!(models[1].id, "sonnet");
        assert_eq!(models[1].windows, vec![1000000]);
    }

    #[test]
    fn live_models_preserve_exact_ids_and_labels() {
        let models = parse_models(
            "codex",
            json!([
                {"id":"model-a","label":"Model A","default":true},
                {"id":"model-a","label":"Duplicate"},
                {"id":"model-b[1m]","label":""},
                {"id":""}
            ]),
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].label, "Model A");
        assert_eq!(models[0].picker_label(), "Model A · model-a (default)");
        assert_eq!(models[1].id, "model-b[1m]");
        assert!(parse_models("codex", json!({"error":"no catalog"})).is_err());
    }

    #[test]
    fn codex_catalog_uses_live_display_labels_without_rewriting_launch_ids() {
        let models = parse_models(
            "codex",
            json!([
                {"id":"gpt-6.1-sol","label":"GPT-6.1-Sol","default":true},
                {"id":"account-special","label":"GPT-6-Astra","default":false}
            ]),
        )
        .unwrap();
        assert_eq!(models[0].picker_label(), "GPT-6.1-Sol (default)");
        assert_eq!(models[1].picker_label(), "GPT-6-Astra · account-special");
        assert!(models[0].is_default);
        let request = crate::controller::NewSession {
            provider: "codex".into(),
            cwd: "/project".into(),
            model: models[1].id.clone(),
            ..Default::default()
        };
        assert_eq!(request.params().unwrap()["model"], "account-special");
    }

    #[test]
    fn permissions_and_context_reach_spawn_without_cross_provider_modes() {
        use crate::controller::NewSession;
        for provider in ["claude", "codex"] {
            for &permission in Permission::choices(provider) {
                let request = NewSession {
                    provider: provider.into(),
                    cwd: "/project".into(),
                    model: "exact-model".into(),
                    context_window: Some(1000000),
                    permission,
                    ..Default::default()
                };
                let params = request.params().unwrap();
                assert_eq!(params["permissionMode"], permission.wire(provider).unwrap());
                assert_eq!(
                    params["skipPermissions"],
                    permission == Permission::FullAccess
                );
                assert_eq!(params["model"], "exact-model");
                assert_eq!(params["contextWindow"], 1000000);
            }
        }
        assert!(Permission::Plan.wire("codex").is_err());
        assert!(Permission::AcceptEdits.wire("claude").is_ok());
    }

    #[test]
    fn effort_follows_the_model_catalog_and_reaches_spawn_only_when_chosen() {
        let models = parse_models(
            "codex",
            json!([
                {"id":"sol","label":"Sol","default":true,"effortLevels":["low","medium","high","xhigh"],"defaultEffort":"medium"},
                {"id":"mini","label":"Mini","effortLevels":["minimal","low","bad effort!"],"defaultEffort":"low"},
                {"id":"plain","label":"Plain"}
            ]),
        )
        .unwrap();
        assert_eq!(
            models[1].efforts,
            ["minimal", "low"],
            "malformed ids are dropped"
        );
        assert_eq!(models[0].default_effort.as_deref(), Some("medium"));
        assert_eq!(
            effort_levels("codex", Some(&models[1]), &models),
            ["minimal", "low"]
        );
        assert_eq!(
            effort_levels("codex", None, &models),
            ["low", "medium", "high", "xhigh"],
            "provider default runs the catalog's default model"
        );
        assert_eq!(
            effort_levels("codex", Some(&models[2]), &models),
            CODEX_FALLBACK_EFFORTS
        );
        assert_eq!(effort_levels("claude", None, &[]), CLAUDE_EFFORTS);
        use crate::controller::NewSession;
        let mut request = NewSession {
            provider: "codex".into(),
            cwd: "/project".into(),
            ..Default::default()
        };
        assert!(
            request.params().unwrap().get("effort").is_none(),
            "default sends nothing"
        );
        request.effort = "xhigh".into();
        assert_eq!(request.params().unwrap()["effort"], "xhigh");
        request.effort = "x; rm".into();
        assert!(request.params().is_err());
        assert!(Permission::AcceptEdits.wire("codex").is_err());
    }
}
