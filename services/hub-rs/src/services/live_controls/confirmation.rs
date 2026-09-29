//! Confirmed local observations, not launch records or credential authority.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

#[derive(Default)]
pub(crate) struct ConfirmedControls(Mutex<State>);
#[derive(Default)]
struct State {
    next: u64,
    rows: BTreeMap<String, Record>,
}
struct Record {
    stamp: u64,
    identity: Value,
    patch: Value,
}
fn identity(row: &Value) -> Option<(String, Value)> {
    let id = row["sessionId"]
        .as_str()
        .or_else(|| row["session_id"].as_str())?;
    if id.is_empty()
        || row["hub"].as_str().is_some_and(|h| !h.is_empty())
        || row["status"] == "ended"
        || row["mode"] == "stopped"
    {
        return None;
    }
    // Require actual engine observation rather than a guessed/default identity.
    if !row["started_at"].is_string() && !row["execution_engine"].is_object() {
        return None;
    }
    Some((
        id.into(),
        json!([
            row["started_at"],
            row["execution_engine"],
            row["pid"],
            row["provider"],
            row["transport"]
        ]),
    ))
}
fn apply(row: &mut Value, patch: &Value) {
    if let Some(settings) = patch["settings"].as_object() {
        for key in ["model", "effort", "permissionMode"] {
            if let Some(value) = settings.get(key).filter(|v| v.is_string()) {
                if !row["settings"].is_object() {
                    row["settings"] = json!({});
                }
                row["settings"][key] = value.clone();
                if key == "effort" {
                    row["liveEffort"] = value.clone();
                }
            }
        }
    }
    for key in ["livePermissionMode", "requestedSelection"] {
        if let Some(value) = patch.get(key) {
            row[key] = value.clone();
        }
    }
}
impl ConfirmedControls {
    pub(crate) fn observe(&self, row: &Value) -> Option<u64> {
        let mut state = self.0.lock().unwrap();
        let Some((id, identity)) = identity(row) else {
            if let Some(id) = row["sessionId"]
                .as_str()
                .or_else(|| row["session_id"].as_str())
            {
                state.rows.remove(id);
            }
            return None;
        };
        if state.rows.get(&id).is_none_or(|r| r.identity != identity) {
            state.next = state
                .next
                .checked_add(1)
                .expect("control observation stamp exhausted");
            let stamp = state.next;
            state.rows.insert(
                id.clone(),
                Record {
                    stamp,
                    identity,
                    patch: json!({}),
                },
            );
        }
        Some(state.rows[&id].stamp)
    }
    pub(crate) fn forget(&self, id: &str) {
        self.0.lock().unwrap().rows.remove(id);
    }
    pub(crate) fn retain(&self, ids: &BTreeSet<String>) {
        self.0.lock().unwrap().rows.retain(|id, _| ids.contains(id));
    }
    pub(crate) fn confirm(&self, row: &mut Value, stamp: Option<u64>, patch: &Value) -> bool {
        let Some((id, identity)) = identity(row) else {
            return false;
        };
        let mut state = self.0.lock().unwrap();
        let Some(record) = state.rows.get_mut(&id) else {
            return false;
        };
        if Some(record.stamp) != stamp || record.identity != identity {
            return false;
        }
        apply(&mut record.patch, patch);
        apply(row, &record.patch);
        true
    }
    pub(crate) fn enrich(&self, mut row: Value) -> Value {
        if let Some((id, identity)) = identity(&row) {
            if let Some(record) = self
                .0
                .lock()
                .unwrap()
                .rows
                .get(&id)
                .filter(|r| r.identity == identity)
            {
                apply(&mut row, &record.patch);
            }
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observed() -> Value {
        json!({"session_id":"manual","sessionId":"manual","started_at":"2026-09-29T00:00:00Z","provider":"claude","mode":"input"})
    }
    #[test]
    fn only_confirmed_same_life_local_fields_survive_observations() {
        let controls = ConfirmedControls::default();
        let mut row = observed();
        let stamp = controls.observe(&row);
        assert!(controls.enrich(row.clone()).get("liveEffort").is_none());
        assert!(controls.confirm(&mut row,stamp,&json!({"settings":{"effort":"high","model":"opus","permissionMode":"plan","profileId":"forged"},"livePermissionMode":"plan","parentSessionId":"forged"})));
        let fresh = controls.enrich(observed());
        assert_eq!(fresh["liveEffort"], "high");
        assert_eq!(fresh["settings"]["model"], "opus");
        assert!(fresh["settings"].get("profileId").is_none());
        assert!(fresh.get("parentSessionId").is_none());
        let mut remote = fresh.clone();
        remote["hub"] = "peer".into();
        assert!(!controls.confirm(&mut remote, stamp, &json!({"settings":{"effort":"low"}})));
        for ended in [json!({"mode":"stopped"}), json!({"status":"ended"})] {
            let mut terminal = observed();
            for (k, v) in ended.as_object().unwrap() {
                terminal[k] = v.clone();
            }
            controls.observe(&terminal);
            assert!(!controls.confirm(
                &mut observed(),
                stamp,
                &json!({"settings":{"effort":"low"}})
            ));
        }
        let next = controls.observe(&observed());
        assert_ne!(next, stamp);
        assert!(controls.enrich(observed()).get("liveEffort").is_none());
        controls.retain(&BTreeSet::new());
        assert!(!controls.confirm(&mut observed(), next, &json!({"settings":{"effort":"low"}})));
        controls.observe(&observed());
        controls.forget("manual"); // SessionStart resets even when daemon reuses started_at.
        assert!(!controls.confirm(&mut observed(), next, &json!({"settings":{"effort":"low"}})));
        let mut changed = observed();
        changed["started_at"] = "2026-09-29T01:00:00Z".into();
        let current = controls.observe(&changed);
        assert!(!controls.confirm(&mut changed, next, &json!({"settings":{"effort":"low"}})));
        assert!(current.is_some());
    }
}
