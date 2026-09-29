use crate::Caller;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct UsagePrefs {
    schedule: Mutex<String>,
    path: Option<PathBuf>,
    routing: Option<Arc<super::routing::RoutingService>>,
}
fn validate(value: &str) -> Result<String> {
    let schedule = value.trim().to_lowercase();
    if schedule != "five_day" && schedule != "seven_day" {
        bail!("usage pacing schedule {value:?} is not five_day or seven_day");
    }
    Ok(schedule)
}
impl UsagePrefs {
    pub fn open(path: Option<PathBuf>) -> Self {
        let schedule = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v["schedule"].as_str().and_then(|s| validate(s).ok()))
            .unwrap_or_default();
        Self {
            schedule: Mutex::new(schedule),
            path,
            routing: None,
        }
    }
    pub fn with_routing(mut self, routing: Arc<super::routing::RoutingService>) -> Self {
        routing.apply_schedule(&self.schedule.lock().unwrap());
        self.routing = Some(routing);
        self
    }
    pub fn get(&self, params: Value) -> Result<Value> {
        if !params.is_null() && !params.as_object().is_some_and(|m| m.is_empty()) {
            bail!("usage.pacingSchedule: no parameters accepted");
        }
        Ok(json!({"schedule":*self.schedule.lock().unwrap(),"configurable":self.path.is_some()}))
    }
    pub fn set(&self, caller: &Caller, params: Value) -> Result<Value> {
        if !caller.trusted {
            bail!("usage.setPacingSchedule requires host authority");
        }
        let selected = validate(params["schedule"].as_str().unwrap_or(""))?;
        let Some(path) = &self.path else {
            bail!("usage pacing schedule cannot be stored: this hub has no preference file");
        };
        let mut schedule = self.schedule.lock().unwrap();
        super::atomic_json(path, &json!({"schedule":selected}), true)?;
        *schedule = selected;
        if let Some(routing) = &self.routing {
            routing.apply_schedule(&schedule);
        }
        Ok(json!({"schedule":*schedule,"configurable":true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_write_uses_legacy_trusted_gate_and_updates_only_projection() {
        let directory = tempfile::tempdir().unwrap();
        let routing =
            Arc::new(super::super::routing::RoutingService::open(directory.path().into()).unwrap());
        let before = routing.matrix();
        let service = UsagePrefs::open(Some(directory.path().join("usage-pacing.json")))
            .with_routing(routing.clone());
        let mut caller = Caller {
            call_id: 0,
            activity_seq: 0,
            federated: false,
            connection_id: 1,
            authenticated_host: false,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        };
        assert_eq!(
            service
                .set(&caller, json!({"schedule":" FIVE_DAY "}))
                .unwrap()["schedule"],
            "five_day"
        );
        assert_eq!(routing.matrix(), before);
        caller.trusted = false;
        assert!(
            service
                .set(&caller, json!({"schedule":"seven_day"}))
                .is_err()
        );
        assert_eq!(
            UsagePrefs::open(Some(directory.path().join("usage-pacing.json")))
                .get(json!({}))
                .unwrap()["schedule"],
            "five_day"
        );
    }
}
