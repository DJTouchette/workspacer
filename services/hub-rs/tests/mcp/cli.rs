//! The former standalone facade entry point is now an owned serve component.
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncBufReadExt;
use workspacer_hub::auth::{self, Scope};
#[tokio::test]
async fn owned_mcp_cli_resolves_flags_and_environment_then_joins_its_listeners() {
    for environment in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let config = root.path().join("config");
        std::fs::create_dir(&config).unwrap();
        let tokens = config.join("tokens.json");
        let view = auth::mint(&tokens, Scope::View, "facade-cli-view").unwrap();
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_workspacer-rust"));
        command.env_clear();
        for key in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .args([
                "serve",
                "--hub-only",
                "--no-jobs",
                "--plugins-dir",
                "",
                "--json",
                "--hub-port",
                "0",
                "--mcp-port",
                "0",
                "--token",
                "facade-cli-owner",
                "--tokens-file",
            ])
            .arg(&tokens)
            .arg("--config-dir")
            .arg(&config)
            .arg("--home-dir")
            .arg(&home)
            .arg("--data-dir")
            .arg(root.path().join("data"));
        if environment {
            command
                .env("WKS_MCP_TOKEN", "facade-cli-static")
                .env("WKS_MCP_UNTOKENED", "operator");
        } else {
            command
                .args([
                    "--mcp-token",
                    "facade-cli-static",
                    "--untokened",
                    "operator",
                ])
                .env("WKS_MCP_TOKEN", "ignored-environment-static")
                .env("WKS_MCP_UNTOKENED", "view");
        }
        let mut child = command
            .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", root.path().join("appdata"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg"))
            .env_remove("HUB_TOKEN")
            .env_remove("WORKSPACER_ALLOW_NEW_TOKEN")
            .env_remove("WKS_NODE_ID")
            .env_remove("WKS_MCP_HUB_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let line = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("owned facade readiness");
        let banner: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(banner["mode"], "hub-only");
        assert!(banner["database"].is_null());
        let mcp = banner["mcpUrl"].as_str().unwrap();
        let health_url = mcp.trim_end_matches("/mcp").to_owned() + "/health";
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let health: Value = http
            .get(&health_url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["service"], "workspacer-mcp-facade");
        assert_eq!(health["hubConnected"], true);
        assert_eq!(health["pluginCatalogReady"], true);
        assert_eq!(health["hubUrl"], banner["busUrl"]);
        for (token, status) in [
            (None, 401),
            (Some("bad-token"), 401),
            (Some("ignored-environment-static"), 401),
            (Some("facade-cli-static"), 200),
            (Some("facade-cli-owner"), 200),
            (Some(view.token.as_str()), 200),
        ] {
            let mut request = http
                .post(mcp)
                .header("accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-03-26")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}));
            if let Some(token) = token {
                request = request.bearer_auth(token)
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            if status == 200 {
                let body: Value = response.json().await.unwrap();
                assert!(
                    body["result"]["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| t["name"] == "help")
                );
                assert_eq!(body["result"]["ttlMs"], 0);
                assert_eq!(body["result"]["cacheScope"], "private");
            }
        }
        auth::save(&tokens, &[]).unwrap();
        assert_eq!(
            http.post(mcp)
                .bearer_auth(&view.token)
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        drop(child.stdin.take());
        let exit = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(exit.success());
        for url in [
            health_url,
            banner["hubUrl"].as_str().unwrap().to_owned() + "/health",
        ] {
            assert!(
                http.get(url).send().await.is_err(),
                "owned listener survived launcher exit"
            );
        }
        assert!(!home.join(".claude/settings.json").exists());
    }
}
