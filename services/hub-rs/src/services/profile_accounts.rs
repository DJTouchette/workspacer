//! Account attribution reads only public identity fields, never credential values.
use super::profiles::{Profile, Profiles};
use crate::Options;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn read(path: &Path) -> Value {
    // A malformed or unreadable credential store is not a positive login.
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NONBLOCK);
    }
    let Ok(file) = open.open(path) else {
        return Value::Null;
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= 4 * 1024 * 1024)
    {
        return Value::Null;
    }
    let mut bytes = Vec::new();
    if file
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64().is_some_and(|v| v != 0.0),
        _ => true,
    }
}
fn text(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}
fn expand(value: &str, home: &Path) -> PathBuf {
    value
        .strip_prefix('~')
        .map(|tail| home.join(tail.trim_start_matches(['/', '\\'])))
        .unwrap_or_else(|| PathBuf::from(value))
}
// Account-root spelling follows Node path.resolve, not the selected-file
// confinement walker. Do not use this lexical helper for access decisions.
pub(crate) fn lexical_root(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut result = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => (),
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
pub(crate) fn claude_json_path(root: &Path, home: &Path) -> PathBuf {
    let root = lexical_root(root);
    if root == lexical_root(&home.join(".claude")) {
        home.join(".claude.json")
    } else {
        root.join(".claude.json")
    }
}
pub(crate) fn root(profile: &Profile, home: &Path) -> PathBuf {
    let (key, default) = match profile.provider.as_str() {
        "codex" => ("CODEX_HOME", ".codex"),
        "copilot" => ("COPILOT_HOME", ".copilot"),
        _ => ("CLAUDE_CONFIG_DIR", ".claude"),
    };
    if !profile.config_dir.trim().is_empty() {
        return expand(profile.config_dir.trim(), home);
    }
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| expand(v.trim(), home))
        .unwrap_or_else(|| home.join(default))
}
pub(crate) fn account(profile: &Profile, home: &Path) -> Value {
    let root = root(profile, home);
    let inspected = lexical_root(&root);
    let provider = if profile.provider.is_empty() {
        "claude"
    } else {
        &profile.provider
    };
    let mut result = json!({"provider":provider,"configRoot":root});
    match provider {
        "codex" => {
            let auth = read(&inspected.join("auth.json"));
            result["signedIn"] = json!(
                text(&auth["tokens"]["account_id"]).is_some()
                    || text(&auth["OPENAI_API_KEY"]).is_some()
                    || text(&auth["tokens"]["access_token"]).is_some()
            );
            if let Some(id) = text(&auth["tokens"]["account_id"]) {
                result["accountId"] = id.into();
            }
            if let Some(mode) = text(&auth["auth_mode"]) {
                result["authMode"] = mode.into();
            }
        }
        "copilot" => {
            if !profile.token_env_var.is_empty() {
                result["tokenEnvVar"] = profile.token_env_var.clone().into();
                result["signedIn"] = json!(
                    std::env::var(&profile.token_env_var).is_ok_and(|v| !v.trim().is_empty())
                );
            }
        }
        _ => {
            let local = read(&inspected.join(".claude.json"));
            result["signedIn"] = json!(
                profile.config_dir.trim().is_empty()
                    || inspected.join(".credentials.json").exists()
                    || truthy(&local["oauthAccount"])
            );
            let identity = read(&claude_json_path(&root, home));
            if let Some(id) = text(&identity["oauthAccount"]["accountUuid"])
                .or_else(|| text(&identity["oauthAccount"]["emailAddress"]))
            {
                result["accountId"] = id.into();
            }
        }
    }
    result
}
pub(crate) fn install(mut options: Options, directory: PathBuf, home: PathBuf) -> Options {
    let profiles = Arc::new(Profiles::new(directory));
    for method in [
        "desktop.claudeProfilesAccounts",
        "desktop.claudeProfilesLoginStatus",
    ] {
        let profiles = profiles.clone();
        let home = home.clone();
        options = options.handler(method, move |_, _| {
            let profiles = profiles.clone();
            let home = home.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    let values = profiles
                        .list()
                        .into_iter()
                        .map(|profile| {
                            let mut value = account(&profile, &home);
                            if method.ends_with("LoginStatus") {
                                value = json!(value["signedIn"] != false);
                            }
                            (profile.id, value)
                        })
                        .collect::<serde_json::Map<_, _>>();
                    anyhow::Ok(Value::Object(values))
                })
                .await?
            }
        });
    }
    let account_profiles = profiles.clone();
    options=options.handler("desktop.claudeProfilesAddAccount",move|_,params|{let profiles=account_profiles.clone();let home=home.clone();async move{
        tokio::task::spawn_blocking(move||{
            let name=params["name"].as_str().unwrap_or("").trim();let name=if name.is_empty(){"Account"}else{name};
            let setup=super::account_setup::create(name,&super::account_setup::primary(&home),&home)?;
            let profile=profiles.call("desktop.claudeProfilesAdd",json!({"name":name,"configDir":setup["dir"],"extraArgs":[],"mcpItemIds":[]}))?;
            anyhow::Ok(json!({"profile":profile,"shared":setup["shared"],"warnings":setup["warnings"]}))
        }).await?
    }});
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_identity_omits_all_secrets_and_unknown_copilot_stays_unknown() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("codex");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("auth.json"),r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"do-not-return","tokens":{"account_id":"account-1","access_token":"private"}}"#).unwrap();
        let profile = Profile {
            provider: "codex".into(),
            config_dir: root.to_string_lossy().into(),
            ..Profile::default()
        };
        let value = account(&profile, home.path());
        assert_eq!(value["signedIn"], true);
        assert_eq!(value["accountId"], "account-1");
        assert_eq!(value["authMode"], "chatgpt");
        assert!(!value.to_string().contains("private"));
        assert!(!value.to_string().contains("do-not-return"));
        let copilot = account(
            &Profile {
                provider: "copilot".into(),
                config_dir: root.to_string_lossy().into(),
                ..Profile::default()
            },
            home.path(),
        );
        assert!(copilot.get("signedIn").is_none());
    }
    #[test]
    fn default_root_dot_spelling_is_lexical_but_symlink_alias_keeps_own_identity() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"primary"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.path().join(".claude/.claude.json"),
            r#"{"oauthAccount":{"accountUuid":"alias"}}"#,
        )
        .unwrap();
        let dotted = Profile {
            config_dir: home
                .path()
                .join("absent/../.claude")
                .to_string_lossy()
                .into(),
            ..Profile::default()
        };
        assert_eq!(account(&dotted, home.path())["accountId"], "primary");
        #[cfg(unix)]
        {
            let alias = home.path().join("alias");
            std::os::unix::fs::symlink(home.path().join(".claude"), &alias).unwrap();
            let selected = Profile {
                config_dir: alias.to_string_lossy().into(),
                ..Profile::default()
            };
            assert_eq!(account(&selected, home.path())["accountId"], "alias");
        }
    }
    #[test]
    fn explicit_claude_account_uses_its_login_and_primary_identity_path() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".claude");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"primary-id","secret":"private"}}"#,
        )
        .unwrap();
        let profile = Profile {
            config_dir: root.to_string_lossy().into(),
            ..Profile::default()
        };
        let value = account(&profile, home.path());
        assert_eq!(value["signedIn"], false);
        assert_eq!(value["accountId"], "primary-id");
        std::fs::write(root.join(".credentials.json"), "{}").unwrap();
        assert_eq!(account(&profile, home.path())["signedIn"], true);
    }
}
