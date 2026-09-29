use super::serve::ServePlan;
use serde_json::{Value, json};
use std::{io::Write, net::SocketAddr};
pub(super) fn banner(
    plan: &ServePlan,
    address: SocketAddr,
    mcp: Option<SocketAddr>,
    token: &str,
    worker: bool,
) -> Value {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", token)
        .finish();
    let daemon = if plan.hub_only {
        plan.external_claudemon.clone()
    } else {
        Some(format!("http://127.0.0.1:{}", plan.api_port))
    };
    json!({"service":"workspacer-rust","hubUrl":format!("http://{address}"),"busUrl":format!("ws://{address}/bus"),"mobileUrl":format!("http://{address}/m?{query}"),"remoteUrl":format!("http://{address}/remote?{query}"),"claudemonUrl":daemon,"mcpUrl":mcp.map(|address|format!("http://{address}/mcp")),"token":token,"database":(!plan.hub_only).then_some(&plan.database),"mode":if plan.hub_only{"hub-only"}else if worker{"worker"}else{"standalone"}})
}
pub(super) fn print_banner(banner: &Value, out: &mut dyn Write) -> std::io::Result<()> {
    writeln!(out, "Workspacer ready")?;
    for (label, key) in [
        ("Hub", "hubUrl"),
        ("Bus", "busUrl"),
        ("Remote", "remoteUrl"),
        ("Mobile", "mobileUrl"),
        ("Claudemon", "claudemonUrl"),
        ("MCP", "mcpUrl"),
        ("Pairing credential", "token"),
    ] {
        if let Some(value) = banner[key].as_str() {
            writeln!(out, "{label}: {value}")?;
        }
    }
    Ok(())
}
pub(super) fn mcp_ready(value: &Value, listen: SocketAddr, hub: &str) -> bool {
    value["status"] == "ok"
        && value["service"] == "workspacer-mcp-facade"
        && value["implementation"] == "rust"
        && value["hubConnected"] == true
        && value["pluginCatalogReady"] == true
        && value["listenAddr"] == listen.to_string()
        && value["hubUrl"] == hub
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_links_roundtrip_opaque_tokens_and_human_banner_keeps_every_endpoint() {
        let address: SocketAddr = "[::1]:7895".parse().unwrap();
        let plan = ServePlan {
            hub_only: false,
            external_claudemon: None,
            config: "config".into(),
            home: "home".into(),
            database: "explicit.sqlite".into(),
            usage_poll_on_boot: Some(false),
            listen: address,
            api_port: 7891,
            hook_port: 7890,
            mcp: Some("127.0.0.1:7897".parse().unwrap()),
        };
        let token = "spaces & plus+ slash/ question?🦀";
        let value = banner(&plan, address, plan.mcp, token, false);
        // Legacy consumers decode the default ready object as string values.
        let flat: std::collections::BTreeMap<String, String> =
            serde_json::from_value(value.clone()).unwrap();
        for key in [
            "busUrl",
            "remoteUrl",
            "mobileUrl",
            "hubUrl",
            "claudemonUrl",
            "token",
        ] {
            assert!(flat.contains_key(key));
        }

        for key in ["remoteUrl", "mobileUrl"] {
            let url = url::Url::parse(value[key].as_str().unwrap()).unwrap();
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                vec![("token".into(), token.into())]
            );
        }
        let mut out = vec![];
        print_banner(&value, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        for key in [
            "hubUrl",
            "busUrl",
            "remoteUrl",
            "mobileUrl",
            "claudemonUrl",
            "mcpUrl",
            "token",
        ] {
            assert!(out.contains(value[key].as_str().unwrap()), "{key}");
        }
        assert!(out.contains("Pairing credential"));
    }
    #[test]
    fn facade_readiness_requires_exact_owned_service_listener_hub_and_catalog() {
        let address: SocketAddr = "127.0.0.1:17897".parse().unwrap();
        let hub = "ws://127.0.0.1:17895/bus";
        let valid = json!({"status":"ok","service":"workspacer-mcp-facade","implementation":"rust","hubConnected":true,"pluginCatalogReady":true,"listenAddr":address.to_string(),"hubUrl":hub});
        assert!(mcp_ready(&valid, address, hub));
        for (key, bad) in [
            ("status", json!("failed")),
            ("service", json!("other")),
            ("implementation", json!("node")),
            ("hubConnected", json!(false)),
            ("pluginCatalogReady", json!(false)),
            ("listenAddr", json!("127.0.0.1:7897")),
            ("hubUrl", json!("ws://127.0.0.1:7895/bus")),
        ] {
            let mut altered = valid.clone();
            altered[key] = bad;
            assert!(!mcp_ready(&altered, address, hub), "{key}");
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(!mcp_ready(&missing, address, hub), "missing {key}");
        }
    }
}
