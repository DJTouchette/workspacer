//! Shared standalone/embedded routing policy. Requests never supply quota evidence.
use super::limits;
mod audit;
pub(crate) use audit::SpawnAudit;
mod events;
pub(crate) mod path;
mod preferences;
use path::path_within;
mod raw;
mod sampler;
#[cfg(windows)]
mod windows_audit;
use anyhow::{Context, Result, bail};
pub use raw::validate_preferences_raw;
pub use sampler::{UsageSampler, install};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

fn word(v: &Value) -> &str {
    v.as_str().unwrap_or("").trim()
}
fn provider(s: &str) -> String {
    match s.trim().to_lowercase().as_str() {
        "anthropic" => "claude".into(),
        "openai" => "codex".into(),
        s => s.into(),
    }
}
fn model(s: &str) -> String {
    let s = s.trim().to_lowercase();
    for suffix in ["[1m]", "-1m"] {
        if s.len() > suffix.len() && s.ends_with(suffix) {
            return s[..s.len() - suffix.len()].into();
        }
    }
    s
}
fn merge(base: &mut Value, patch: &Value) {
    if let (Some(dst), Some(src)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in src {
            if v.is_null() {
                continue;
            }
            if let Some(old) = dst.get_mut(k) {
                merge(old, v);
            } else {
                dst.insert(k.clone(), v.clone());
            }
        }
    } else if !patch.is_null() {
        *base = patch.clone();
    }
}
fn defaults() -> Value {
    serde_yaml::from_str(include_str!("routing.default.yaml")).expect("compiled routing policy")
}
fn validate_matrix_shape(matrix: &Value) -> Result<()> {
    for key in [
        "roles",
        "profiles",
        "providers",
        "thresholds",
        "mode_shifts",
        "modes",
        "ceilings",
        "capability_ranks",
    ] {
        if !matrix[key].is_object() {
            bail!("routing {key} must be a mapping");
        }
    }
    for rows in matrix["profiles"].as_object().unwrap().values() {
        let rows = rows
            .as_object()
            .context("routing profile must be mapping")?;
        for row in rows.values() {
            if !row.is_object() {
                bail!("routing assignment must be mapping");
            }
            if let Some(alternatives) = row.get("alternatives") {
                if !alternatives
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_object))
                {
                    bail!("routing alternatives must be assignment array");
                }
            }
        }
    }
    Ok(())
}
fn normalize(matrix: &mut Value) {
    for profile in matrix["profiles"]
        .as_object_mut()
        .into_iter()
        .flat_map(|m| m.values_mut())
    {
        for row in profile
            .as_object_mut()
            .into_iter()
            .flat_map(|m| m.values_mut())
        {
            row["provider"] = provider(word(&row["provider"])).into();
            for alt in row["alternatives"].as_array_mut().into_iter().flatten() {
                alt["provider"] = provider(word(&alt["provider"])).into();
            }
        }
    }
}
fn candidates(row: &Value) -> Vec<&Value> {
    std::iter::once(row)
        .chain(row["alternatives"].as_array().into_iter().flatten())
        .collect()
}
fn rank(matrix: &Value, c: &str) -> i64 {
    matrix["capability_ranks"][c].as_i64().unwrap_or(i64::MAX)
}
fn active_profile(matrix: &Value) -> &str {
    let named = word(&matrix["active_profile"]);
    if matrix["profiles"].get(named).is_some() {
        named
    } else {
        "mixed"
    }
}
fn assignment(matrix: &Value, profile: &str, cap: &str, pin: &str) -> Result<Value> {
    let row = &matrix["profiles"][profile][cap];
    if row.is_null() {
        bail!("profile {profile} does not resolve capability {cap}");
    }
    if pin.is_empty() {
        return Ok(row.clone());
    }
    for a in candidates(row) {
        if provider(word(&a["provider"])) == pin {
            return Ok(a.clone());
        }
    }
    for rows in matrix["profiles"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for a in candidates(&rows[cap]) {
            if provider(word(&a["provider"])) == pin {
                return Ok(a.clone());
            }
        }
    }
    bail!("capability {cap} cannot be routed on explicitly requested provider {pin}")
}
fn canonical(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        None
    } else {
        std::fs::canonicalize(path).ok()
    }
}
fn ceiling<'a>(matrix: &'a Value, cwd: &str) -> Option<(&'a str, &'a Value)> {
    let rows = matrix["ceilings"].as_object()?;
    let mut best = rows.get_key_value("default").map(|(k, v)| (k.as_str(), v));
    let mut longest = 0;
    if let Some(cwd) = canonical(cwd) {
        for (key, value) in rows {
            if key == "default" {
                continue;
            }
            if let Some(root) = canonical(key) {
                if path_within(&cwd, &root) && root.as_os_str().len() > longest {
                    longest = root.as_os_str().len();
                    best = Some((key.as_str(), value));
                }
            }
        }
    }
    best
}
fn freshness(matrix: &Value, params: &Value) -> Option<String> {
    let profile = active_profile(matrix);
    let role = word(&params["role"]);
    let mut caps = vec![
        word(&params["capability"]).to_lowercase(),
        word(&matrix["roles"][role]).to_lowercase(),
    ];
    for shift in matrix["mode_shifts"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        caps.push(word(&shift[role]).to_lowercase());
    }
    caps.into_iter()
        .find(|c| !c.is_empty() && matrix["profiles"][profile][c]["fresh"] == true)
}
fn named_rank(matrix: &Value, p: &str, m: &str, e: &str) -> i64 {
    if p.is_empty() || m.is_empty() {
        return 0;
    }
    let mut strongest = if matrix["_host_authority"].is_object() {
        named_rank(&matrix["_host_authority"], p, m, e)
    } else {
        0
    };
    for rows in matrix["profiles"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for (cap, row) in rows.as_object().into_iter().flat_map(|m| m.iter()) {
            for a in candidates(row) {
                if provider(word(&a["provider"])) == p
                    && model(word(&a["model"])) == model(m)
                    && (e.is_empty() || word(&a["effort"]).eq_ignore_ascii_case(e))
                {
                    strongest = strongest.max(rank(matrix, cap));
                }
            }
        }
    }
    strongest
}
// Read the same model carriers as actual spawn admission. A canonical-only
// selection must not disappear at the ceiling lookup, and two conflicting
// companions must not let policy authorize one model while a provider uses another.
fn requested_model(params: &Value) -> Result<String> {
    fn optional_text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok((!value.trim().is_empty()).then_some(value.as_str())),
            _ => bail!("{key} must be text"),
        }
    }
    let window = match params.get("contextWindow") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| anyhow::anyhow!("invalid-context-window"))?,
        ),
    };
    let legacy = optional_text(params, "model")?;
    let identity = optional_text(params, "modelIdentity")?;
    let Some(pin) = optional_text(params, "provider")? else {
        // A remote profile/default may choose the provider. Preserve the
        // reference's unknown-provider classification rather than guessing it.
        // The two syntax families are Claude markers and opaque identities;
        // conflicting companions that neither family accepts are always invalid.
        if legacy.is_some()
            && identity.is_some()
            && !["claude", "codex"].iter().any(|provider| {
                crate::model_selection::normalize_model_input(provider, legacy, identity, window)
                    .is_ok()
            })
        {
            bail!("invalid model selection: conflicting-model-identity");
        }
        return Ok(identity.or(legacy).unwrap_or_default().to_owned());
    };
    Ok(
        crate::model_selection::normalize_model_input(pin, legacy, identity, window)
            .map_err(|error| anyhow::anyhow!("invalid model selection: {}", error.code()))?
            .map(|value| value.selection.model)
            .unwrap_or_default(),
    )
}
/// Called after profile/config defaults resolve, and before any provider launches.
/// Only changed field names are returned; caller-authored receipts are never consulted.
pub fn sanitize(matrix: &Value, params: &mut Value) -> Result<Vec<String>> {
    if !word(&params["resumeSessionId"]).is_empty() {
        if let Some(cap) = freshness(matrix, params) {
            bail!(
                "resume refused for session {}: routing.yaml capability {cap} requires fresh context; omit resumeSessionId",
                word(&params["resumeSessionId"])
            );
        }
    }
    let requested_model = requested_model(params)?;
    let Some((key, c)) = ceiling(matrix, word(&params["cwd"])) else {
        return Ok(vec![]);
    };
    let max = word(&c["max_capability"]);
    if max.is_empty() {
        return Ok(vec![]);
    }
    let limit = rank(matrix, max);
    if limit == i64::MAX {
        bail!("ceiling {key} names unranked capability {max}; spawn refused");
    }
    let capability = word(&params["capability"]).to_lowercase();
    let p = provider(word(&params["provider"]));
    let over = (!capability.is_empty() && rank(matrix, &capability) > limit)
        || named_rank(matrix, &p, &requested_model, word(&params["effort"])) > limit;
    if !over {
        return Ok(vec![]);
    }
    if params["exactModel"] == true {
        bail!(
            "exactModel conflicts with routing.yaml {key} capability ceiling {max}; no substitute was launched"
        );
    }
    let safe = assignment(matrix, active_profile(matrix), max, &p)?;
    if safe["enabled"] == false
        || matrix["providers"][word(&safe["provider"])]["enabled"] == false
        || word(&safe["model"]).is_empty()
    {
        bail!("ceiling {key} has no enabled concrete replacement");
    }
    let mut changed = vec![];
    for (field, value) in [
        ("capability", json!(max)),
        ("provider", safe["provider"].clone()),
        ("model", safe["model"].clone()),
        ("effort", safe["effort"].clone()),
    ] {
        if params[field] != value {
            changed.push(field.into());
            if value.is_null() {
                params
                    .as_object_mut()
                    .context("spawn requires object")?
                    .remove(field);
            } else {
                params[field] = value;
            }
        }
    }
    Ok(changed)
}
fn mode(matrix: &Value, cap: &Value, demand: Option<f64>) -> (&'static str, bool) {
    let p = word(&cap["provider"]);
    let local = word(&matrix["modes"]["providers"][p]);
    let global = word(&matrix["modes"]["global"]);
    let manual = if local.is_empty() || local == "auto" {
        global
    } else {
        local
    };
    match manual {
        "normal" => return ("normal", true),
        "conserve" => return ("conserve", true),
        "spend_down" => return ("spend_down", true),
        _ => {}
    }
    let health = word(&cap["effectiveHealth"]);
    let pace = word(&cap["pace"]["state"]);
    let buckets: Vec<&Value> = cap["buckets"].as_array().into_iter().flatten().collect();
    if matches!(health, "red" | "exhausted")
        || pace == "overspending"
        || demand.is_some_and(|d| {
            buckets
                .iter()
                .any(|b| b["remainingPercent"].as_f64().is_some_and(|r| d > r))
        })
    {
        return ("conserve", false);
    }
    if health == "green" && !matches!(pace, "ahead" | "overspending") {
        let th = &matrix["thresholds"]["spend_down"];
        if let Some(demand) = demand {
            for b in buckets {
                if let (Some(t), Some(r)) = (
                    b["resetsInSeconds"].as_f64(),
                    b["remainingPercent"].as_f64(),
                ) {
                    if t > 0.
                        && t < th["time_to_reset_minutes"].as_f64().unwrap_or(0.) * 60.
                        && r >= th["min_remaining_pct"].as_f64().unwrap_or(100.)
                        && demand
                            <= r * th["max_forecast_pct_of_remaining"].as_f64().unwrap_or(0.) / 100.
                    {
                        return ("spend_down", false);
                    }
                }
            }
        }
    }
    ("normal", false)
}
fn forecast(matrix: &Value, params: &Value) -> Value {
    if let Some(pct) = params["forecastDemandBeforeResetPct"].as_f64() {
        return json!({"known":pct>=0.,"pctOfAllowance":pct.max(0.),"because":if pct>=0. {format!("caller forecasts {pct}% of allowance before reset")}else{"negative forecast cannot establish demand".into()}});
    }
    let mut units = 0.;
    let mut counts = std::collections::BTreeMap::<String, i64>::new();
    let mut unknown = Vec::new();
    for work in params["expectedWork"].as_array().into_iter().flatten() {
        let n = work["count"].as_i64().unwrap_or(0);
        if n <= 0 {
            continue;
        }
        let phase = word(&work["phase"]);
        if let Some(w) = matrix["forecast_weights"][phase].as_f64() {
            units += n as f64 * w;
            *counts.entry(phase.into()).or_default() += n;
        } else {
            unknown.push(phase.to_string());
        }
    }
    unknown.sort();
    let phases: Vec<_> = counts
        .iter()
        .map(|(p, n)| format!("{p} x{n} @ {}", matrix["forecast_weights"][p]))
        .collect();
    json!({"known":false,"pctOfAllowance":0,"units":units,"phases":phases,"unweightedPhases":unknown,"because":"weighted work units are not a measured quota percentage"})
}
fn usable(matrix: &Value, row: &Value, capacity: &Value, mode: &str) -> bool {
    row["enabled"] != false
        && matrix["providers"][word(&row["provider"])]["enabled"] != false
        && !word(&row["model"]).is_empty()
        && !matches!(word(&capacity["effectiveHealth"]), "red" | "exhausted")
        && mode != "conserve"
}
fn effort_step(matrix: &Value, row: &mut Value, cap: &str, mode: &str) -> Option<Value> {
    let shifts = &matrix["mode_shifts"][mode];
    let step = shifts["effort_step"].as_i64().unwrap_or(0);
    if step == 0
        || !shifts["effort_step_capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == cap))
    {
        return None;
    }
    let from = word(&row["effort"]).to_string();
    let p = word(&row["provider"]);
    let ladder: &[&str] = match p {
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        "codex" => &["minimal", "low", "medium", "high", "xhigh"],
        _ => &[],
    };
    let mut to = from.clone();
    let mut reason = "effort ladder unavailable or unspecified";
    if let Some(at) = ladder.iter().position(|x| *x == from) {
        let floor = word(&row["min_effort"]);
        let min = ladder.iter().position(|x| *x == floor).unwrap_or(0);
        if min <= at {
            to = ladder[(at as i64 + step).clamp(min as i64, ladder.len() as i64 - 1) as usize]
                .into();
            reason = "mode effort step bounded by provider ladder and min_effort";
        } else {
            reason = "min_effort exceeds declared effort; step refused";
        }
    }
    if !to.is_empty() {
        row["effort"] = to.clone().into();
    }
    Some(json!({"from":from,"to":to,"step":step,"because":reason}))
}
pub fn select(matrix: &Value, params: &Value, report: &Value, now: i64) -> Result<Value> {
    let role = word(&params["role"]);
    let shipped = defaults();
    let declared = word(&matrix["roles"][role]);
    let base = if declared.is_empty()
        || matrix["profiles"][active_profile(matrix)]
            .get(declared)
            .is_none()
    {
        word(&shipped["roles"][role])
    } else {
        declared
    };
    if base.is_empty() {
        return Ok(
            json!({"role":role,"capability":"","baseCapability":"","profile":matrix["active_profile"],"provider":"","mode":"normal","eligible":false,"decidedAt":now,"reason":[format!("unknown routing role {role:?}")]}),
        );
    }
    let profile = if word(&params["profile"]).is_empty()
        || matrix["profiles"].get(word(&params["profile"])).is_none()
    {
        active_profile(matrix)
    } else {
        word(&params["profile"])
    };
    let pin = provider(if word(&params["provider"]).is_empty() {
        word(&params["preferredProvider"])
    } else {
        word(&params["provider"])
    });
    let account = if word(&params["account"]).is_empty() {
        word(&params["profileId"])
    } else {
        word(&params["account"])
    };
    let demand = params["forecastDemandBeforeResetPct"]
        .as_f64()
        .filter(|d| d.is_finite() && *d >= 0.);
    let mut row = match assignment(matrix, profile, base, &pin) {
        Ok(row) => row,
        Err(error) => {
            return Ok(
                json!({"role":role,"capability":base,"baseCapability":base,"profile":profile,"provider":pin,"mode":"normal","eligible":false,"decidedAt":now,"reason":[error.to_string()]}),
            );
        }
    };
    if matrix["providers"][word(&row["provider"])]["enabled"] == false {
        return Ok(
            json!({"role":role,"capability":base,"baseCapability":base,"profile":profile,"provider":row["provider"],"mode":"normal","eligible":false,"decidedAt":now,"reason":["subject provider explicitly disabled"]}),
        );
    }
    let capacity = limits::capacity(matrix, report, word(&row["provider"]), account, now);
    let (mode, manual) = mode(matrix, &capacity, demand);
    let mode_provider = word(&row["provider"]).to_string();
    let mut reasons = vec![format!(
        "{mode_provider}: observed {}, effective {}, mode {mode}",
        word(&capacity["health"]),
        word(&capacity["effectiveHealth"])
    )];
    let mut cap = base.to_string();
    let shift = word(&matrix["mode_shifts"][mode][role]);
    let mut shift_capacity = None;
    if !shift.is_empty() {
        if let Ok(candidate) = assignment(matrix, profile, shift, &pin) {
            let landing =
                limits::capacity(matrix, report, word(&candidate["provider"]), account, now);
            let (landing_mode, _) = self::mode(matrix, &landing, demand);
            if word(&candidate["provider"]) == mode_provider
                || usable(matrix, &candidate, &landing, landing_mode)
            {
                cap = shift.into();
                row = candidate;
                reasons.push(format!("{mode} shifts {role} from {base} to {cap}"));
            }
            shift_capacity = Some(landing);
        }
    }
    let mut ceiling_out = None;
    if let Some((key, limit)) = ceiling(matrix, word(&params["cwd"])) {
        let max = word(&limit["max_capability"]);
        if !max.is_empty() {
            if rank(matrix, max) == i64::MAX {
                bail!("ceiling {key} cannot rank {max}");
            }
            let refused = rank(matrix, &cap) > rank(matrix, max);
            ceiling_out = Some(json!({"key":key,"maxCapability":max,"capabilityRefused":refused}));
            if refused {
                cap = max.into();
                row = assignment(matrix, profile, &cap, &pin)?;
                reasons.push(format!(
                    "directory ceiling {key} clamps capability to {cap}"
                ));
            }
        }
    }
    let primary = row.clone();
    let mut fell_over = None;
    if pin.is_empty()
        && row["alternatives"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    {
        let previous = provider(word(&params["previousProvider"]));
        let independent =
            params["independentFamily"] == true || params["requireIndependentFamily"] == true;
        let mut options = candidates(&primary);
        if independent && !previous.is_empty() {
            options.sort_by_key(|a| provider(word(&a["provider"])) == previous);
        }
        for candidate in options {
            let candidate_capacity =
                limits::capacity(matrix, report, word(&candidate["provider"]), account, now);
            let (candidate_mode, _) = self::mode(matrix, &candidate_capacity, demand);
            if matrix["_catalog"][word(&candidate["provider"])]["state"] != "unavailable"
                && usable(matrix, candidate, &candidate_capacity, candidate_mode)
            {
                if candidate != &primary {
                    row = candidate.clone();
                    fell_over = Some(primary.clone());
                    shift_capacity = Some(candidate_capacity);
                    reasons.push(format!(
                        "selected alternative {} {}",
                        word(&row["provider"]),
                        word(&row["model"])
                    ));
                }
                break;
            }
        }
    }
    let step_mode = if mode == "normal" && capacity["pace"]["state"] == "ahead" {
        let landing = shift_capacity
            .as_ref()
            .filter(|c| c["provider"] == row["provider"])
            .unwrap_or(&capacity);
        if landing["pace"]["state"] == "ahead" {
            "conserve"
        } else {
            "normal"
        }
    } else {
        mode
    };
    let stepped = if matrix["mode_shifts"][step_mode]["effort_step"]
        .as_i64()
        .unwrap_or(0)
        > 0
        && rank(matrix, &cap) > rank(matrix, base)
    {
        Some(
            json!({"from":row["effort"],"to":row["effort"],"because":"capability was already promoted; one promotion per decision"}),
        )
    } else {
        effort_step(matrix, &mut row, &cap, step_mode)
    };
    let eligible = row["enabled"] != false
        && matrix["providers"][word(&row["provider"])]["enabled"] != false
        && !word(&row["model"]).is_empty();
    let mut out = json!({"role":role,"capability":cap,"baseCapability":base,"profile":profile,"provider":row["provider"],"model":if eligible {row["model"].clone()} else {json!("")},"effort":row["effort"],"fresh":row["fresh"]==true || primary["fresh"]==true,"eligible":eligible,"independentFamily":word(&params["previousProvider"]).is_empty() || provider(word(&params["previousProvider"]))!=provider(word(&row["provider"])),"mode":mode,"modeManual":manual,"modeProvider":mode_provider,"capacity":capacity,"demand":forecast(matrix,params),"reason":reasons,"decidedAt":now});
    if let Some(c) = ceiling_out {
        out["ceiling"] = c;
    }
    if let Some(c) = shift_capacity {
        out["shiftCapacity"] = c;
    }
    if let Some(c) = fell_over {
        out["fellOverFrom"] = c;
    }
    if let Some(c) = stepped {
        out["effortStep"] = c;
    }
    if params["ticketId"].is_string() {
        out["ticketId"] = params["ticketId"].clone();
    }
    Ok(out)
}
struct State {
    matrix: Value,
    checked: Instant,
    stamp: String,
    error: Option<String>,
    schedule: String,
    catalog: Value,
    preferences: Option<preferences::Sources>,
}
pub struct RoutingService {
    directory: PathBuf,
    log: audit::DecisionLog,
    state: Mutex<State>,
}
impl RoutingService {
    pub fn open(directory: PathBuf) -> Result<Self> {
        let (mut matrix, error, preferences) = match preferences::load(&directory) {
            Ok(source) => (source.effective.clone(), None, Some(source)),
            Err(error) => {
                // A broken managed overlay cannot erase valid host-owned ceilings.
                let mut base = defaults();
                if let Ok(bytes) = std::fs::read(directory.join("routing.yaml")) {
                    if let Ok(patch) = serde_yaml::from_slice::<Value>(&bytes) {
                        if patch.is_object() {
                            merge(&mut base, &patch);
                        }
                    }
                }
                if validate_matrix_shape(&base).is_err() {
                    base = defaults();
                }
                (base, Some(error.to_string()), None)
            }
        };
        normalize(&mut matrix);
        let stamp = stamp(&matrix);
        Ok(Self {
            log: audit::DecisionLog::new(&directory),
            directory,
            state: Mutex::new(State {
                matrix,
                checked: Instant::now(),
                stamp,
                error,
                schedule: String::new(),
                catalog: json!({}),
                preferences,
            }),
        })
    }
    pub fn matrix(&self) -> Value {
        let mut state = self.state.lock().unwrap();
        if state.checked.elapsed() >= Duration::from_secs(30) {
            state.checked = Instant::now();
            match preferences::load(&self.directory) {
                Ok(source) => {
                    state.preferences = Some(source.clone());
                    state.matrix = source.effective;
                    state.stamp = stamp(&state.matrix);
                    state.error = None;
                }
                Err(error) => state.error = Some(format!("{error}; previous policy retained")),
            }
        }
        let mut matrix = state.matrix.clone();
        matrix["_catalog"] = state.catalog.clone();
        matrix
    }

    pub fn preferences(
        &self,
        caller: &crate::Caller,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        preferences::handle(self, caller, method, params)
    }
    pub fn update_catalog(&self, provider: &str, models: Option<Vec<Value>>) {
        let mut state = self.state.lock().unwrap();
        state.catalog[provider] = match models {
            Some(models) => {
                json!({"state":if models.is_empty() && provider != "claude" {"unavailable"} else {"available"},"observedAt":chrono::Utc::now().timestamp_millis(),"models":models})
            }
            None => json!({"state":"unknown","observedAt":chrono::Utc::now().timestamp_millis()}),
        };
    }
    pub fn catalog(&self) -> Value {
        self.state.lock().unwrap().catalog.clone()
    }
    pub fn apply_schedule(&self, schedule: &str) {
        self.state.lock().unwrap().schedule = schedule.into();
    }
    pub fn sanitize_spawn(&self, params: &mut Value) -> Result<Vec<String>> {
        sanitize(&self.matrix(), params)
    }
    pub fn select(&self, params: Value, report: &Value, now: i64) -> Result<Value> {
        let mut out = select(&self.matrix(), &params, report, now)?;
        out["decisionId"] = uuid::Uuid::new_v4().to_string().into();
        let state = self.state.lock().unwrap();
        out["matrix"] = json!({"path":self.directory.join("routing.yaml"),"hash":state.stamp,"error":state.error});
        Ok(out)
    }
    pub fn usage_report(&self, report: &Value, now: i64) -> Value {
        let mut config = self.matrix()["thresholds"]["pacing"].clone();
        let schedule = self.state.lock().unwrap().schedule.clone();
        match schedule.as_str() {
            "five_day" => {
                config["seven_day"]["curve"] = "five_day".into();
                config["seven_day"]["weekend_weight"] = 0.into();
                config["seven_day"]["weekend"] = "spend_tail".into();
                config["seven_day"]["weekend_reserve_pct"] = 0.into();
            }
            "seven_day" => config["seven_day"]["curve"] = "calendar".into(),
            _ => {}
        }
        limits::projection(report, &config, now)
    }
}
fn stamp(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_routing_contract() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/routing-policy-cases.json"
        ))
        .unwrap();
        let now = corpus["now"].as_i64().unwrap();
        fn check(name: &str, want: &Value, got: &Value) {
            if let Some(map) = want.as_object() {
                for (k, v) in map {
                    check(&format!("{name}.{k}"), v, &got[k]);
                }
            } else if let (Some(a), Some(b)) = (want.as_f64(), got.as_f64()) {
                assert!((a - b).abs() < 1e-10, "{name}: {a} != {b}");
            } else {
                assert_eq!(want, got, "{name}");
            }
        }
        for case in corpus["cases"].as_array().unwrap() {
            let mut matrix = defaults();
            merge(&mut matrix, &case["patch"]);
            normalize(&mut matrix);
            let mut providers = Vec::new();
            for (p, c) in case["capacity"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
            {
                let account = c.get("account").cloned().unwrap_or(json!(""));
                let fresh = c.get("fresh").cloned().unwrap_or(json!(true));
                let window = json!({"used_percent":{"state":"ok","value":c["used"]},"resets_at":now+c["resetIn"].as_i64().unwrap(),"window_minutes":c["minutes"],"is_current":true});
                providers.push(json!({"provider":p,"accounts":[{"is_default":account=="","account":account,"fresh":fresh,"windows":{"five_hour":window,"seven_day":window,"monthly":{"used_percent":{"state":"unavailable"}}}}]}));
            }
            let report = json!({"generated_at":now,"providers":providers});
            let got = select(&matrix, &case["request"], &report, now).unwrap();
            check(case["name"].as_str().unwrap(), &case["expect"], &got);
        }
    }
    #[test]
    fn overview_schedule_does_not_modify_routing_policy() {
        let dir = tempfile::tempdir().unwrap();
        let service = RoutingService::open(dir.path().into()).unwrap();
        let before = service.matrix();
        let before_decision = select(&before, &json!({"role":"scout"}), &json!({}), 100).unwrap();
        service.apply_schedule("five_day");
        assert_eq!(before, service.matrix());
        assert_eq!(
            before_decision,
            select(&service.matrix(), &json!({"role":"scout"}), &json!({}), 100).unwrap()
        );
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
            .unwrap()
            .timestamp();
        let reset = chrono::DateTime::parse_from_rfc3339("2026-09-28T00:00:00Z")
            .unwrap()
            .timestamp();
        let report = json!({"providers":[{"provider":"codex","accounts":[{"account":"","windows":{"seven_day":{"used_percent":{"state":"ok","value":40},"resets_at":reset,"window_minutes":10080}}}]}]});
        let five = service.usage_report(&report, now);
        assert_eq!(
            five["providers"][0]["accounts"][0]["windows"]["seven_day"]["pace"]["curve"],
            "five_day"
        );
        service.apply_schedule("seven_day");
        let seven = service.usage_report(&report, now);
        assert_eq!(
            seven["providers"][0]["accounts"][0]["windows"]["seven_day"]["pace"]["curve"],
            "calendar"
        );
    }
    #[test]
    fn bad_managed_overlay_preserves_host_ceiling_at_boot() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("routing.yaml"),
            "ceilings:\n  default: {max_capability: cheap}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("routing-preferences.json"), "broken").unwrap();
        let service = RoutingService::open(dir.path().into()).unwrap();
        assert_eq!(
            service.matrix()["ceilings"]["default"]["max_capability"],
            "cheap"
        );
    }
    #[test]
    fn concrete_ceiling_and_exact_model() {
        let matrix = defaults();
        let mut request = json!({"provider":"claude","model":"opus[1m]","capability":"cheap"});
        let changed = sanitize(&matrix, &mut request).unwrap();
        assert!(changed.contains(&"model".to_string()));
        assert_eq!(request["effort"], "high");
        assert_eq!(request["capability"], "frontier");
        assert!(
            sanitize(
                &matrix,
                &mut json!({"provider":"claude","model":"opus","exactModel":true})
            )
            .is_err()
        );
    }
    #[test]
    fn freshness_and_unranked_ceilings_fail_closed() {
        let mut matrix = defaults();
        assert!(
            sanitize(
                &matrix,
                &mut json!({"role":"reviewer","resumeSessionId":"old"})
            )
            .is_err()
        );
        assert!(sanitize(&matrix, &mut json!({"resumeSessionId":"old"})).is_ok());
        matrix["ceilings"]["default"]["max_capability"] = "typo".into();
        assert!(sanitize(&matrix, &mut json!({})).is_err());
    }
    #[test]
    fn explicit_provider_survives_clamp_and_unknown_evidence_is_not_green() {
        let decision = select(
            &defaults(),
            &json!({"role":"judge","provider":"claude"}),
            &json!({}),
            100,
        )
        .unwrap();
        assert_eq!(decision["provider"], "claude");
        assert_eq!(decision["capability"], "frontier");
        assert_eq!(decision["capacity"]["health"], "unknown");
        assert_eq!(decision["capacity"]["effectiveHealth"], "yellow");
    }
    #[test]
    fn canonical_directory_boundaries_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("secure");
        let sibling = root.path().join("secure-old");
        std::fs::create_dir(&child).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let mut matrix = defaults();
        matrix["ceilings"][child.to_str().unwrap()] = json!({"max_capability":"cheap"});
        assert_eq!(
            ceiling(&matrix, child.to_str().unwrap()).unwrap().1["max_capability"],
            "cheap"
        );
        assert_eq!(
            ceiling(&matrix, sibling.to_str().unwrap()).unwrap().0,
            "default"
        );
        #[cfg(unix)]
        {
            let alias = root.path().join("alias");
            std::os::unix::fs::symlink(&child, &alias).unwrap();
            assert_eq!(
                ceiling(&matrix, alias.to_str().unwrap()).unwrap().1["max_capability"],
                "cheap"
            );
        }
    }
}
