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
fn find_binary(provider: &str) -> Option<PathBuf> {
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let names = if cfg!(windows) {
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
    options=options.handler("providers.listModels",move |_,params|{let engine=engine.clone();let cfg=cfg.clone();let routing=routing.clone();async move{
        let provider=params["provider"].as_str().unwrap_or("");if !["codex","copilot","opencode","pi"].contains(&provider){anyhow::bail!("providers.listModels requires {{ provider: 'codex'|'copilot'|'opencode'|'pi' }}");}
        let cwd=paths::canonicalize(Path::new(params["cwd"].as_str().unwrap_or("")))?;
        let Some(engine)=engine else{if let Some(routing)=routing{routing.update_catalog(provider,None);}return Ok(json!([]));};let binary=resolve_binary(provider,&cfg.get());
        let query=url::form_urlencoded::Serializer::new(String::new()).append_pair("cwd",&cwd.to_string_lossy()).append_pair("bin",&binary).finish();
        let rows=engine.request(Command::Request {method:"GET".into(),path:format!("/providers/{provider}/models?{query}"),payload:None}).await.ok();
        if let Some(routing)=routing {routing.update_catalog(provider,rows.as_ref().and_then(|v|v["models"].as_array()).cloned());}
        Ok(Value::Array(rows.as_ref().and_then(|v|v["models"].as_array()).map(|models|models.iter().map(|m|json!({"id":m["id"].as_str().unwrap_or(""),"label":m["label"].as_str().unwrap_or(""),"default":m["default"].as_bool().unwrap_or(false)})).collect()).unwrap_or_default()))
    }});
    options.handler("providers.checkAll", move |_, _| {
        let cfg = config.clone();
        async move { tokio::task::spawn_blocking(move || Ok(check_all(&cfg.get()))).await? }
    })
}
