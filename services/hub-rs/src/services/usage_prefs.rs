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

    fn trusted_caller() -> Caller {
        Caller {
            call_id: 0,
            activity_seq: 0,
            federated: false,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        }
    }

    #[test]
    fn absent_storage_and_invalid_parameters_never_claim_a_saved_preference() {
        let memory = UsagePrefs::open(None);
        for params in [Value::Null, json!({})] {
            assert_eq!(
                memory.get(params).unwrap(),
                json!({"schedule":"","configurable":false})
            );
        }
        for params in [json!([]), json!(false), json!({"schedule":"five_day"})] {
            assert!(
                memory
                    .get(params)
                    .unwrap_err()
                    .to_string()
                    .contains("no parameters accepted")
            );
        }
        assert!(
            memory
                .set(&trusted_caller(), json!({"schedule":"five_day"}))
                .is_err()
        );
        assert_eq!(memory.get(Value::Null).unwrap()["schedule"], "");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage-pacing.json");
        let service = UsagePrefs::open(Some(path.clone()));
        let mut untrusted = trusted_caller();
        untrusted.trusted = false;
        untrusted.authenticated_host = false;
        untrusted.scope = "view".into();
        assert!(
            service
                .set(&untrusted, json!({"schedule":"five_day"}))
                .unwrap_err()
                .to_string()
                .contains("usage.setPacingSchedule")
        );
        assert!(!path.exists());
        let expected = service
            .set(&trusted_caller(), json!({"schedule":"seven_day"}))
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        for params in [
            Value::Null,
            json!({}),
            json!({"schedule":false}),
            json!({"schedule":"weekends_only"}),
        ] {
            assert!(service.set(&trusted_caller(), params).is_err());
            assert_eq!(service.get(Value::Null).unwrap(), expected);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn stored_weekday_schedule_changes_only_weekly_expectation_for_actual_reset_interval() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("routing.yaml");
        let policy = b"thresholds:\n  pacing:\n    seven_day:\n      timezone: UTC\n";
        std::fs::write(&policy_path, policy).unwrap();
        let routing =
            Arc::new(super::super::routing::RoutingService::open(dir.path().into()).unwrap());
        let matrix = routing.matrix();
        let path = dir.path().join("usage-pacing.json");
        let service = UsagePrefs::open(Some(path.clone())).with_routing(routing.clone());
        let time = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .timestamp()
        };
        let project = |now, reset| {
            let report = json!({"generated_at":now,"providers":[{"provider":"codex","accounts":[{"account":"","fresh":true,"windows":{
                "seven_day":{"used_percent":{"state":"ok","value":82},"resets_at":reset,"window_minutes":10080},
                "five_hour":{"used_percent":{"state":"ok","value":40},"resets_at":now+5400,"window_minutes":300}
            }}]}]});
            routing.usage_report(&report, now)["providers"][0]["accounts"][0]["windows"].clone()
        };
        let now = time("2026-09-05T15:00:00Z");
        let reset = time("2026-09-07T00:00:00Z");
        let inherited = project(now, reset);
        service
            .set(&trusted_caller(), json!({"schedule":"seven_day"}))
            .unwrap();
        let seven = project(now, reset);
        assert_eq!(inherited, seven);
        service
            .set(&trusted_caller(), json!({"schedule":"five_day"}))
            .unwrap();
        let five = project(now, reset);
        assert_eq!(five["seven_day"]["pace"]["curve"], "five_day");
        assert_eq!(seven["seven_day"]["pace"]["curve"], "calendar");
        assert!(
            five["seven_day"]["pace"]["expectedPct"].as_f64().unwrap()
                > seven["seven_day"]["pace"]["expectedPct"].as_f64().unwrap()
        );
        assert_eq!(
            five["seven_day"]["pace"]["usedPct"],
            seven["seven_day"]["pace"]["usedPct"]
        );
        assert_eq!(five["five_hour"], seven["five_hour"]);
        let reset = time("2026-09-09T15:00:00Z");
        let expected = |at| {
            let windows = project(time(at), reset);
            assert_eq!(windows["seven_day"]["pace"]["known"], true);
            windows["seven_day"]["pace"]["expectedPct"]
                .as_f64()
                .unwrap()
        };
        let friday = expected("2026-09-04T23:00:00Z");
        let saturday = expected("2026-09-05T12:00:00Z");
        let sunday = expected("2026-09-06T12:00:00Z");
        let monday = expected("2026-09-07T12:00:00Z");
        assert_eq!(saturday, sunday);
        assert!(saturday > friday && monday > sunday);
        assert_eq!(routing.matrix(), matrix);
        assert_eq!(std::fs::read(&policy_path).unwrap(), policy);
        let reloaded = UsagePrefs::open(Some(path)).with_routing(routing.clone());
        assert_eq!(reloaded.get(Value::Null).unwrap()["schedule"], "five_day");
        assert_eq!(project(now, time("2026-09-07T00:00:00Z")), five);
        let mut files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                std::ffi::OsString::from("routing.yaml"),
                std::ffi::OsString::from("usage-pacing.json")
            ]
        );
    }
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
