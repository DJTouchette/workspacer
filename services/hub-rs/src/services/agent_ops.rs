//! Local dismissal and orphan discovery. Dismissals are generation fenced so
//! retained daemon history cannot resurrect rows, while an intentional resume can.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Mutex};

pub fn ended(row: &Value) -> bool {
    row["status"] == "ended" || row["mode"] == "stopped"
}
pub fn validate_close(row: &Value) -> Result<()> {
    if row["hub"].as_str().is_some_and(|h| !h.is_empty()) {
        bail!("remote sessions must be closed on their owning hub");
    }
    if !ended(row)
        && (matches!(
            row["ambientState"].as_str(),
            Some("thinking" | "streaming" | "background")
        ) || row["mode"] == "responding"
            || row["background_tasks"].as_u64().unwrap_or(0) > 0)
    {
        bail!("session is working; wait for it to finish before closing");
    }
    Ok(())
}
#[derive(Default)]
pub struct Dismissals(Mutex<BTreeMap<String, (String, Value)>>);
impl Dismissals {
    pub fn dismiss(&self, id: &str, generation: String, row: Value) {
        let mut minimal = json!({"sessionId":id});
        for key in ["label", "cwd", "isWakeTarget", "lastActivity"] {
            if let Some(value) = row.get(key) {
                minimal[key] = value.clone();
            }
        }
        self.0
            .lock()
            .unwrap()
            .insert(id.into(), (generation, minimal));
    }
    pub fn retain_inventory(&self, ids: &std::collections::BTreeSet<String>) {
        self.0.lock().unwrap().retain(|id, _| ids.contains(id));
    }
    pub fn allows(&self, id: &str, generation: &str) -> bool {
        let mut rows = self.0.lock().unwrap();
        if let Some((old, _)) = rows.get(id) {
            if old == generation || generation.is_empty() {
                return false;
            }
            rows.remove(id);
        }
        true
    }
    pub fn tombstones(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap()
            .values()
            .map(|(_, row)| {
                let mut row = row.clone();
                row["status"] = json!("ended");
                row["mode"] = json!("stopped");
                row
            })
            .collect()
    }
}
pub fn orphans(rows: &[Value]) -> Value {
    let parents: BTreeMap<_, _> = rows
        .iter()
        .filter_map(|r| r["sessionId"].as_str().map(|id| (id, r)))
        .collect();
    let mut candidates: BTreeMap<String, Value> = BTreeMap::new();
    for row in rows {
        let id = row["sessionId"].as_str().unwrap_or("");
        let parent = row["parentSessionId"].as_str().unwrap_or("");
        if id.is_empty()
            || parent.is_empty()
            || parent == id
            || ended(row)
            || row["hub"].as_str().is_some_and(|s| !s.is_empty())
        {
            continue;
        }
        let known = parents.get(parent).copied();
        if known.is_some_and(|r| !ended(r)) {
            continue;
        }
        let candidate=candidates.entry(parent.into()).or_insert_with(||{
            let mut value=json!({"sessionId":parent,"confirmedManager":known.is_some_and(|r|r["isWakeTarget"]==true),"children":[]});
            if let Some(known)=known{for key in ["label","cwd"]{if let Some(field)=known.get(key){value[key]=field.clone();}}if let Some(at)=known.get("lastActivity"){value["endedAt"]=at.clone();}}
            value
        });
        candidate["children"]
            .as_array_mut()
            .unwrap()
            .push(json!(id));
    }
    let mut rows: Vec<_> = candidates.into_values().collect();
    for row in &mut rows {
        row["children"]
            .as_array_mut()
            .unwrap()
            .sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    rows.sort_by(|a, b| {
        b["children"]
            .as_array()
            .unwrap()
            .len()
            .cmp(&a["children"].as_array().unwrap().len())
            .then_with(|| a["sessionId"].as_str().cmp(&b["sessionId"].as_str()))
    });
    let note = if rows.is_empty() {
        "Nothing is orphaned here: every live agent either has a live parent or was never dispatched by one.".to_owned()
    } else {
        let confirmed = rows
            .iter()
            .filter(|row| row["confirmedManager"] == true)
            .count();
        format!(
            "{} dead parent(s) still have live children; {confirmed} are confirmed managers. Pick the one you are replacing — match its label/cwd against what you were told to take over — and pass its sessionId as fromSessionId to adopt_workers. Adopting the wrong group re-points another manager's workers onto you, so do not guess between two candidates: read a worker of each first. An unknown parent is not proof of a manager.",
            rows.len()
        )
    };
    json!({"candidates":rows,"note":note})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dismissal_survives_updates_but_not_new_generation() {
        let d = Dismissals::default();
        d.dismiss(
            "a",
            "one".into(),
            json!({"sessionId":"a","isWakeTarget":true}),
        );
        assert!(!d.allows("a", "one"));
        assert!(!d.allows("a", ""));
        assert_eq!(d.tombstones()[0]["status"], "ended");
        assert!(d.allows("a", "two"));
        assert!(d.tombstones().is_empty());
        d.dismiss(
            "b",
            "one".into(),
            json!({"lastReply":"large reply","label":"kept"}),
        );
        assert!(d.tombstones()[0].get("lastReply").is_none());
        d.retain_inventory(&Default::default());
        assert!(d.allows("b", "one"));
    }
    #[test]
    fn working_refusal_and_orphan_provenance() {
        assert!(validate_close(&json!({"status":"active","ambientState":"streaming"})).is_err());
        assert!(validate_close(&json!({"status":"ended","ambientState":"streaming"})).is_ok());
        let result = orphans(&[
            json!({"sessionId":"dead","status":"ended","isWakeTarget":true}),
            json!({"sessionId":"b","parentSessionId":"dead"}),
            json!({"sessionId":"a","parentSessionId":"unknown"}),
            json!({"sessionId":"remote","parentSessionId":"dead","hub":"peer"}),
        ]);
        assert_eq!(result["candidates"][0]["confirmedManager"], true);
        assert_eq!(result["candidates"][1]["confirmedManager"], false);
    }
}
