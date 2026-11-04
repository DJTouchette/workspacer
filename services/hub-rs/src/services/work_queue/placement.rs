//! Placement: which host should run an item. Pure and deterministic, so
//! `queue.targets` and `host: "any"` give the same answer and every decision
//! is explained by `reasons`. A peer's advertised facts only choose a target;
//! the destination re-checks admission and authority itself.
use serde_json::{Value, json};

/// What the item needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    /// `spawn` or `shell`.
    pub kind: String,
    /// Wanted provider; empty means any ready provider will do.
    pub provider: String,
    /// Wanted working directory; empty means none.
    pub cwd: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderReady {
    pub provider: String,
    pub found: bool,
    pub authenticated: Option<bool>,
}

/// One host as observed by the caller of [`evaluate`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    /// `local` or the peer name.
    pub host: String,
    pub local: bool,
    pub connected: bool,
    pub dispatch: bool,
    /// The host's `host.resources` object, when it could be read.
    pub resources: Option<Value>,
    /// Provider readiness; `None` when it could not be read (or, locally,
    /// when this hub has no readiness probe).
    pub providers: Option<Vec<ProviderReady>>,
    /// The host's directory choices (`fleet.dispatchCapabilities` cwds).
    pub cwds: Vec<String>,
    /// Local only: the requested cwd is an absolute existing directory.
    pub cwd_exists: bool,
    /// Diagnostics gathered while observing (e.g. a failed peer read).
    pub notes: Vec<String>,
}

/// One `queue.targets` row plus its ordering key.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub local: bool,
    pub eligible: bool,
    pub score: f64,
    pub row: Value,
}

/// Evaluate every candidate. Returns the rows in input order and the best
/// eligible host (highest score; local wins a tie; then host name).
pub fn evaluate(request: &Request, candidates: &[Candidate]) -> (Vec<Target>, Option<String>) {
    let targets: Vec<Target> = candidates.iter().map(|c| target(request, c)).collect();
    let best = targets
        .iter()
        .filter(|t| t.eligible)
        .max_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then(a.local.cmp(&b.local))
                .then(b.host.cmp(&a.host))
        })
        .map(|t| t.host.clone());
    (targets, best)
}

/// The `queue.targets` result object.
pub fn wire(targets: &[Target], best: Option<String>) -> Value {
    json!({
        "targets": targets.iter().map(|t| t.row.clone()).collect::<Vec<_>>(),
        "best": best,
    })
}

fn ready(provider: &ProviderReady, local: bool) -> bool {
    // A peer must confirm its login (the same bar remote dispatch admission
    // sets); locally an unconfirmed login is still worth trying.
    provider.found
        && if local {
            provider.authenticated != Some(false)
        } else {
            provider.authenticated == Some(true)
        }
}

fn ineligibility(request: &Request, c: &Candidate) -> Vec<String> {
    let mut why = Vec::new();
    if !c.connected {
        why.push("not connected".to_owned());
    }
    if !c.local && !c.dispatch {
        why.push("dispatch is not enabled for this peer".to_owned());
    }
    if request.kind == "spawn" {
        match &c.providers {
            None if c.local => {}
            None => why.push("provider readiness unavailable".to_owned()),
            Some(providers) if request.provider.is_empty() => {
                if !providers.iter().any(|p| ready(p, c.local)) {
                    why.push("no provider is installed and logged in".to_owned());
                }
            }
            Some(providers) => match providers.iter().find(|p| p.provider == request.provider) {
                Some(p) if ready(p, c.local) => {}
                Some(p) if !p.found => {
                    why.push(format!("provider {} is not installed", request.provider))
                }
                Some(_) => why.push(format!("provider {} is not logged in", request.provider)),
                None => why.push(format!("provider {} is not installed", request.provider)),
            },
        }
    }
    if !request.cwd.is_empty() {
        if c.local {
            if !c.cwd_exists {
                why.push(format!(
                    "cwd {} is not an existing directory on this host",
                    request.cwd
                ));
            }
        } else if !c.cwds.contains(&request.cwd) {
            why.push(format!(
                "cwd {} is not among this host's directory choices",
                request.cwd
            ));
        }
    }
    why
}

