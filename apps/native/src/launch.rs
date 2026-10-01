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
        models.push(ModelChoice {
            id: id.into(),
            label,
            windows: window.into_iter().collect(),
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
                windows: vec![200000, 1000000]
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
        assert_eq!(models[1].id, "model-b[1m]");
        assert!(parse_models("codex", json!({"error":"no catalog"})).is_err());
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
        assert!(Permission::AcceptEdits.wire("codex").is_err());
    }
}
