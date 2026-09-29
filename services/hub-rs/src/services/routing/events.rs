//! The public event is deliberately narrower than the private decision audit.
use super::*;
fn assignment(value: &Value) -> Value {
    let mut out = json!({"provider":word(&value["provider"]),"model":word(&value["model"])});
    for key in ["effort", "minEffort"] {
        let source = if key == "minEffort" && value[key].is_null() {
            &value["min_effort"]
        } else {
            &value[key]
        };
        if !word(source).is_empty() {
            out[key] = source.clone();
        }
    }
    if value["fresh"] == true {
        out["fresh"] = true.into();
    }
    if value["enabled"].is_boolean() {
        out["enabled"] = value["enabled"].clone();
    }
    if let Some(rows) = value["alternatives"]
        .as_array()
        .filter(|rows| !rows.is_empty())
    {
        out["alternatives"] = rows.iter().map(assignment).collect::<Vec<_>>().into();
    }
    out
}
pub(super) fn projection(decision: &Value) -> Value {
    let mut out = json!({
        "decisionId":word(&decision["decisionId"]),"role":word(&decision["role"]),
        "capability":word(&decision["capability"]),"baseCapability":word(&decision["baseCapability"]),
        "eligible":decision["eligible"]==true,"mode":word(&decision["mode"]),
        "modeManual":decision["modeManual"]==true,"decidedAt":decision["decidedAt"].as_i64().unwrap_or(0)
    });
    for key in ["ticketId", "profile", "provider", "model", "effort"] {
        if !word(&decision[key]).is_empty() {
            out[key] = decision[key].clone();
        }
    }
    let capacity = if decision["shiftCapacity"].is_object()
        && decision["shiftCapacity"]["provider"] == decision["provider"]
    {
        &decision["shiftCapacity"]
    } else {
        &decision["capacity"]
    };
    if !word(&capacity["health"]).is_empty() {
        out["health"] = capacity["health"].clone();
    }
    let ceiling = &decision["ceiling"];
    if ["capabilityRefused", "toolScopeRefused", "denied"]
        .iter()
        .any(|key| ceiling[key] == true)
    {
        out["ceilingCapped"] = true.into();
    }
    if !word(&ceiling["maxCapability"]).is_empty() {
        out["ceilingMaxCapability"] = ceiling["maxCapability"].clone();
    }
    if let Some(reasons) = decision["reason"]
        .as_array()
        .filter(|rows| !rows.is_empty())
    {
        let key = word(&ceiling["key"]);
        out["reason"] = reasons
            .iter()
            .filter_map(Value::as_str)
            .map(|reason| {
                // The Rust policy's private explanation names its matched directory.
                // Public events must not disclose that key through prose either.
                if !key.is_empty() && key != "default" {
                    reason.replace(key, "[project]")
                } else {
                    reason.into()
                }
            })
            .collect::<Vec<_>>()
            .into();
    }
    let pace = &capacity["pace"];
    if !word(&pace["state"]).is_empty() {
        out["pace"] = pace["state"].clone();
    }
    if pace["known"] == true {
        if let Some(ratio) = pace["ratio"].as_f64().filter(|ratio| ratio.is_finite()) {
            out["paceRatio"] = json!(ratio);
        }
        if !word(&pace["window"]).is_empty() {
            out["paceWindow"] = pace["window"].clone();
        }
    }
    if decision["fellOverFrom"].is_object() {
        out["fellOverFrom"] = assignment(&decision["fellOverFrom"]);
    }
    if decision["effortStep"].is_object() {
        let step = &decision["effortStep"];
        let mut value = json!({"why":if step["why"].is_string() {word(&step["why"])} else {word(&step["because"])}});
        for key in ["from", "to"] {
            if !word(&step[key]).is_empty() {
                value[key] = step[key].clone();
            }
        }
        out["effortStep"] = value;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_projection_tracks_chosen_capacity_and_never_exports_private_audit_fields() {
        let mut decision = json!({"decisionId":"decision","provider":"codex","capacity":{"provider":"claude","health":"red","pace":{"known":true,"state":"overspending","ratio":2.0,"window":"seven_day"}},"shiftCapacity":{"provider":"codex","health":"green","pace":{"known":true,"state":"on_track","ratio":0.0,"window":"five_hour"}},"cwd":"PRIVATE_CWD","account":"PRIVATE_ACCOUNT","matrix":{"path":"PRIVATE_MATRIX"},"ceiling":{"key":"PRIVATE_CWD","maxCapability":"cheap","capabilityRefused":true},"reason":["directory ceiling PRIVATE_CWD clamps capability to cheap"],"fellOverFrom":{"provider":"claude","model":"opus","apiKey":"PRIVATE_KEY","alternatives":[{"provider":"codex","model":"model","cwd":"PRIVATE_NESTED"}]},"effortStep":{"from":"high","to":"medium","because":"mode step","secret":"PRIVATE_STEP"}});
        let projected = projection(&decision);
        assert_eq!(projected["health"], "green");
        assert_eq!(projected["paceRatio"], 0.0);
        assert_eq!(projected["paceWindow"], "five_hour");
        assert_eq!(
            projected["effortStep"],
            json!({"from":"high","to":"medium","why":"mode step"})
        );
        assert!(!projected.to_string().contains("PRIVATE_"));
        decision["shiftCapacity"]["provider"] = "other".into();
        assert_eq!(projection(&decision)["health"], "red");
        decision["capacity"]["pace"] =
            json!({"known":false,"state":"unknown","ratio":0.0,"window":"five_hour"});
        assert!(projection(&decision).get("paceRatio").is_none());
        assert!(projection(&decision).get("paceWindow").is_none());
        decision["capacity"].as_object_mut().unwrap().remove("pace");
        assert!(projection(&decision).get("pace").is_none());
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::{
        Hub, Options, auth,
        client::Client,
        protocol::{Event, Frame},
    };
    async fn next(connection: &mut crate::Connection) -> Frame {
        tokio::time::timeout(Duration::from_secs(5), connection.recv())
            .await
            .unwrap()
            .unwrap()
    }
    #[tokio::test]
    async fn select_broadcasts_a_private_data_free_decision_to_viewers_but_preview_does_not() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("PRIVATE_PROJECT");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            root.path().join("config.yaml"),
            "agents:\n  checkProviderOnStartup: false\n",
        )
        .unwrap();
        let mut matrix = json!({"ceilings":{}});
        matrix["ceilings"][project.to_str().unwrap()] = json!({"max_capability":"cheap"});
        std::fs::write(
            root.path().join("routing.yaml"),
            serde_yaml::to_string(&matrix).unwrap(),
        )
        .unwrap();
        let tokens = root.path().join("tokens.json");
        let viewer = auth::mint(&tokens, auth::Scope::View, "fixture viewer").unwrap();
        let mut options = Options::default();
        options.config_dir = Some(root.path().into());
        options.scoped_tokens = Some(tokens);
        let hub = Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let mut watcher = hub
            .handle()
            .connect_authenticated(viewer.token, false)
            .await
            .unwrap();
        assert_eq!(next(&mut watcher).await.op, "hello");
        watcher
            .send(Frame {
                topics: vec!["routing.decision".into(), "agent.snapshot".into()],
                ..Frame::op("subscribe")
            })
            .unwrap();
        assert_eq!(next(&mut watcher).await.op, "subscribed");
        let caller = Client::connect(&hub.handle()).await.unwrap();
        let params = json!({"role":"implementer","cwd":project,"account":"PRIVATE_ACCOUNT","ticketId":"ticket"});
        caller
            .call("routing.preview", params.clone())
            .await
            .unwrap();
        hub.handle()
            .publish_wait(Event::new(
                "agent.snapshot",
                "fixture",
                json!({"sentinel":true}),
            ))
            .await
            .unwrap();
        assert_eq!(
            next(&mut watcher).await.event.unwrap().topic,
            "agent.snapshot"
        );
        let answer = caller.call("routing.select", params).await.unwrap();
        let event = next(&mut watcher).await.event.unwrap();
        assert_eq!(event.topic, "routing.decision");
        assert_eq!(event.source, "routing");
        let data = event.data.unwrap();
        assert_eq!(data["decisionId"], answer["decisionId"]);
        assert_eq!(data["ticketId"], "ticket");
        assert_eq!(data["capability"], "cheap");
        assert!(!data.to_string().contains("PRIVATE_"));
        assert!(data.get("matrix").is_none());
        let log = std::fs::read_to_string(root.path().join("routing-decisions.jsonl")).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&log).unwrap()["decisionId"],
            answer["decisionId"]
        );
        hub.shutdown().unwrap();
    }
}
