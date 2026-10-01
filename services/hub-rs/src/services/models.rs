use super::{config::Config, paths};
use crate::{
    Options,
    model_selection::{ModelSelection, claude_argv_model, normalize_model_selection, window_for},
};
use claudemon::daemon::embedded::Command;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

fn concrete(model: &str) -> Option<(String, Vec<u64>)> {
    let regex = regex::Regex::new(r"^claude-([a-z]+)-(\d+(?:-\d+)*?)(?:-\d{6,})?$").unwrap();
    let captures = regex.captures(model)?;
    Some((
        captures[1].into(),
        captures[2]
            .split('-')
            .map(|n| n.parse().unwrap_or(0))
            .collect(),
    ))
}
fn newer(a: &[u64], b: &[u64]) -> bool {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| !o.is_eq())
        .is_some_and(|o| o.is_gt())
}
pub fn catalog(config: &Value, live: &[String]) -> Value {
    let persisted = config["seenModels"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let seen: BTreeSet<_> = persisted
        .iter()
        .chain(live)
        .filter_map(|s| normalize_model_selection(s, None).ok().map(|s| s.model))
        .filter(|s| !s.starts_with('<'))
        .collect();
    let mut newest = BTreeMap::<String, Vec<u64>>::new();
    for id in &seen {
        if let Some((family, version)) = concrete(id)
            && newest
                .get(&family)
                .is_none_or(|current| newer(&version, current))
        {
            newest.insert(family, version);
        }
    }
    let seen: Vec<_> = seen
        .into_iter()
        .filter(|id| {
            concrete(id).is_none_or(|(family, version)| newest.get(&family) != Some(&version))
        })
        .collect();
    let mut aliases = Vec::new();
    for (legacy, family, label) in [
        ("fable", "fable", "Fable"),
        ("opus", "opus", "Opus"),
        ("opus[1m]", "opus", "Opus"),
        ("sonnet", "sonnet", "Sonnet"),
        ("sonnet[1m]", "sonnet", "Sonnet"),
        ("haiku", "haiku", "Haiku"),
    ] {
        let selection = normalize_model_selection(legacy, None).unwrap();
        let window = selection
            .context_window
            .or_else(|| window_for(&selection.model))
            .or_else(|| window_for(&format!("claude-{}", selection.model)))
            .expect("builtin model window");
        let label = newest
            .get(family)
            .map(|v| {
                format!(
                    "{label} {}",
                    v.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
                )
            })
            .unwrap_or(label.into());
        let value = claude_argv_model(&ModelSelection {
            model: selection.model.clone(),
            context_window: Some(window),
        })
        .unwrap();
        aliases.push(json!({"model":selection.model,"value":value,"label":label,"contextWindow":window,"context":if window>=1_000_000{format!("{}M",window/1_000_000)}else{format!("{}K",window/1000)}}));
    }
    let selection = normalize_model_selection(
        config["defaultModel"].as_str().unwrap_or(""),
        config["contextWindow"].as_u64(),
    )
    .ok();
    json!({"defaultModel":selection.as_ref().map(|s|s.model.as_str()).unwrap_or(""),"contextWindow":selection.as_ref().and_then(|s|s.context_window),
        "skipPermissionsDefault":config["skipPermissionsDefault"].as_bool().unwrap_or(false),"defaultPermissionMode":config["defaultPermissionMode"].as_str().unwrap_or(""),"aliases":aliases,"seen":seen})
}
fn find_binary_on_path(
    provider: &str,
    search_path: &std::ffi::OsStr,
    windows: bool,
) -> Option<PathBuf> {
    for directory in std::env::split_paths(search_path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let names = if windows {
            vec![
                format!("{provider}.cmd"),
                format!("{provider}.exe"),
                provider.into(),
            ]
        } else {
            vec![provider.into()]
        };
        for name in names {
            let path = directory.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
fn find_binary(provider: &str) -> Option<PathBuf> {
    find_binary_on_path(
        provider,
        &std::env::var_os("PATH").unwrap_or_default(),
        cfg!(windows),
    )
}
pub fn resolve_binary(provider: &str, config: &Value) -> String {
    let custom = config["agents"]["binaries"][provider]
        .as_str()
        .unwrap_or("")
        .trim();
    if !custom.is_empty() {
        return custom.into();
    }
    find_binary(provider)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(provider.into())
}
/// The legacy Claude escape hatch affects spawning, not provider detection.
pub fn resolve_spawn_binary(provider: &str, config: &Value) -> String {
    let custom = config["agents"]["binaries"][provider]
        .as_str()
        .unwrap_or("")
        .trim();
    if custom.is_empty()
        && provider == "claude"
        && let Ok(binary) = std::env::var("WKS_CLAUDE_BIN")
        && !binary.trim().is_empty()
    {
        return binary.trim().into();
    }
    resolve_binary(provider, config)
}

fn provider_request(
    params: &Value,
    config: &Value,
    home: Option<&Path>,
) -> anyhow::Result<(String, Command)> {
    let provider = params["provider"].as_str().unwrap_or("");
    if !["codex", "copilot", "opencode", "pi"].contains(&provider) {
        anyhow::bail!(
            "providers.listModels requires {{ provider: 'codex'|'copilot'|'opencode'|'pi' }}"
        );
    }
    let requested = params["cwd"].as_str().unwrap_or("");
    // The initial Codex picker has no project yet. Resolve this explicit
    // discovery request on the hub, never against the client's filesystem.
    let cwd = if provider == "codex"
        && requested.is_empty()
        && params["useHomeDirectory"].as_bool() == Some(true)
    {
        paths::canonicalize(home.ok_or_else(|| anyhow::anyhow!("Hub home directory unavailable"))?)?
    } else {
        paths::canonicalize(Path::new(requested))?
    };
    let binary = resolve_binary(provider, config);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cwd", &cwd.to_string_lossy())
        .append_pair("bin", &binary)
        .finish();
    Ok((
        provider.into(),
        Command::Request {
            method: "GET".into(),
            path: format!("/providers/{provider}/models?{query}"),
            payload: None,
        },
    ))
}

fn provider_rows(value: Value) -> Option<Vec<Value>> {
    // Serde also permits tuple-like arrays for structs; Go's JSON object
    // decoder does not. Pin object/null shape before typed deserialization.
    if !value.is_null() && !value.is_object() {
        return None;
    }
    if let Some(rows) = value["models"].as_array()
        && rows.iter().any(|row| !row.is_null() && !row.is_object())
    {
        return None;
    }
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Model {
        id: Option<String>,
        label: Option<String>,
        default: Option<bool>,
    }
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Response {
        models: Option<Vec<Option<Model>>>,
    }
    // Go's typed decoder soft-fails the entire response on a wrong field type.
    // Missing/null fields retain zero values, including null model rows.
    let response = serde_json::from_value::<Option<Response>>(value)
        .ok()?
        .unwrap_or_default();
    Some(response.models.unwrap_or_default().into_iter().map(|row| {
        let row = row.unwrap_or_default();
        json!({"id":row.id.unwrap_or_default(),"label":row.label.unwrap_or_default(),"default":row.default.unwrap_or_default()})
    }).collect())
}
pub fn check_all(config: &Value) -> Value {
    Value::Array(["claude","codex","copilot","opencode","pi"].iter().map(|provider|{
        let custom=config["agents"]["binaries"][provider].as_str().unwrap_or("").trim();
        let resolved=if custom.is_empty(){find_binary(provider)}else{Path::new(custom).is_file().then(||PathBuf::from(custom))};
        json!({"provider":provider,"found":resolved.is_some(),"resolvedPath":resolved,"customBin":custom})
    }).collect())
}
pub(crate) fn install(mut options: Options, config: Arc<Config>) -> Options {
    let engine = options.engine.clone();
    let cfg = config.clone();
    let routing = options.routing.clone();
    options = options.handler("claude.listModels", move |_, _| {
        let engine = engine.clone();
        let cfg = cfg.clone();
        let routing = routing.clone();
        async move {
            let current = cfg.get();
            let mut live = Vec::new();
            if let Some(engine) = engine
                && let Ok(rows) = engine.request(Command::Sessions).await
                && let Some(rows) = rows.as_array()
            {
                for row in rows {
                    if let Some(model) = row["usage"]["model"].as_str() {
                        live.push(model.into());
                    }
                }
            }
            let result = catalog(&current["claude"], &live);
            if let Some(routing) = routing {
                let mut models: Vec<_> = result["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| json!({"id":a["value"],"label":a["label"]}))
                    .collect();
                models.extend(
                    result["seen"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|id| json!({"id":id})),
                );
                routing.update_catalog("claude", Some(models));
            }
            Ok(result)
        }
    });
    let engine = options.engine.clone();
    let cfg = config.clone();
    let routing = options.routing.clone();
    let home = options
        .home_dir
        .clone()
        .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()));
    options = options.handler("providers.listModels", move |_, params| {
        let engine = engine.clone();
        let cfg = cfg.clone();
        let routing = routing.clone();
        let home = home.clone();
        async move {
            let (provider, request) = provider_request(&params, &cfg.get(), home.as_deref())?;
            let Some(engine) = engine else {
                if let Some(routing) = routing {
                    routing.update_catalog(&provider, None);
                }
                anyhow::bail!(
                    "Provider model discovery is unavailable: no execution engine is connected"
                );
            };
            let response = engine.request(request).await;
            let rows = response.as_ref().ok().cloned().and_then(provider_rows);
            if let Some(routing) = routing {
                // Keep upstream metadata used by routing, but never cache a
                // response that failed the public typed-model contract.
                routing.update_catalog(
                    &provider,
                    rows.as_ref()
                        .and_then(|_| response.as_ref().ok()?.get("models")?.as_array().cloned()),
                );
            }
            response
                .map_err(|error| anyhow::anyhow!("Could not load {provider} models: {error}"))?;
            let rows = rows
                .ok_or_else(|| anyhow::anyhow!("{provider} returned an invalid model catalog"))?;
            Ok(Value::Array(rows))
        }
    });
    options.handler("providers.checkAll", move |_, _| {
        let cfg = config.clone();
        async move { tokio::task::spawn_blocking(move || Ok(check_all(&cfg.get()))).await? }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_codex_catalog_uses_hub_home_only_when_explicitly_requested() {
        let home = tempfile::tempdir().unwrap();
        let params = json!({"provider":"codex", "cwd":"", "useHomeDirectory":true});
        let (_, command) = provider_request(&params, &json!({}), Some(home.path())).unwrap();
        let Command::Request { path, .. } = command else {
            panic!("wrong engine command")
        };
        let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query["cwd"],
            paths::canonicalize(home.path()).unwrap().to_string_lossy()
        );
        assert!(provider_request(&params, &json!({}), None).is_err());
        for params in [
            json!({"provider":"codex", "cwd":""}),
            json!({"provider":"codex", "cwd":"relative", "useHomeDirectory":true}),
            json!({"provider":"copilot", "cwd":"", "useHomeDirectory":true}),
        ] {
            assert!(provider_request(&params, &json!({}), Some(home.path())).is_err());
        }
        let project = home.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let (_, command) = provider_request(
            &json!({"provider":"codex", "cwd":project, "useHomeDirectory":true}),
            &json!({}),
            Some(home.path()),
        )
        .unwrap();
        let Command::Request { path, .. } = command else {
            panic!("wrong engine command")
        };
        let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query["cwd"],
            paths::canonicalize(&project).unwrap().to_string_lossy()
        );
    }

    #[test]
    fn binary_search_skips_directories_and_preserves_platform_candidate_order() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        std::fs::create_dir_all(first.join("codex")).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let binary = second.join("codex");
        std::fs::write(&binary, "fixture").unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(find_binary_on_path("codex", &path, false), Some(binary));
        std::fs::write(first.join("codex.exe"), "fixture").unwrap();
        std::fs::write(first.join("codex.cmd"), "fixture").unwrap();
        assert_eq!(
            find_binary_on_path("codex", &path, true),
            Some(first.join("codex.cmd"))
        );
        assert_eq!(find_binary_on_path("absent", &path, false), None);
    }
    #[test]
    fn provider_relay_preserves_typed_response_defaults_and_rejects_invalid_shapes() {
        assert_eq!(provider_rows(json!({"models":[{"id":"gpt-x","label":"GPT X","default":true},{"id":"gpt-y","label":"GPT Y"}]})).unwrap(), vec![json!({"id":"gpt-x","label":"GPT X","default":true}),json!({"id":"gpt-y","label":"GPT Y","default":false})]);
        for value in [Value::Null, json!({}), json!({"models":null})] {
            assert_eq!(provider_rows(value), Some(vec![]));
        }
        assert_eq!(
            provider_rows(json!({"models":[null,{"id":null,"label":null,"default":null}]}))
                .unwrap(),
            vec![json!({"id":"","label":"","default":false}); 2]
        );
        for invalid in [
            json!([]),
            json!({"models":{}}),
            json!({"models":[{"id":1}]}),
            json!({"models":[{"label":false}]}),
            json!({"models":[{"default":"true"}]}),
            json!({"models":["bad"]}),
            json!({"models":[[]]}),
            json!({"models":[["id","label",true]]}),
        ] {
            assert!(provider_rows(invalid.clone()).is_none(), "{invalid}");
        }
    }
    #[test]
    fn provider_query_encodes_canonical_cwd_and_custom_binary_after_admission() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("spaces & query");
        std::fs::create_dir(&cwd).unwrap();
        for provider in ["codex", "copilot", "opencode", "pi"] {
            let config = json!({"agents":{"binaries":{provider:" custom & binary "}}});
            let (_, command) =
                provider_request(&json!({"provider":provider,"cwd":cwd}), &config, None).unwrap();
            let Command::Request {
                method,
                path,
                payload,
            } = command
            else {
                panic!("wrong engine command")
            };
            assert_eq!(method, "GET");
            assert!(payload.is_none());
            let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
            assert_eq!(url.path(), format!("/providers/{provider}/models"));
            let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(
                query["cwd"],
                paths::canonicalize(&cwd).unwrap().to_string_lossy()
            );
            assert_eq!(query["bin"], "custom & binary");
        }
        for params in [
            json!({"provider":"claude","cwd":cwd}),
            json!({"provider":"unknown","cwd":cwd}),
            json!({"provider":"codex"}),
            json!({"provider":"codex","cwd":"relative"}),
        ] {
            assert!(provider_request(&params, &json!({}), None).is_err());
        }
    }
}
