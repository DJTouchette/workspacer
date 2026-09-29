//! Authorization compositions are separate decisions from per-method tier
//! membership. This retains authtoken/composition_test.go without a Go runner.
//! Concrete closure mechanisms remain tested by jobs.rs, services.rs, stores.rs,
//! auth.rs, push/tests.rs and the desktop replay containment suites; a closed
//! entry here is bound to its exact pair and cannot exempt an unrelated pair.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use workspacer_hub::{
    Hub, Options,
    auth::{self, Scope},
    client::Client,
};
fn policy() -> Value {
    serde_json::from_str(include_str!("fixtures/authorization-compositions.json")).unwrap()
}
fn words(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}
fn scopes() -> BTreeMap<String, BTreeSet<String>> {
    [Scope::View, Scope::Triage]
        .into_iter()
        .map(|s| (s.name().into(), s.methods().iter().cloned().collect()))
        .collect()
}
fn closed_pair(proof: &str) -> Option<(&'static str, &'static str)> {
    Some(match proof {
        "jobs-owner" => ("jobs.upsert", "jobs.run"),
        "layout-scrub" => ("layout.set", "agents.spawn"),
        // Still provided by Electron, not a new standalone Rust feature.
        "replay-containment" => ("replay.open", "replay.read"),
        "terminal-topics" => ("sessions.attachTerminal", "pty.bytes.*"),
        "saved-session-scrub" => ("sessions.save", "agents.spawn"),
        "saved-layout-scrub" => ("layouts.save", "agents.spawn"),
        "push-endpoint" => ("push.subscribe", "agents.sendMessage"),
        _ => return None,
    })
}
fn check(policy: &Value, tiers: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    let mut errors = vec![];
    let actors = words(&policy["actors"]);
    let inert = words(&policy["inert"]);
    let vocabulary: Value =
        serde_json::from_str(include_str!("../assets/hub-vocabulary.json")).unwrap();
    let methods = words(&vocabulary["methods"]);
    if actors.len() < 50
        || !actors.is_disjoint(&inert)
        || actors.union(&inert).cloned().collect::<BTreeSet<_>>() != methods
    {
        errors.push("actor classification is incomplete or contradictory".into());
    }
    let pairs = policy["pairs"].as_array().unwrap();
    if pairs.len() < 8 {
        errors.push("composition population collapsed".into());
    }
    let mut open_held = 0;
    let mut distinct_pairs = BTreeSet::new();
    for pair in pairs {
        let a = pair["a"].as_str().unwrap();
        let b = pair["b"].as_str().unwrap();
        if !distinct_pairs.insert((a, b)) {
            errors.push(format!("duplicate composition {a} + {b}"));
        }
        let accepted = words(&pair["acceptedIn"]);
        let closed = pair["closedProof"].as_str();
        if let Some(proof) = closed {
            if closed_pair(proof) != Some((a, b)) || !accepted.is_empty() {
                errors.push(format!("invalid pair-bound closure {a} + {b}"));
            }
        }
        for (tier, held) in tiers {
            let both = held.contains(a) && held.contains(b);
            if both && closed.is_none() {
                open_held += 1;
                if !accepted.contains(tier) {
                    errors.push(format!("unaccepted composition {tier}: {a} + {b}"));
                }
            }
            if accepted.contains(tier) && !both {
                errors.push(format!("stale acceptance {tier}: {a} + {b}"));
            }
        }
        for tier in accepted {
            if !tiers.contains_key(&tier) {
                errors.push(format!("unknown accepted tier {tier}"));
            }
        }
    }
    if open_held == 0 {
        errors.push("no open held composition evaluated".into());
    }
    for (tier, held) in tiers {
        if !held.is_subset(&methods) {
            errors.push(format!("tier {tier} grants unclassified methods"));
        }
        let allowed = policy["acknowledgedActors"].get(tier);
        let Some(allowed) = allowed else {
            errors.push(format!("missing actor acknowledgments {tier}"));
            continue;
        };
        let allowed = words(allowed);
        let actual = held.intersection(&actors).cloned().collect::<BTreeSet<_>>();
        if actual != allowed {
            errors.push(format!("actor acknowledgments drifted for {tier}"));
        }
        if tier == "triage" && actual.is_empty() {
            errors.push("triage holds no actors".into());
        }
    }
    for absent in [
        "agents.spawn",
        "terminals.create",
        "sessions.terminalInput",
        "sessions.attachTerminal",
        "claude.answer",
        "fs.write",
        "fs.read",
        "git.commit",
        "git.push",
        "config.save",
        "layout.set",
        "claude.gate",
    ] {
        if tiers["triage"].contains(absent) {
            errors.push(format!("triage acquired deliberately absent {absent}"));
        }
    }
    errors
}
#[test]
fn accepted_compositions_and_actor_acknowledgments_match_actual_scope_methods() {
    assert_eq!(check(&policy(), &scopes()), Vec::<String>::new());
    assert_eq!(
        Scope::Provider.methods(),
        &["layout.get", "plugins.prepareLaunch"]
    );
    assert!(!Scope::Provider.methods().contains(&"*".into()));
    for method in Scope::Provider.methods() {
        assert!(method == "plugins.prepareLaunch" || Scope::View.methods().contains(method));
    }
}
#[test]
fn composition_mutations_cannot_hide_in_a_closed_or_stale_exception() {
    let p = policy();
    let mut tiers = scopes();
    tiers
        .get_mut("view")
        .unwrap()
        .insert("agents.sendMessage".into());
    tiers
        .get_mut("view")
        .unwrap()
        .insert("claude.approve".into());
    assert!(check(&p, &tiers).iter().any(|e| e.contains("unaccepted")));
    let mut p = policy();
    p["pairs"][7]["closedProof"] = json!("push-endpoint");
    p["pairs"][7]["acceptedIn"] = json!([]);
    assert!(
        check(&p, &scopes())
            .iter()
            .any(|e| e.contains("pair-bound"))
    );
    let mut tiers = scopes();
    tiers.get_mut("triage").unwrap().remove("claude.approve");
    assert!(
        check(&policy(), &tiers)
            .iter()
            .any(|e| e.contains("stale acceptance"))
    );
    let mut tiers = scopes();
    tiers
        .get_mut("view")
        .unwrap()
        .insert("sessions.save".into());
    assert!(
        check(&policy(), &tiers)
            .iter()
            .any(|e| e.contains("actor acknowledgments"))
    );
    let mut p = policy();
    p["actors"]
        .as_array_mut()
        .unwrap()
        .retain(|x| x != "sessions.save");
    assert!(
        check(&p, &scopes())
            .iter()
            .any(|e| e.contains("classification"))
    );
}
#[test]
fn provider_runbooks_cannot_mint_a_node_on_the_human_operator_ladder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mint =
        regex::Regex::new(r"workspacer(?:-rust)? token create[^\n]*--scope\s+([a-z]+)[^\n]*")
            .unwrap();
    for line in [
        "workspacer token create --label fly-node --scope operator",
        "workspacer-rust token create --scope operator --label fly-node",
    ] {
        let row = mint.captures(line).unwrap();
        assert!(row[0].contains("fly-node"));
        assert_eq!(&row[1], "operator");
    }
    for file in ["deploy/fly/RUNBOOK.md", "deploy/fly/node/RUNBOOK.md"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        let mut found = 0;
        for row in mint.captures_iter(&text) {
            if row[0].contains("fly-node") {
                found += 1;
                assert_eq!(&row[1], "provider", "{file}: {}", &row[0]);
            }
        }
        assert!(found > 0, "{file} omitted its provider mint instructions");
    }
}
#[tokio::test]
async fn triage_composition_is_real_authorization_but_does_not_grant_other_actors() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = dir.path().join("tokens.json");
    let mut options = Options::default();
    options.scoped_tokens = Some(tokens.clone());
    // Observe broker authorization with inert stand-ins, never invoke a model,
    // send a prompt or resolve a real provider approval.
    for method in [
        "agents.sendMessage",
        "claude.approve",
        "sessions.save",
        "claude.setPermissionMode",
        "fs.write",
        "agents.spawn",
    ] {
        options = options.handler(method, |caller, _| async move {
            Ok(json!({"scope":caller.scope,"host":caller.authenticated_host}))
        });
    }
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    for scope in [Scope::View, Scope::Triage, Scope::Provider] {
        let record = auth::mint(&tokens, scope, "composition-fixture").unwrap();
        let client = Client::from_connection(
            hub.handle()
                .connect_authenticated(record.token, false)
                .await
                .unwrap(),
        );
        for method in ["agents.sendMessage", "claude.approve"] {
            let result = client
                .call(method, json!({"sessionId":"fixture","text":"inert"}))
                .await;
            if scope == Scope::Triage {
                let result = result.unwrap();
                assert_eq!(result["scope"], "triage");
                assert_eq!(result["host"], false);
            } else {
                assert!(result.is_err(), "{method}");
            }
        }
        for method in [
            "sessions.save",
            "claude.setPermissionMode",
            "fs.write",
            "agents.spawn",
        ] {
            assert!(
                client.call(method, json!({})).await.is_err(),
                "{} {method}",
                scope.name()
            );
        }
        client.close();
    }
    hub.shutdown().unwrap();
}
