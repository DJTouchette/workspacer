use serde_json::{Value, json};
use std::collections::BTreeMap;
use workspacer_hub::services::quiescence::*;
fn empty(now_ms: i64) -> Evidence {
    Evidence {
        now_ms,
        sessions: Ok(json!([])),
        clients: Ok(vec![]),
        jobs: Ok(vec![]),
        peers: Ok(vec![]),
        operations: vec![],
    }
}
fn kinds(blockers: &[Blocker]) -> Vec<String> {
    let mut v: Vec<_> = blockers.iter().map(|b| b.kind.clone()).collect();
    v.sort();
    v
}
#[test]
fn portable_fleet_quiescence_contract() {
    let data: Value = serde_json::from_str(include_str!(
        "../../../contracts/fleet-quiescence-cases.json"
    ))
    .unwrap();
    for case in data["cases"].as_array().unwrap() {
        let now = 1_000_000;
        let mut input = empty(now);
        input.sessions = case["sessionsError"]
            .as_str()
            .map(|e| Err(e.into()))
            .unwrap_or_else(|| Ok(case["sessions"].clone()));
        input.clients = Ok(case["clients"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, c)| ClientInfo {
                connection_id: i as u64,
                label: c["label"].as_str().unwrap_or_default().into(),
                activity_seq: 1,
                idle_active_ms: now - c["idleForMs"].as_i64().unwrap(),
                provider: false,
                plugin: false,
                internal: false,
            })
            .collect());
        input.jobs = Ok(case["jobs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| JobInfo {
                id: j["id"].as_str().unwrap().into(),
                name: String::new(),
                action_kind: j["kind"].as_str().unwrap().into(),
                next_run_ms: j["nextInMs"].as_i64().map(|n| now + n),
                running: j["running"].as_bool().unwrap_or(false),
            })
            .collect());
        input.peers = Ok(case["peers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| PeerSessions {
                name: p["name"].as_str().unwrap().into(),
                connected: p["error"].is_null(),
                sessions: Ok(p["sessions"].clone()),
            })
            .collect());
        let tun = Tunables {
            keep_jobs_awake: case["keepJobsAwake"].as_bool().unwrap_or(false),
            ..Default::default()
        };
        let mut expected: Vec<String> = serde_json::from_value(case["expect"].clone()).unwrap();
        expected.sort();
        assert_eq!(
            kinds(&evaluate(&input, tun, &BTreeMap::new())),
            expected,
            "{}",
            case["name"]
        );
    }
    for case in data["monitorCases"].as_array().unwrap() {
        let mon = Monitor::new(Tunables {
            dwell_ms: case["dwellMs"].as_i64().unwrap(),
            max_sample_gap_ms: case["maxGapMs"].as_i64().unwrap(),
            ..Default::default()
        });
        let mut result = None;
        for sample in case["samples"].as_array().unwrap() {
            let mut input = empty(sample["at"].as_i64().unwrap());
            input.sessions = Ok(sample["sessions"].clone());
            result = Some(mon.observe(&input, &BTreeMap::new()));
        }
        let result = case["latestAt"]
            .as_i64()
            .map(|at| mon.latest(at))
            .unwrap_or_else(|| result.unwrap());
        assert_eq!(
            json!({"quiescent":result.quiescent,"since":result.since,"calmSeconds":result.calm_seconds,"kinds":kinds(&result.blockers)}),
            case["expect"],
            "{}",
            case["name"]
        );
    }
}
#[test]
fn unknown_evidence_and_same_timestamp_later_activity_never_count_as_quiet() {
    assert_eq!(
        evaluate(
            &Evidence::unknown(1000),
            Tunables::default(),
            &BTreeMap::new()
        )
        .len(),
        4
    );
    let mut input = empty(1000);
    input.clients = Ok(vec![ClientInfo {
        connection_id: 3,
        label: "native UI".into(),
        activity_seq: 8,
        idle_active_ms: 1000,
        provider: false,
        plugin: false,
        internal: false,
    }]);
    assert!(evaluate(&input, Tunables::default(), &BTreeMap::from([(3, 8)])).is_empty());
    assert_eq!(
        kinds(&evaluate(
            &input,
            Tunables::default(),
            &BTreeMap::from([(3, 7)])
        )),
        vec!["client-active"]
    );
    input.clients.as_mut().unwrap()[0].internal = true;
    assert!(evaluate(&input, Tunables::default(), &BTreeMap::new()).is_empty());
}
#[test]
fn power_config_alone_cannot_grant_stop_authority() {
    use power::PowerConfig;
    let config = PowerConfig::from_get(|key| {
        match key {
            "WKS_MACHINE_IDLE_MODE" => "stop",
            "WKS_MACHINE_IDLE_TIMEOUT" => "12m",
            "WKS_MACHINE_POWER" => "fly",
            "WKS_MACHINE_WAKE" => "http",
            "WKS_MACHINE_WAKE_URL" => "https://example.test/wake?secret=x",
            _ => "",
        }
        .into()
    });
    let info = config.info(None);
    assert_eq!(info["canStop"], false);
    assert_eq!(info["idleMode"], "observe");
    assert_eq!(info["wakeUrl"], "");
    assert_eq!(power::duration_ms("1h2m3.5s"), Some(3_723_500));
}
