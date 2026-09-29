//! Readiness-only Codex contract pinned to the source-verified 0.153.4 CLI.
//! No auth files, threads, package installation, or provider fallback.
use super::{
    completion::{self, Outcome},
    text::{self, Failure},
};
use crate::services::owned_process;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;
const VERSION: &str = "codex-cli 0.153.4";
const AUTH: &[&str] = &[
    "auth_elicitation",
    "secret_auth_storage",
    "respect_system_proxy",
    "network_proxy",
    "psp",
    "use_agent_identity",
];
const DISABLED: &[&str] = &[
    "shell_tool",
    "view_image",
    "plugins",
    "remote_plugin",
    "apps",
    "hooks",
    "plugin_hooks",
    "remote_models",
    "memories",
    "external_agent_memory_import",
    "external_migration",
    "js_repl",
    "js_repl_tools_only",
    "image_generation",
    "browser_use",
    "computer_use",
    "deferred_executor",
    "token_budget",
    "current_time_reminder",
    "sleep_tool",
    "tool_suggest",
    "code_mode",
    "code_mode_only",
    "multi_agent",
    "multi_agent_v2",
    "search_tool",
    "tool_search",
    "request_permissions_tool",
    "standalone_web_search",
    "sqlite",
    "unbounded_connection_retries",
    "shell_snapshot",
    "shell_snapshot_v2",
];
fn overrides(values: &BTreeMap<String, Value>) -> Vec<String> {
    values
        .iter()
        .flat_map(|(key, value)| ["-c".into(), format!("{key}={value}")])
        .collect()
}
#[derive(Clone, Debug)]
struct Metadata {
    store: String,
    auth: BTreeMap<String, Value>,
    model: String,
    effort: String,
}
fn config(value: &Value) -> Outcome<Metadata> {
    if !value.is_object()
        || value
            .get("model_provider")
            .is_some_and(|v| !v.is_null() && v != "" && v != "openai")
        || truthy(&value["model_providers"]["openai"])
        || value["features"].as_object().is_some_and(|features| {
            features.keys().any(|key| {
                ["auth", "proxy", "psp", "identity"]
                    .iter()
                    .any(|part| key.contains(part))
                    && !AUTH.contains(&key.as_str())
            })
        })
        || value.get("chatgpt_base_url").is_some_and(|v| {
            truthy(v)
                && v != "https://chatgpt.com/backend-api/"
                && v != "https://chatgpt.com/backend-api"
        })
        || [
            "forced_login_method",
            "forced_chatgpt_workspace_id",
            "model_provider_auth",
        ]
        .iter()
        .any(|key| truthy(&value[key]))
    {
        return Err(Failure::Unsupported);
    }
    let mut auth = BTreeMap::new();
    for name in AUTH {
        let value = &value["features"][name];
        if value.is_null() {
            continue;
        }
        let boolean = value.as_bool().ok_or(Failure::Unsupported)?;
        auth.insert(format!("features.{name}"), json!(boolean));
    }
    let store = match value.get("cli_auth_credentials_store") {
        None | Some(Value::Null) => "file",
        Some(value) => value.as_str().ok_or(Failure::Unsupported)?,
    };
    if !["file", "keyring", "auto"].contains(&store) {
        return Err(Failure::Unsupported);
    }
    Ok(Metadata {
        store: store.into(),
        auth,
        model: String::new(),
        effort: String::new(),
    })
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        _ => true,
    }
}
fn model(value: &Value) -> Outcome<(String, String)> {
    if truthy(&value["nextCursor"]) {
        return Err(Failure::Unsupported);
    }
    let rows = value["data"].as_array().ok_or(Failure::Unsupported)?;
    let syntax = regex::Regex::new(r"^[a-zA-Z0-9._-]{1,96}$").unwrap();
    let cheap = regex::Regex::new(r"(?:mini|nano|luna)(?:$|[-.])").unwrap();
    let row = rows
        .iter()
        .find(|row| {
            row["hidden"] != true
                && row["model"].as_str().is_some_and(|model| {
                    ["gpt-", "codex-"]
                        .iter()
                        .any(|prefix| model.starts_with(prefix))
                        || model.as_bytes().get(0) == Some(&b'o')
                            && model.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
                })
                && row["model"]
                    .as_str()
                    .is_some_and(|m| syntax.is_match(m) && cheap.is_match(m))
        })
        .ok_or(Failure::Unsupported)?;
    let effort = ["none", "minimal", "low"]
        .into_iter()
        .find(|effort| {
            row["supportedReasoningEfforts"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|row| row["reasoningEffort"] == *effort))
        })
        .ok_or(Failure::Unsupported)?;
    Ok((row["model"].as_str().unwrap().into(), effort.into()))
}
fn read_json(path: &Path, limit: u64) -> Option<Value> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
fn resolve(configured: &Path, home: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(configured).ok()?;
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > 8192 {
        return Some(configured.into());
    }
    let bytes = std::fs::read(configured).ok()?;
    if !bytes.starts_with(b"#!") {
        return Some(configured.into());
    }
    if format!("{:x}", Sha256::digest(&bytes))
        != "0f769462bfa40e84c92f1c4481b268ecaca740dda521338f7255406496aaf3e6"
    {
        return None;
    }
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => ("darwin-x64", "x86_64-apple-darwin"),
        ("macos", "aarch64") => ("darwin-arm64", "aarch64-apple-darwin"),
        _ => return None,
    };
    let cache = std::env::var_os("npm_config_cache")
        .map(PathBuf::from)
        .unwrap_or(home.join(".npm"))
        .join("_npx");
    let entries = std::fs::read_dir(cache)
        .ok()?
        .take(257)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if entries.len() > 256 {
        return None;
    }
    let mut matches = BTreeSet::new();
    for entry in entries {
        let root = entry.path().join("node_modules/@openai");
        let Some(package) = read_json(&root.join("codex/package.json"), 128 * 1024) else {
            continue;
        };
        if package["name"] != "@openai/codex" || package["version"] != "0.153.4" {
            continue;
        }
        let path = root.join(format!(
            "codex-{}/vendor/{}/bin/codex",
            platform.0, platform.1
        ));
        if path.is_file() {
            if let Ok(path) = path.canonicalize() {
                matches.insert(path);
            }
        }
    }
    if matches.len() == 1 {
        matches.pop_first()
    } else {
        None
    }
}
async fn metadata(binary: &Path) -> Outcome<Metadata> {
    let values = [
        "features.plugins",
        "features.remote_plugin",
        "features.hooks",
        "features.plugin_hooks",
        "features.apps",
        "features.external_migration",
        "features.memories",
        "features.remote_control",
        "skills.bundled.enabled",
        "analytics.enabled",
        "feedback.enabled",
        "features.sqlite",
    ]
    .into_iter()
    .map(|key| (key.into(), json!(false)))
    .collect();
    let mut command = Command::new(binary);
    command
        .arg("app-server")
        .args(overrides(&values))
        .current_dir(std::env::temp_dir());
    let mut state: Option<Metadata> = None;
    let initial = [
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"workspacer-readiness","version":"1"},"capabilities":{"experimentalApi":true}}}),
    ];
    let result=owned_process::json_exchange(&mut command,&initial,move|row|{
        anyhow::ensure!(row.get("error").is_none_or(Value::is_null),"metadata unavailable");
        match row["id"].as_u64(){
            Some(1)=>Ok((vec![json!({"method":"initialized"}),json!({"id":2,"method":"config/read","params":{"includeLayers":false}})],None)),
            Some(2)=>{state=Some(config(&row["result"]["config"]).map_err(|_|anyhow::anyhow!("metadata refused"))?);Ok((vec![json!({"id":3,"method":"configRequirements/read","params":{}})],None))},
            Some(3)=>{anyhow::ensure!(row["result"].is_object()&&row["result"].get("requirements").is_some_and(Value::is_null),"managed policy");Ok((vec![json!({"id":4,"method":"model/list","params":{"includeHidden":false,"limit":100}})],None))},
            Some(4)=>{let (model,effort)=model(&row["result"]).map_err(|_|anyhow::anyhow!("metadata model unavailable"))?;let state=state.as_ref().ok_or_else(||anyhow::anyhow!("metadata order"))?;Ok((vec![],Some(json!({"store":state.store,"auth":state.auth,"model":model,"effort":effort}))))},
            _=>Ok((vec![],None)),
        }
    },128*1024,Duration::from_secs(4)).await.map_err(|error|if error.to_string().contains("timed out"){Failure::Timeout}else{Failure::Unsupported})?;
    Ok(Metadata {
        store: result["store"].as_str().unwrap().into(),
        auth: serde_json::from_value(result["auth"].clone()).map_err(|_| Failure::Unsupported)?,
        model: result["model"].as_str().unwrap().into(),
        effort: result["effort"].as_str().unwrap().into(),
    })
}
fn catalog(metadata: &Metadata, transport: &Value) -> Value {
    json!({"models":[{"slug":metadata.model,"display_name":metadata.model,"description":null,"supported_reasoning_levels":[{"effort":metadata.effort,"description":"Readiness effort"}],"default_reasoning_level":metadata.effort,"shell_type":"disabled","visibility":"list","supported_in_api":true,"priority":0,"support_verbosity":false,"truncation_policy":{"mode":"tokens","limit":1000},"experimental_supported_tools":[],"apply_patch_tool_type":null,"model_messages":{"instructions_template":"Reply OK.","persistent_instructions":""},"include_skills_usage_instructions":false,"include_apps_usage_instructions":false,"include_plugin_usage_instructions":false,"supports_search_tool":false,"tool_mode":"direct","multi_agent_version":"disabled","node_repl_disabled":true,"use_responses_lite":transport["use_responses_lite"].as_bool().unwrap_or(false),"supports_reasoning_summary_parameter":transport["supports_reasoning_summary_parameter"].as_bool().unwrap_or(true)}]})
}
fn args(metadata: &Metadata, catalog: &Path) -> Vec<String> {
    let mut values: BTreeMap<String,Value>=serde_json::from_value(json!({"skills.include_instructions":false,"skills.bundled.enabled":false,"tools.update_plan.enabled":false,"tools.experimental_request_user_input.enabled":false,"agents.enabled":false,"project_doc_max_bytes":0,"include_environment_context":false,"include_permissions_instructions":false,"include_apps_instructions":false,"include_collaboration_mode_instructions":false,"developer_instructions":"","web_search":"disabled","analytics.enabled":false,"feedback.enabled":false,"model_reasoning_effort":metadata.effort,"model_reasoning_summary":"none","notify":[],"model_catalog_json":catalog,"cli_auth_credentials_store":metadata.store,"model_provider":"workspacer_readiness","model_providers.workspacer_readiness.name":"OpenAI","model_providers.workspacer_readiness.wire_api":"responses","model_providers.workspacer_readiness.http_headers.version":"0.153.4","model_providers.workspacer_readiness.requires_openai_auth":true,"model_providers.workspacer_readiness.env_http_headers.OpenAI-Organization":"OPENAI_ORGANIZATION","model_providers.workspacer_readiness.env_http_headers.OpenAI-Project":"OPENAI_PROJECT","model_providers.workspacer_readiness.request_max_retries":0,"model_providers.workspacer_readiness.stream_max_retries":0})).unwrap();
    values.extend(
        DISABLED
            .iter()
            .map(|key| (format!("features.{key}"), json!(false))),
    );
    values.extend(metadata.auth.clone());
    let mut args = [
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--skip-git-repo-check",
        "--json",
        "--model",
        &metadata.model,
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(overrides(&values));
    args.push("-".into());
    args
}
fn answer(output: &str) -> Outcome<()> {
    let mut answer = String::new();
    let mut started = false;
    let mut completed = false;
    for line in output.trim().lines() {
        let row: Value = serde_json::from_str(line).map_err(|_| Failure::Error)?;
        if row["type"] == "error" || row["type"] == "turn.failed" {
            return Err(text::classify(line));
        }
        if row["type"] == "turn.started" {
            started = true;
        }
        if row["type"] == "item.completed" {
            if row["item"]["type"] == "error" {
                let message = row["item"]["message"].as_str().unwrap_or("");
                if !started
                    && (message.starts_with("Under-development features enabled:")
                        || regex::Regex::new(r"^`\[features\]\.[a-z_]+` is deprecated\.")
                            .unwrap()
                            .is_match(message))
                {
                    continue;
                }
                return Err(text::classify(message));
            }
            if row["item"]["type"] == "agent_message" {
                answer = row["item"]["text"].as_str().unwrap_or("").into();
            } else if row["item"]["type"] != "reasoning" {
                return Err(Failure::Error);
            }
        }
        if row["type"] == "turn.completed" {
            completed = true;
        }
    }
    if completed && answer.trim() == "OK" {
        Ok(())
    } else {
        Err(Failure::Empty)
    }
}
pub async fn ping(binary: &Path, home: &Path) -> Outcome<()> {
    if cfg!(windows)
        || std::env::var_os("OPENAI_BASE_URL").is_some_and(|v| !v.is_empty())
        || std::env::var_os("CODEX_HOME")
            .is_some_and(|v| !v.is_empty() && !Path::new(&v).is_absolute())
    {
        return Err(Failure::Unsupported);
    }
    let binary = resolve(binary, home).ok_or(Failure::Unsupported)?;
    if !completion::native(&binary) {
        return Err(Failure::Unsupported);
    }
    let version = completion::run(
        Command::new(&binary)
            .arg("--version")
            .current_dir(std::env::temp_dir()),
        "",
        256,
        Duration::from_secs(2),
    )
    .await?;
    if version.trim() != VERSION {
        return Err(Failure::Unsupported);
    }
    let metadata = metadata(&binary).await?;
    let root = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or(home.join(".codex"));
    let cache =
        read_json(&root.join("models_cache.json"), 4_000_000).ok_or(Failure::Unsupported)?;
    if cache["client_version"] != "0.153.4" {
        return Err(Failure::Unsupported);
    }
    let transport = cache["models"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["slug"] == metadata.model))
        .ok_or(Failure::Unsupported)?;
    if ["use_responses_lite", "supports_reasoning_summary_parameter"]
        .iter()
        .any(|key| {
            transport
                .get(key)
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
        })
    {
        return Err(Failure::Unsupported);
    }
    let scratch = tempfile::Builder::new()
        .prefix("workspacer-codex-ping-")
        .tempdir()
        .map_err(|_| Failure::Error)?;
    let path = scratch.path().join("models.json");
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .and_then(|mut file| {
                file.write_all(&serde_json::to_vec(&catalog(&metadata, transport)).unwrap())
            })
            .map_err(|_| Failure::Error)?;
    }
    let output = completion::run(
        Command::new(binary)
            .args(args(&metadata, &path))
            .current_dir(scratch.path()),
        "Reply OK.",
        8000,
        Duration::from_secs(12),
    )
    .await?;
    answer(&output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auth_metadata_never_silently_changes_account_route() {
        for cfg in [
            json!({"model_provider":"custom"}),
            json!({"model_providers":{"openai":{}}}),
            json!({"features":{"unknown_auth":true}}),
            json!({"features":{"auth_elicitation":"yes"}}),
            json!({"chatgpt_base_url":"https://other.example"}),
            json!({"forced_login_method":"chatgpt"}),
            json!({"cli_auth_credentials_store":"other"}),
        ] {
            assert!(config(&cfg).is_err(), "{cfg}");
        }
        let metadata = config(
            &json!({"cli_auth_credentials_store":"keyring","features":{"auth_elicitation":true}}),
        )
        .unwrap();
        assert_eq!(metadata.store, "keyring");
        assert_eq!(metadata.auth["features.auth_elicitation"], true);
    }
    #[test]
    fn readiness_selects_only_advertised_cheap_effort_and_pins_empty_tools() {
        let rows = json!({"data":[{"model":"gpt-expensive","supportedReasoningEfforts":[{"reasoningEffort":"low"}]},{"model":"gpt-5.4-mini","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"none"}]}]});
        assert_eq!(
            model(&rows).unwrap(),
            ("gpt-5.4-mini".into(), "none".into())
        );
        assert!(model(&json!({"data":rows["data"],"nextCursor":"next"})).is_err());
        let metadata = Metadata {
            store: "file".into(),
            auth: BTreeMap::new(),
            model: "gpt-5.4-mini".into(),
            effort: "none".into(),
        };
        let cat = catalog(
            &metadata,
            &json!({"use_responses_lite":true,"supports_reasoning_summary_parameter":false}),
        );
        assert_eq!(cat["models"][0]["experimental_supported_tools"], json!([]));
        assert_eq!(cat["models"][0]["use_responses_lite"], true);
        let args = args(&metadata, Path::new("/tmp/public-models.json"));
        assert!(args.iter().any(|v| v == "features.shell_tool=false"));
        assert!(
            args.iter()
                .any(|v| v == "model_providers.workspacer_readiness.request_max_retries=0")
        );
        assert!(!args.iter().any(|v| v.contains("Reply OK")));
    }
    #[test]
    fn ping_needs_completed_exact_answer_and_rejects_tool_items() {
        let good = "{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"OK\"}}\n{\"type\":\"turn.completed\"}";
        assert_eq!(answer(good), Ok(()));
        assert_eq!(
            answer(&good.replace("agent_message", "command_execution")),
            Err(Failure::Error)
        );
        assert_eq!(answer(&good.replace("OK", "Maybe")), Err(Failure::Empty));
        assert_eq!(
            answer("{\"type\":\"turn.failed\",\"message\":\"401 Unauthorized\"}"),
            Err(Failure::Authentication)
        );
    }
}
