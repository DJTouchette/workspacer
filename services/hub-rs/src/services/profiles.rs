use super::config::{ConfigLock, atomic_bytes};
use crate::Options;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub config_dir: String,
    #[serde(deserialize_with = "strings")]
    pub extra_args: Vec<String>,
    #[serde(deserialize_with = "strings")]
    pub mcp_item_ids: Vec<String>,
    pub is_default: bool,
    #[serde(deserialize_with = "weight", serialize_with = "serialize_weight")]
    pub weight: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub preset: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token_env_var: String,
}
fn strings<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}
fn weight<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<f64, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0))
}
fn serialize_weight<S: serde::Serializer>(
    value: &f64,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && *value >= 0.0 && *value < i64::MAX as f64 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}
impl Profile {
    fn default_profile() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            is_default: true,
            ..Self::default()
        }
    }
    pub fn normalize(&mut self) {
        if !self.weight.is_finite() || self.weight < 0.0 {
            self.weight = 0.0;
        }
        if !matches!(self.provider.as_str(), "codex" | "copilot") {
            self.provider.clear();
        }
        if !self.provider.is_empty() {
            self.mcp_item_ids.clear();
        }
        if self.provider == "copilot" {
            self.weight = 0.0;
        }
        if self.provider == "codex" {
            self.preset = self
                .preset
                .trim()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .collect();
            if !self
                .preset
                .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            {
                self.preset.clear();
            }
        } else {
            self.preset.clear();
        }
        self.token_env_var = self.token_env_var.trim().into();
        if self.provider != "copilot"
            || !regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$")
                .unwrap()
                .is_match(&self.token_env_var)
        {
            self.token_env_var.clear();
        }
    }
}
pub struct Profiles {
    path: PathBuf,
    lock: Mutex<()>,
}
impl Profiles {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("claude-profiles.json"),
            lock: Mutex::new(()),
        }
    }
    fn raw(&self) -> Result<Vec<Profile>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        Ok(
            serde_json::from_value::<Option<Vec<Profile>>>(value["profiles"].clone())?
                .unwrap_or_default(),
        )
    }
    fn save(&self, profiles: &mut [Profile]) -> Result<()> {
        for profile in profiles.iter_mut() {
            profile.normalize();
        }
        atomic_bytes(
            &self.path,
            serde_json::to_vec_pretty(&json!({"profiles":profiles}))?.as_slice(),
        )
    }
    fn normalized(&self) -> Result<Vec<Profile>> {
        let mut profiles = self.raw()?;
        for profile in &mut profiles {
            profile.normalize();
        }
        if profiles.is_empty() {
            profiles.push(Profile::default_profile());
        }
        Ok(profiles)
    }
    fn list_checked(&self) -> Result<Vec<Profile>> {
        let _guard = self.lock.lock().unwrap();
        if !self.raw()?.is_empty() {
            return self.normalized();
        }
        let _file_lock = ConfigLock::take(&self.path)?;
        let mut profiles = self.normalized()?;
        self.save(&mut profiles)?;
        Ok(profiles)
    }
    pub fn list(&self) -> Vec<Profile> {
        match self.list_checked() {
            Ok(profiles) => profiles,
            Err(error) => {
                eprintln!("profile store unavailable: {error}");
                vec![Profile::default_profile()]
            }
        }
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let method = match method {
            "desktop.claudeProfilesAdd" => "claude.profiles.add",
            "desktop.claudeProfilesUpdate" => "claude.profiles.update",
            "desktop.claudeProfilesRemove" => "claude.profiles.remove",
            other => other,
        };
        if method == "claude.profiles.list" {
            return Ok(json!(self.list_checked()?));
        }
        let _guard = self.lock.lock().unwrap();
        let _file_lock = ConfigLock::take(&self.path)?;
        let mut profiles = self.normalized()?;
        match method {
            "claude.profiles.add" => {
                let name = params["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("claude.profiles.add requires {{ name }}"))?;
                let mut profile = Profile {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.into(),
                    config_dir: params["configDir"].as_str().unwrap_or("").trim().into(),
                    extra_args: parse_strings(&params["extraArgs"])?,
                    mcp_item_ids: parse_strings(&params["mcpItemIds"])?,
                    is_default: profiles.is_empty(),
                    provider: params["init"]["provider"].as_str().unwrap_or("").into(),
                    preset: params["init"]["preset"].as_str().unwrap_or("").into(),
                    weight: params["init"]["weight"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .unwrap_or(0.0),
                    token_env_var: params["init"]["tokenEnvVar"].as_str().unwrap_or("").into(),
                };
                profile.normalize();
                profiles.push(profile.clone());
                self.save(&mut profiles)?;
                Ok(json!(profile))
            }
            "claude.profiles.update" => {
                let id = params["id"].as_str().unwrap_or("");
                let update = &params["updates"];
                let index = profiles
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or_else(|| anyhow!("profile {id:?} not found"))?;
                for (key, field) in [("name", &mut profiles[index].name)] {
                    if let Some(value) = update.get(key).filter(|v| !v.is_null()) {
                        *field = value
                            .as_str()
                            .ok_or_else(|| anyhow!("{key} must be text"))?
                            .into();
                    }
                }
                if let Some(value) = update.get("configDir").filter(|v| !v.is_null()) {
                    profiles[index].config_dir = value
                        .as_str()
                        .ok_or_else(|| anyhow!("configDir must be text"))?
                        .trim()
                        .into();
                }
                if let Some(value) = update.get("extraArgs").filter(|v| !v.is_null()) {
                    profiles[index].extra_args = parse_strings(value)?;
                }
                if let Some(value) = update.get("mcpItemIds").filter(|v| !v.is_null()) {
                    profiles[index].mcp_item_ids = parse_strings(value)?;
                }
                let profile = &mut profiles[index];
                for (key, field) in [
                    ("provider", &mut profile.provider),
                    ("preset", &mut profile.preset),
                    ("tokenEnvVar", &mut profile.token_env_var),
                ] {
                    if let Some(value) = update.get(key).filter(|v| !v.is_null()) {
                        *field = value
                            .as_str()
                            .ok_or_else(|| anyhow!("{key} must be text"))?
                            .into();
                    }
                }
                if let Some(value) = update.get("weight") {
                    profiles[index].weight = value
                        .as_f64()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .unwrap_or(0.0);
                }
                if update["isDefault"] == true {
                    for profile in &mut profiles {
                        profile.is_default = profile.id == id;
                    }
                }
                self.save(&mut profiles)?;
                Ok(json!(profiles[index]))
            }
            "claude.profiles.remove" => {
                let id = params["id"].as_str().unwrap_or("");
                if id == "default" {
                    return Ok(json!({"ok":true}));
                }
                profiles.retain(|p| p.id != id);
                if !profiles.iter().any(|p| p.is_default)
                    && let Some(first) = profiles.first_mut()
                {
                    first.is_default = true;
                }
                self.save(&mut profiles)?;
                Ok(json!({"ok":true}))
            }
            _ => bail!("unknown profile method"),
        }
    }
    pub fn get(&self, id: &str) -> Option<Profile> {
        if id.is_empty() {
            None
        } else {
            self.list().into_iter().find(|p| p.id == id)
        }
    }
}
fn parse_strings(value: &Value) -> Result<Vec<String>> {
    if value.is_null() {
        return Ok(vec![]);
    }
    serde_json::from_value(value.clone()).map_err(Into::into)
}
pub fn environment(profile: &Profile, home: &Path) -> BTreeEnv {
    let mut env = BTreeEnv::new();
    let directory = profile.config_dir.trim();
    if !directory.is_empty() {
        let key = match profile.provider.as_str() {
            "codex" => "CODEX_HOME",
            "copilot" => "COPILOT_HOME",
            _ => "CLAUDE_CONFIG_DIR",
        };
        let directory = if let Some(rest) = directory.strip_prefix('~') {
            home.join(rest.trim_start_matches(['/', '\\']))
                .to_string_lossy()
                .into_owned()
        } else {
            directory.into()
        };
        env.insert(key.into(), directory);
    }
    if profile.provider == "copilot"
        && !profile.token_env_var.is_empty()
        && let Ok(value) = std::env::var(&profile.token_env_var)
        && !value.trim().is_empty()
    {
        env.insert("COPILOT_GITHUB_TOKEN".into(), value.trim().into());
    }
    env
}
type BTreeEnv = std::collections::BTreeMap<String, String>;
pub(crate) fn install(mut options: Options, directory: PathBuf) -> Options {
    let profiles = Arc::new(Profiles::new(directory));
    for method in [
        "claude.profiles.list",
        "claude.profiles.add",
        "claude.profiles.update",
        "claude.profiles.remove",
        "desktop.claudeProfilesAdd",
        "desktop.claudeProfilesUpdate",
        "desktop.claudeProfilesRemove",
    ] {
        let profiles = profiles.clone();
        options = options.handler(method, move |_, params| {
            let profiles = profiles.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let result = profiles.call(method, params)?;
                    Ok(if method == "desktop.claudeProfilesRemove" {
                        Value::Null
                    } else {
                        result
                    })
                })
                .await?
            }
        });
    }
    options
}

