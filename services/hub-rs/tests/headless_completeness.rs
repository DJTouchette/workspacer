//! Shipping literal client calls must resolve in the actual owned headless hub.
use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{Options, backend::Backend, client::Client};

#[tokio::test]
async fn every_shipped_client_literal_has_a_registered_headless_provider() {
    let brief_methods = declared_brief_methods();
    let calls = regex::Regex::new(
        r"(?s)(?:\bclient\s*\.\s*call|\bbusCall|\bcall)(?:<.*?>)?\(\s*'([a-z][\w.]*\.[\w.]+)'",
    )
    .unwrap();
    let clients: Vec<_> = [
        (
            "web",
            include_str!("../../../apps/desktop/src/renderer/src/backend/webBackend.ts"),
            71,
        ),
        ("mobile", include_str!("../assets/web/mobile.html"), 19),
        ("remote", include_str!("../assets/web/remote.html"), 8),
    ]
    .into_iter()
    .map(|(client, source, minimum)| {
        let names: BTreeSet<_> = calls
            .captures_iter(source)
            .map(|hit| hit[1].to_owned())
            .collect();
        assert!(
            names.len() >= minimum,
            "{client} parser or shipping surface lost its reviewed floor: {} < {minimum}",
            names.len()
        );
        (client, names)
    })
    .collect();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let mut options = Options::default();
    options.home_dir = Some(root.path().join("home"));
    options.config_dir = Some(config.clone());
    // These are the full standalone composition inputs set by cli/serve.rs,
    // not additional method registrations invented by this fixture.
    options.data_dir = Some(config.clone());
    options.jobs_file = Some(config.join("jobs.json"));
    options.scoped_tokens = Some(config.join("tokens.json"));
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "headless-inventory-fixture".into();
    let owner = Backend::start(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("daemon.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (health, provided) = loop {
        let health = owner.handle().health().await.unwrap();
        let provided: BTreeSet<_> = health["methodNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect();
        // Node/provider actors register asynchronously after the bus is ready.
        if (clients.iter().all(|(_, names)| names.is_subset(&provided))
            && brief_methods.is_subset(&provided))
            || tokio::time::Instant::now() >= deadline
        {
            break (health, provided);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let client = Client::connect(&owner.handle()).await.unwrap();
    let listed = client.call("nodes.list", json!({})).await;
    let wake = client
        .call("nodes.wake", json!({"id":"unconfigured"}))
        .await;
    let sleep = client
        .call("nodes.sleep", json!({"id":"unconfigured"}))
        .await;
    client.close();
    owner.shutdown().await.unwrap();
    assert_eq!(listed.unwrap(), json!([]));
    for result in [wake, sleep] {
        assert!(result.unwrap_err().to_string().contains("unknown node"));
    }
    assert_eq!(health["launchReady"], true);
    assert!(
        provided.len() >= 100,
        "headless registration unexpectedly sparse"
    );
    assert!(!provided.contains("fixture.unprovided"));
    let missing_briefs: Vec<_> = brief_methods.difference(&provided).collect();
    assert!(
        missing_briefs.is_empty(),
        "agent-facing brief methods lack providers: {missing_briefs:?}"
    );
    for (client, names) in clients {
        let missing: Vec<_> = names.difference(&provided).collect();
        assert!(
            missing.is_empty(),
            "{client} calls unprovided capabilities: {missing:?}"
        );
    }
}

fn declared_brief_methods() -> BTreeSet<String> {
    let binding =
        regex::Regex::new(r#""(brief_[a-zA-Z_]+)"\s*=>\s*"(brief\.[a-zA-Z.]+)""#).unwrap();
    let bindings: std::collections::BTreeMap<_, _> = binding
        .captures_iter(include_str!("../src/mcp.rs"))
        .map(|capture| (capture[1].to_owned(), capture[2].to_owned()))
        .collect();
    assert!(
        bindings.len() >= 3,
        "brief binding parser lost its reviewed floor"
    );
    let tools: serde_json::Value =
        serde_json::from_str(include_str!("../assets/mcp-effective-tools.json")).unwrap();
    let exposed: Vec<_> = tools["operator"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .filter(|name| name.starts_with("brief_"))
        .collect();
    assert!(exposed.len() >= 3);
    for name in exposed {
        assert!(
            bindings.contains_key(name),
            "exposed {name} has no parsed brief binding"
        );
    }
    let vocabulary: serde_json::Value =
        serde_json::from_str(include_str!("../assets/hub-vocabulary.json")).unwrap();
    bindings
        .into_values()
        .chain(
            vocabulary["methods"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|method| method.as_str())
                .filter(|method| method.starts_with("brief."))
                .map(str::to_owned),
        )
        .collect()
}

#[test]
fn headless_port_keeps_explicit_scope_standing_and_shared_client_wire_markers() {
    use workspacer_hub::auth::Scope;
    let standing: std::collections::BTreeMap<_, _> = [
        ("agents.reportProgress", true),
        ("sessions.recent", true),
        ("agents.reparent", false),
        ("agents.close", false),
        ("agents.orphans", false),
        ("agents.notifyWhen", false),
        ("brief.append", false),
        ("brief.archive", false),
        ("brief.check", false),
        ("terminals.open", false),
        ("fs.readImage", false),
        ("claude.setPermissionMode", false),
        ("claude.setModel", false),
        ("claude.setEffort", false),
        ("claude.handoffBrief", false),
    ]
    .into();
    for method in declared_brief_methods() {
        assert_eq!(
            standing.get(method.as_str()),
            Some(&false),
            "undeclared brief scope standing: {method}"
        );
    }
    for scope in [Scope::View, Scope::Triage] {
        let allowed = scope.methods();
        assert!(allowed.len() >= 10);
        assert!(!allowed.iter().any(|method| method == "*"));
        for (method, scoped) in &standing {
            if !scoped || scope == Scope::View {
                assert_eq!(
                    allowed.iter().any(|allowed| allowed == method),
                    *scoped,
                    "{} {method}",
                    scope.name()
                );
            }
        }
    }
    let mobile = include_str!("../assets/web/mobile.html");
    for marker in [
        "[fleet] Worker escalated — blocked and did not complete:",
        "FLEET_ESCALATION_RE",
        "entry.escalation = escalation[2]",
        "FLEET · WORKER ESCALATED",
    ] {
        assert!(
            mobile.contains(marker),
            "mobile lost escalation marker {marker}"
        );
    }
    let progress = include_str!("../../../apps/desktop/src/main/services/progressReports.ts");
    for declaration in [
        "export const NOTE_MAX = 500",
        "export const MIN_INTERVAL_MS = 60_000",
        "export const MAX_REPORTS = 20",
    ] {
        assert!(
            progress.contains(declaration),
            "progress bounds drifted: {declaration}"
        );
    }
}

#[test]
fn orphan_candidates_exclude_dead_children_and_keep_adoption_guidance() {
    use workspacer_hub::services::agent_ops::orphans;
    let rows = [
        json!({"sessionId":"live-parent","status":"active","isWakeTarget":true}),
        json!({"sessionId":"live-child","status":"active","parentSessionId":"live-parent"}),
        json!({"sessionId":"dead-parent","status":"ended","isWakeTarget":true,"label":"predecessor","cwd":"/project"}),
        json!({"sessionId":"orphan-2","status":"active","parentSessionId":"dead-parent"}),
        json!({"sessionId":"orphan-1","status":"active","parentSessionId":"dead-parent"}),
        json!({"sessionId":"dead-child","status":"ended","parentSessionId":"dead-parent"}),
        json!({"sessionId":"dead-solo","status":"ended","isWakeTarget":true}),
    ];
    let result = orphans(&rows);
    assert_eq!(result["candidates"].as_array().unwrap().len(), 1);
    let candidate = &result["candidates"][0];
    assert_eq!(candidate["sessionId"], "dead-parent");
    assert_eq!(candidate["label"], "predecessor");
    assert_eq!(candidate["confirmedManager"], true);
    assert_eq!(candidate["children"], json!(["orphan-1", "orphan-2"]));
    assert!(
        result["note"]
            .as_str()
            .unwrap()
            .contains("do not guess between two candidates")
    );
    let empty = orphans(&rows[..2]);
    assert_eq!(empty["candidates"], json!([]));
    assert!(
        empty["note"]
            .as_str()
            .unwrap()
            .contains("Nothing is orphaned")
    );
}
