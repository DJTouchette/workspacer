//! Actual installed handler inventory. Advertised authorization vocabulary and
//! MCP tool names are deliberately not treated as implementation evidence.
use crate::{Options, backend::Backend};
use anyhow::{Context, Result};
use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub fn compare(health: &Value) -> Result<Value> {
    let actual: BTreeSet<String> = health["methodNames"]
        .as_array()
        .context("health lacks installed methodNames")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .context("non-text installed method")
        })
        .collect::<Result<_>>()?;
    let reference: Value =
        serde_json::from_str(include_str!("../../assets/brain-capabilities.json"))?;
    let mut report = json!({"source":reference["source"],"installed":actual,"launchReady":health["launchReady"],"mcpReady":health["mcpReady"],"behavioralParityProven":false});
    for scope in ["full", "catalog", "hub"] {
        let expected: BTreeSet<String> = reference[scope]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        report[scope] = json!({"expected":expected.len(),"present":expected.intersection(&actual).count(),"missing":expected.difference(&actual).collect::<Vec<_>>()});
    }
    Ok(report)
}
/// Start a temporary complete local graph with empty stores and no selected
/// plugins, jobs, peers or provider launches. Never adopts the user's backend.
pub async fn probe() -> Result<Value> {
    let root = tempfile::tempdir()?;
    let config = root.path().join("config");
    let home = root.path().join("home");
    let data = root.path().join("data");
    for path in [&config, &home, &data] {
        std::fs::create_dir_all(path)?;
    }
    std::fs::write(
        config.join("config.yaml"),
        "agents:\n  checkProviderOnStartup: false\n",
    )?;
    let mut options = Options::default();
    options.home_dir = Some(home);
    options.config_dir = Some(config.clone());
    options.data_dir = Some(data);
    options.scoped_tokens = Some(config.join("tokens.json"));
    options.plugins_dir = Some(config.join("plugins"));
    options.token = uuid::Uuid::new_v4().to_string();
    options.listen = Some("127.0.0.1:0".parse()?);
    options.mcp_listen = Some("127.0.0.1:0".parse()?);
    let mut backend = Backend::prepare(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("sessions.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )?;
    let run = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        backend.initialize(options).await?;
        let health = backend.handle().health().await?;
        compare(&health)
    })
    .await;
    let cleanup = backend.shutdown().await;
    let report = run.context("capability inventory readiness timed out")??;
    cleanup?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn complete_graph_inventory_reports_actual_handlers_without_catalog_inference() {
        let _engine_guard = crate::backend::ENGINE_TEST_LOCK.lock().await;
        let report = probe().await.unwrap();
        assert_eq!(report["launchReady"], true);
        assert_eq!(report["mcpReady"], true);
        let installed = report["installed"].as_array().unwrap();
        assert!(installed.contains(&json!("agents.spawn")));
        assert!(installed.contains(&json!("desktop.managerRequestSend")));
        let empty =
            compare(&json!({"methodNames":[],"launchReady":false,"mcpReady":false})).unwrap();
        assert_eq!(empty["full"]["present"], 0);
        assert!(
            empty["full"]["missing"]
                .as_array()
                .unwrap()
                .contains(&json!("claude.setModel"))
        );
        eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}