#[cfg(test)]
mod desktop_tests {
    use super::*;
    #[test]
    fn managed_profile_init_and_provider_switch_round_trip_without_stale_fields() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = Profiles::new(dir.path().into());
        let codex=profiles.call("desktop.claudeProfilesAdd",json!({"name":"Work","configDir":"~/work-codex","mcpItemIds":["claude-only"],"init":{"provider":"codex","preset":"work","weight":3}})).unwrap();
        assert_eq!(codex["provider"], "codex");
        assert_eq!(codex["preset"], "work");
        assert_eq!(codex["weight"], 3);
        assert_eq!(codex["mcpItemIds"], json!([]));
        let switched=profiles.call("desktop.claudeProfilesUpdate",json!({"id":codex["id"],"updates":{"provider":"copilot","tokenEnvVar":"CORPORATE_GITHUB_TOKEN","weight":9}})).unwrap();
        assert_eq!(switched["weight"], 0);
        assert!(switched.get("preset").is_none());
        assert_eq!(switched["tokenEnvVar"], "CORPORATE_GITHUB_TOKEN");
        let claude = profiles
            .call(
                "desktop.claudeProfilesUpdate",
                json!({"id":codex["id"],"updates":{"provider":"claude"}}),
            )
            .unwrap();
        assert!(claude.get("provider").is_none());
        assert!(claude.get("tokenEnvVar").is_none());
        assert_eq!(
            profiles
                .get(codex["id"].as_str().unwrap())
                .unwrap()
                .provider,
            ""
        );
    }
}