fn target(request: &Request, c: &Candidate) -> Target {
    let mut reasons = ineligibility(request, c);
    let eligible = reasons.is_empty();
    let queue = c.resources.as_ref().map(|r| &r["queue"]);
    let gate_open = queue.is_some_and(|q| q["gate"]["open"] == true);
    let (slots, running) = if request.kind == "shell" {
        ("shellSlots", "shellRunning")
    } else {
        ("agentSlots", "agentsRunning")
    };
    let number = |key: &str| queue.and_then(|q| q[key].as_u64()).unwrap_or(0);
    let free = number(slots).saturating_sub(number(running));
    let queued = number("queued");
    let memory = c
        .resources
        .as_ref()
        .and_then(|r| r["memory"]["pressureAvg10"].as_f64())
        .filter(|p| p.is_finite())
        .map(|p| p.clamp(0.0, 100.0));
    let mut score = 0.0;
    if eligible {
        // Lexicographic tiers folded into one number: gate, free slots,
        // queue length, memory pressure. Each tier's span is smaller than one
        // step of the tier above, so the number orders exactly like the tuple.
        score = 1.0
            + if gate_open { 1_000_000.0 } else { 0.0 }
            + free.min(99) as f64 * 10_000.0
            + (99 - queued.min(99)) as f64 * 100.0
            + (100.0 - memory.unwrap_or(0.0)) * 0.99;
        score = (score * 100.0).round() / 100.0;
        let noun = if request.kind == "shell" {
            "shell"
        } else {
            "agent"
        };
        match (&c.resources, gate_open) {
            (None, _) => reasons.push("resources unavailable".to_owned()),
            (Some(_), true) => reasons.push("gate open".to_owned()),
            (Some(resources), false) => {
                let why: Vec<&str> = resources["queue"]["gate"]["reasons"]
                    .as_array()
                    .map(|r| r.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                reasons.push(if why.is_empty() {
                    "gate closed".to_owned()
                } else {
                    format!("gate closed: {}", why.join("; "))
                });
            }
        }
        if c.resources.is_some() {
            reasons.push(format!("{free} free {noun} slots"));
            reasons.push(format!("{queued} queued"));
            if let Some(memory) = memory {
                reasons.push(format!("memory pressure {memory:.1}"));
            }
        }
        if c.local && request.kind == "spawn" && c.providers.is_none() {
            reasons.push("provider readiness not probed on this host".to_owned());
        }
    }
    reasons.extend(c.notes.iter().cloned());
    let providers: Vec<Value> = c
        .providers
        .iter()
        .flatten()
        .map(
            |p| json!({"provider": p.provider, "found": p.found, "authenticated": p.authenticated}),
        )
        .collect();
    Target {
        host: c.host.clone(),
        local: c.local,
        eligible,
        score,
        row: json!({
            "host": c.host,
            "local": c.local,
            "connected": c.connected,
            "dispatch": c.local || c.dispatch,
            "resources": c.resources,
            "providers": providers,
            "cwds": c.cwds,
            "eligible": eligible,
            "score": score,
            "reasons": reasons,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(open: bool, agents: (u64, u64), queued: u64, memory: Option<f64>) -> Value {
        json!({
            "memory": {"pressureAvg10": memory},
            "queue": {
                "agentSlots": agents.0, "agentsRunning": agents.1,
                "shellSlots": 2, "shellRunning": 0, "queued": queued,
                "gate": {"open": open, "reasons": if open { json!([]) } else { json!(["memory pressure 30.0 >= 25.0"]) }},
            },
        })
    }
    fn claude(found: bool, authenticated: Option<bool>) -> Option<Vec<ProviderReady>> {
        Some(vec![ProviderReady {
            provider: "claude".into(),
            found,
            authenticated,
        }])
    }
    fn local(resources: Value) -> Candidate {
        Candidate {
            host: "local".into(),
            local: true,
            connected: true,
            dispatch: true,
            resources: Some(resources),
            providers: claude(true, None),
            cwd_exists: true,
            ..Default::default()
        }
    }
    fn peer(name: &str, resources: Value) -> Candidate {
        Candidate {
            host: name.into(),
            connected: true,
            dispatch: true,
            resources: Some(resources),
            providers: claude(true, Some(true)),
            cwds: vec!["/work/repo".into()],
            ..Default::default()
        }
    }
    fn spawn(cwd: &str) -> Request {
        Request {
            kind: "spawn".into(),
            provider: "claude".into(),
            cwd: cwd.into(),
        }
    }
    fn reasons(target: &Target) -> Vec<String> {
        serde_json::from_value(target.row["reasons"].clone()).unwrap()
    }

    #[test]
    fn ineligible_hosts_say_why_and_are_never_best() {
        let mut offline = peer("offline", resources(true, (4, 0), 0, None));
        offline.connected = false;
        let mut closed = peer("closed", resources(true, (4, 0), 0, None));
        closed.dispatch = false;
        let mut no_login = peer("no-login", resources(true, (4, 0), 0, None));
        no_login.providers = claude(true, None);
        let mut missing = peer("missing", resources(true, (4, 0), 0, None));
        missing.providers = claude(false, Some(false));
        let mut elsewhere = peer("elsewhere", resources(true, (4, 0), 0, None));
        elsewhere.cwds = vec!["/other".into()];
        let mut unknown = peer("unknown", resources(true, (4, 0), 0, None));
        unknown.providers = None;
        unknown.notes = vec!["capabilities: not authorized".into()];
        let mut here = local(resources(true, (4, 0), 0, None));
        here.cwd_exists = false;
        let candidates = [offline, closed, no_login, missing, elsewhere, unknown, here];
        let (targets, best) = evaluate(&spawn("/work/repo"), &candidates);
        assert_eq!(best, None);
        assert!(targets.iter().all(|t| !t.eligible && t.score == 0.0));
        let all: Vec<Vec<String>> = targets.iter().map(reasons).collect();
        assert_eq!(all[0], ["not connected"]);
        assert_eq!(all[1], ["dispatch is not enabled for this peer"]);
        assert_eq!(all[2], ["provider claude is not logged in"]);
        assert_eq!(all[3], ["provider claude is not installed"]);
        assert_eq!(
            all[4],
            ["cwd /work/repo is not among this host's directory choices"]
        );
        assert_eq!(
            all[5],
            [
                "provider readiness unavailable",
                "capabilities: not authorized"
            ]
        );
        assert_eq!(
            all[6],
            ["cwd /work/repo is not an existing directory on this host"]
        );
    }

    #[test]
    fn gate_then_free_slots_then_queue_then_memory_then_local() {
        let request = spawn("");
        let pick = |candidates: &[Candidate]| evaluate(&request, candidates).1.unwrap();
        // An open gate beats any number of free slots behind a closed one.
        assert_eq!(
            pick(&[
                local(resources(false, (64, 0), 0, Some(0.0))),
                peer("open", resources(true, (1, 0), 50, Some(20.0))),
            ]),
            "open"
        );
        // More free slots of the item's kind.
        assert_eq!(
            pick(&[
                local(resources(true, (4, 3), 0, None)),
                peer("roomy", resources(true, (4, 1), 9, None)),
            ]),
            "roomy"
        );
        // Then a shorter queue.
        assert_eq!(
            pick(&[
                local(resources(true, (4, 0), 3, None)),
                peer("idle", resources(true, (4, 0), 0, Some(24.0))),
            ]),
            "idle"
        );
        // Then lower memory pressure.
        assert_eq!(
            pick(&[
                local(resources(true, (4, 0), 0, Some(10.0))),
                peer("calm", resources(true, (4, 0), 0, Some(2.5))),
            ]),
            "calm"
        );
        // A full tie goes to this machine; between peers, to the first name.
        assert_eq!(
            pick(&[
                peer("b", resources(true, (4, 0), 0, None)),
                local(resources(true, (4, 0), 0, None)),
            ]),
            "local"
        );
        assert_eq!(
            pick(&[
                peer("b", resources(true, (4, 0), 0, None)),
                peer("a", resources(true, (4, 0), 0, None)),
            ]),
            "a"
        );
    }

    #[test]
    fn shell_items_count_shell_slots_and_skip_provider_checks() {
        let request = Request {
            kind: "shell".into(),
            ..Default::default()
        };
        let mut busy = local(resources(true, (4, 0), 0, None));
        busy.resources.as_mut().unwrap()["queue"]["shellRunning"] = json!(2);
        busy.providers = Some(vec![]);
        let mut free = peer("free", resources(true, (4, 4), 0, None));
        free.providers = None;
        let (targets, best) = evaluate(&request, &[busy, free]);
        assert_eq!(best.as_deref(), Some("free"));
        assert_eq!(
            reasons(&targets[1]),
            ["gate open", "2 free shell slots", "0 queued"]
        );
        assert_eq!(targets[0].row["providers"], json!([]));
    }

    #[test]
    fn unknown_resources_and_an_unprobed_local_host_stay_eligible_but_rank_low() {
        let mut blind = peer("blind", Value::Null);
        blind.resources = None;
        let mut unprobed = local(resources(false, (4, 0), 0, None));
        unprobed.providers = None;
        let (targets, best) = evaluate(&spawn(""), &[blind, unprobed]);
        assert!(targets.iter().all(|t| t.eligible));
        assert_eq!(reasons(&targets[0]), ["resources unavailable"]);
        assert_eq!(
            reasons(&targets[1]),
            [
                "gate closed: memory pressure 30.0 >= 25.0",
                "4 free agent slots",
                "0 queued",
                "provider readiness not probed on this host"
            ]
        );
        assert_eq!(best.as_deref(), Some("local"));
        let row = &targets[1].row;
        assert_eq!(row["dispatch"], true);
        assert_eq!(row["local"], true);
        assert!(row["score"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn scores_are_deterministic_and_order_like_the_tuple() {
        let a = evaluate(
            &spawn(""),
            &[local(resources(true, (4, 1), 2, Some(12.345)))],
        );
        let b = evaluate(
            &spawn(""),
            &[local(resources(true, (4, 1), 2, Some(12.345)))],
        );
        assert_eq!(a, b);
        // 1 + gate + 3 free + 97 queue-room + 87.655 * 0.99
        assert_eq!(a.0[0].score, 1_039_787.78);
    }
}
