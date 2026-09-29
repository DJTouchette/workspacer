//! Provenance rewriting shared by local and remote capability dispatch.
use crate::auth::{Identity, spawn_keys};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) fn sanitize(identity: &Identity, method: &str, mut params: Value) -> Result<Value> {
    if method == "fleetWorkflows.request" && !identity.may_assert_session() {
        bail!("Fleet workflow management is local host only");
    }
    if method == "agents.dispatchReplay" {
        let map = params
            .as_object_mut()
            .ok_or_else(|| anyhow!("invalid replay"))?;
        map.retain(|key, _| !key.eq_ignore_ascii_case("originKey"));
        map.insert("originKey".into(), json!(identity.token_id));
        return Ok(params);
    }
    if method == "agents.reportProgress" && !identity.trusted() {
        if let Some(map) = params.as_object_mut() {
            map.retain(|key, _| !key.eq_ignore_ascii_case("callerSessionId"));
        }
        return Ok(params);
    }
    if !matches!(method, "agents.spawn" | "agents.dispatchPrepare") {
        return Ok(params);
    }
    let Some(map) = params.as_object_mut() else {
        return Ok(params);
    };
    let mut folded = BTreeMap::new();
    for key in map.keys() {
        let lower = key.to_lowercase();
        if let Some(previous) = folded.insert(lower, key) {
            bail!("agents.spawn: params carry {previous:?} and {key:?}, which differ only by case");
        }
    }
    for key in map.keys() {
        if let Some(canonical) = spawn_keys()
            .iter()
            .find(|known| known.eq_ignore_ascii_case(key))
            && key != canonical
        {
            bail!("agents.spawn: non-canonical param {key:?}; use {canonical:?}");
        }
    }
    map.remove("launchIntegrationGranted");
    if let Some(selected) = map.get("launchIntegrationId").filter(|v| !v.is_null()) {
        let id = selected
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 200 && s.trim() == *s)
            .ok_or_else(|| anyhow!("invalid launch integration selection"))?;
        let _ = id;
        if !identity.authenticated_host() {
            bail!("launch integrations require the server owner");
        }
        map.insert("launchIntegrationGranted".into(), json!(true));
    }
    if !identity.may_assert_session() {
        map.remove("dispatchOwnerSessionId");
        map.remove("retrySourceSessionId");
    }
    if !identity.federated {
        map.remove("remoteOrigin");
    } else if let Some(origin) = map.get_mut("remoteOrigin") {
        let origin = origin
            .as_object_mut()
            .ok_or_else(|| anyhow!("invalid remote origin"))?;
        origin.retain(|key, _| !key.eq_ignore_ascii_case("ownerKey"));
        origin.insert("ownerKey".into(), json!(identity.token_id));
    }
    for key in ["profileGranted", "yoloGranted", "escalationScrubbed"] {
        map.remove(key);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Kind, Record};
    fn scoped(scope: &str, facade: bool) -> Identity {
        Identity {
            kind: Kind::Scoped(Record {
                scope: scope.into(),
                facade_authority: facade,
                ..Default::default()
            }),
            token_id: "verified".into(),
            federated: false,
        }
    }
    #[test]
    fn spawn_identity_stamps_cannot_be_supplied_by_a_scoped_operator() {
        let caller = scoped("operator", false);
        let p=sanitize(&caller,"agents.spawn",json!({"remoteOrigin":{"ownerKey":"forged"},"dispatchOwnerSessionId":"someone","retrySourceSessionId":"another","profileGranted":true,"yoloGranted":true,"escalationScrubbed":["fake"],"model":"opus"})).unwrap();
        assert_eq!(p, json!({"model":"opus"}));
        assert!(
            sanitize(
                &caller,
                "agents.spawn",
                json!({"launchIntegrationId":"plugin"})
            )
            .is_err()
        );
        for key in [
            "Model",
            "ProfileGranted",
            "RemoteOrigin",
            "LaunchIntegrationGranted",
        ] {
            assert!(sanitize(&caller, "agents.spawn", json!({key:true})).is_err());
        }
        assert!(sanitize(&caller, "agents.spawn", json!({"unknown":1,"UNKNOWN":2})).is_err());
    }
    #[test]
    fn only_local_host_or_provisioned_facade_can_assert_local_session_lineage() {
        let raw = json!({"dispatchOwnerSessionId":"parent"});
        assert_eq!(
            sanitize(&Identity::host("host"), "agents.spawn", raw.clone()).unwrap(),
            raw
        );
        assert_eq!(
            sanitize(&scoped("operator", true), "agents.spawn", raw.clone()).unwrap(),
            raw
        );
        assert_eq!(
            sanitize(&scoped("provider", true), "agents.spawn", raw.clone()).unwrap(),
            json!({})
        );
        let mut peer = Identity::host("peer");
        peer.federated = true;
        let p=sanitize(&peer,"agents.spawn",json!({"remoteOrigin":{"ownerkey":"forged","dispatchId":"id"},"dispatchOwnerSessionId":"parent"})).unwrap();
        assert_eq!(p["remoteOrigin"]["ownerKey"], peer.token_id);
        assert!(p.get("dispatchOwnerSessionId").is_none());
        assert!(p["remoteOrigin"].get("ownerkey").is_none());
    }
    #[test]
    fn progress_and_replay_use_verified_identity() {
        assert_eq!(sanitize(&scoped("view",false),"agents.reportProgress",json!({"callerSessionId":"forged","CallerSessionId":"also-forged","text":"progress"})).unwrap(),json!({"text":"progress"}));
        assert_eq!(
            sanitize(
                &scoped("operator", false),
                "agents.dispatchReplay",
                json!({"OriginKey":"forged","dispatchId":"one"})
            )
            .unwrap(),
            json!({"originKey":"verified","dispatchId":"one"})
        );
    }
}
