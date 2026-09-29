use super::*;
use std::collections::BTreeMap;

fn rename(key: &str, camel: bool) -> String {
    let pairs = [
        ("active_profile", "activeProfile"),
        ("mode_shifts", "modeShifts"),
        ("forecast_weights", "forecastWeights"),
        ("min_effort", "minEffort"),
        ("effort_step", "effortStep"),
        ("effort_step_capabilities", "effortStepCapabilities"),
        ("spend_down", "spendDown"),
        ("time_to_reset_minutes", "timeToResetMinutes"),
        ("min_remaining_pct", "minRemainingPct"),
        ("max_forecast_pct_of_remaining", "maxForecastPctOfRemaining"),
        ("yellow_at_used_pct", "yellowAtUsedPct"),
        ("red_at_used_pct", "redAtUsedPct"),
        ("conserve_at_ratio", "conserveAtRatio"),
        ("block_spend_down_at_ratio", "blockSpendDownAtRatio"),
        ("min_elapsed_pct", "minElapsedPct"),
        ("expected_offset_pct", "expectedOffsetPct"),
        ("seven_day", "sevenDay"),
        ("weekend_weight", "weekendWeight"),
        ("weekend_reserve_pct", "weekendReservePct"),
    ];
    pairs
        .iter()
        .find_map(|(snake, wire)| {
            if camel && key == *snake {
                Some(wire.to_string())
            } else if !camel && key == *wire {
                Some(snake.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| key.into())
}
fn fields(value: &Value, camel: bool) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (rename(k, camel), fields(v, camel)))
                .collect(),
        ),
        Value::Array(a) => a.iter().map(|v| fields(v, camel)).collect(),
        v => v.clone(),
    }
}
fn safe(matrix: &Value) -> Value {
    let mut out = json!({});
    for key in [
        "active_profile",
        "roles",
        "profiles",
        "providers",
        "modes",
        "mode_shifts",
        "thresholds",
        "forecast_weights",
    ] {
        out[rename(key, true)] = match key {
            "roles" | "forecast_weights" | "modes" => matrix[key].clone(),
            "profiles" => Value::Object(
                matrix[key]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .map(|(p, rows)| {
                        (
                            p.clone(),
                            Value::Object(
                                rows.as_object()
                                    .into_iter()
                                    .flat_map(|m| m.iter())
                                    .map(|(c, row)| (c.clone(), fields(row, true)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            ),
            "providers" => Value::Object(
                matrix[key]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .map(|(p, row)| {
                        (
                            p.clone(),
                            if row.get("enabled").is_some() {
                                json!({"enabled":row["enabled"]})
                            } else {
                                json!({})
                            },
                        )
                    })
                    .collect(),
            ),
            "mode_shifts" => Value::Object(
                matrix[key]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .map(|(mode, row)| {
                        let mut result = json!({"roles":{}});
                        for (k, v) in row.as_object().into_iter().flat_map(|m| m.iter()) {
                            if k == "effort_step" || k == "effort_step_capabilities" {
                                result[rename(k, true)] = v.clone();
                            } else {
                                result["roles"][k] = v.clone();
                            }
                        }
                        (mode.clone(), result)
                    })
                    .collect(),
            ),
            _ => fields(&matrix[key], true),
        };
    }
    out
}
fn yaml_patch(wire: &Value) -> Value {
    let mut out = json!({});
    for (key, value) in wire.as_object().into_iter().flat_map(|m| m.iter()) {
        out[rename(key, false)] = match key.as_str() {
            "roles" | "forecastWeights" | "modes" => value.clone(),
            "profiles" => Value::Object(
                value
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .map(|(p, rows)| {
                        (
                            p.clone(),
                            Value::Object(
                                rows.as_object()
                                    .into_iter()
                                    .flat_map(|m| m.iter())
                                    .map(|(c, row)| (c.clone(), fields(row, false)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            ),
            "modeShifts" => Value::Object(
                value
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .map(|(mode, row)| {
                        let mut result = row.get("roles").cloned().unwrap_or(json!({}));
                        for k in ["effortStep", "effortStepCapabilities"] {
                            if let Some(v) = row.get(k) {
                                result[rename(k, false)] = v.clone();
                            }
                        }
                        (mode.clone(), result)
                    })
                    .collect(),
            ),
            _ => fields(value, false),
        };
    }
    out
}
fn object<'a>(v: &'a Value, path: &str) -> Result<&'a serde_json::Map<String, Value>> {
    v.as_object()
        .with_context(|| format!("{path} must be an object"))
}
fn no_null(v: &Value) -> Result<()> {
    match v {
        Value::Null => bail!("null is not a preference; use reset"),
        Value::Object(m) => {
            for v in m.values() {
                no_null(v)?
            }
        }
        Value::Array(a) => {
            for v in a {
                no_null(v)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn check_row(row: &Value, alternative: bool) -> Result<()> {
    for (k, v) in object(row, "assignment")? {
        match k.as_str() {
            "provider" | "model" | "effort" | "minEffort" if v.is_string() => {}
            "fresh" | "enabled" if v.is_boolean() => {}
            "alternatives" if !alternative => {
                for a in v.as_array().context("alternatives must be array")? {
                    check_row(a, true)?;
                }
            }
            _ => bail!("unknown or mistyped assignment field {k}"),
        }
    }
    Ok(())
}
fn validate_patch(base: &Value, patch: &Value) -> Result<()> {
    no_null(patch)?;
    if serde_json::to_vec(patch)?.len() > 256 * 1024 {
        bail!("routing preferences payload too large");
    }
    let schema = safe(base);
    for (k, v) in object(patch, "preferences")? {
        match k.as_str() {
            "activeProfile" => {
                let p = v.as_str().context("activeProfile must be a string")?;
                if base["profiles"].get(p).is_none() {
                    bail!("unknown profile {p}");
                }
            }
            "profiles" => {
                for (p, rows) in object(v, "profiles")? {
                    for (c, row) in object(rows, "profile")? {
                        if base["profiles"][p].get(c).is_none() {
                            bail!("unknown assignment {p}.{c}");
                        }
                        check_row(row, false)?;
                    }
                }
            }
            "roles" => {
                for (r, c) in object(v, "roles")? {
                    if base["roles"].get(r).is_none() || rank(base, word(c)) == i64::MAX {
                        bail!("unknown role or capability {r}");
                    }
                }
            }
            "providers" => {
                for (p, row) in object(v, "providers")? {
                    if base["providers"].get(p).is_none() {
                        bail!("unknown provider {p}");
                    }
                    for (k, v) in object(row, "provider")? {
                        if k != "enabled" || !v.is_boolean() {
                            bail!("unknown provider preference {k}");
                        }
                    }
                }
            }
            "forecastWeights" => {
                for (p, w) in object(v, "forecastWeights")? {
                    if base["forecast_weights"].get(p).is_none()
                        || !w.as_f64().is_some_and(|w| (0. ..=1e6).contains(&w))
                    {
                        bail!("invalid forecast phase or weight {p}");
                    }
                }
            }
            "modes" => {
                for (k, v) in object(v, "modes")? {
                    match k.as_str() {
                        "global" => check_mode(v)?,
                        "providers" => {
                            for (p, m) in object(v, "providers")? {
                                if base["providers"].get(p).is_none() {
                                    bail!("unknown provider {p}");
                                }
                                check_mode(m)?;
                            }
                        }
                        _ => bail!("unknown mode field {k}"),
                    }
                }
            }
            "modeShifts" => {
                for (m, row) in object(v, "modeShifts")? {
                    if base["mode_shifts"].get(m).is_none() {
                        bail!("unknown mode shift {m}");
                    }
                    for (k, v) in object(row, "modeShift")? {
                        match k.as_str() {
                            "roles" => {
                                for (r, c) in object(v, "roles")? {
                                    if base["roles"].get(r).is_none()
                                        || rank(base, word(c)) == i64::MAX
                                    {
                                        bail!("unknown shift role or capability");
                                    }
                                }
                            }
                            "effortStep" if v.as_i64().is_some_and(|v| (-10..=10).contains(&v)) => {
                            }
                            "effortStepCapabilities" => {
                                for c in v
                                    .as_array()
                                    .context("effortStepCapabilities must be array")?
                                {
                                    if rank(base, word(c)) == i64::MAX {
                                        bail!("unknown step capability");
                                    }
                                }
                            }
                            _ => bail!("unknown or invalid shift field {k}"),
                        }
                    }
                }
            }
            "thresholds" => check_shape(v, &schema[k], k)?,
            _ => bail!("unknown or protected preference field {k}"),
        }
    }
    Ok(())
}
fn check_mode(v: &Value) -> Result<()> {
    if !matches!(
        v.as_str(),
        Some("auto" | "normal" | "conserve" | "spend_down")
    ) {
        bail!("mode must be auto, normal, conserve or spend_down");
    }
    Ok(())
}
fn check_shape(patch: &Value, base: &Value, path: &str) -> Result<()> {
    if let Some(map) = patch.as_object() {
        for (k, v) in map {
            let b = base
                .get(k)
                .with_context(|| format!("unknown preference {path}.{k}"))?;
            check_shape(v, b, &format!("{path}.{k}"))?;
        }
    } else if !(patch.is_number() && base.is_number()
        || patch.is_boolean() && base.is_boolean()
        || patch.is_string() && base.is_string())
    {
        bail!("invalid type at {path}");
    }
    Ok(())
}
fn effort_rung(p: &str, e: &str) -> Option<usize> {
    let ladder: &[&str] = match p {
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        "codex" => &["minimal", "low", "medium", "high", "xhigh"],
        _ => &[],
    };
    ladder.iter().position(|x| *x == e)
}
fn classified_rank(base: &Value, a: &Value) -> i64 {
    let p = word(&a["provider"]);
    let m = word(&a["model"]);
    let e = word(&a["effort"]);
    let exact = named_rank(base, p, m, e);
    if exact > 0 {
        return exact;
    }
    let Some(rung) = effort_rung(p, e) else {
        return i64::MAX;
    };
    let mut upper = usize::MAX;
    let mut best = i64::MAX;
    for rows in base["profiles"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for (c, row) in rows.as_object().into_iter().flat_map(|m| m.iter()) {
            for old in candidates(row) {
                if word(&old["provider"]) != p || model(word(&old["model"])) != model(m) {
                    continue;
                }
                if let Some(r) = effort_rung(p, word(&old["effort"])) {
                    if r >= rung && r < upper {
                        upper = r;
                        best = rank(base, c);
                    } else if r == upper {
                        best = best.max(rank(base, c));
                    }
                }
            }
        }
    }
    best
}
fn secure(base: &Value, next: &Value) -> Result<()> {
    for (p, rows) in next["profiles"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
    {
        for (c, row) in rows.as_object().into_iter().flat_map(|m| m.iter()) {
            let inherited = &base["profiles"][p][c];
            if inherited == row {
                continue;
            }
            if inherited["fresh"] == true && row["fresh"] != true {
                bail!("host freshness floor cannot be lowered for {p}.{c}");
            }
            for a in candidates(row) {
                if !["claude", "codex", "copilot", "opencode", "pi"].contains(&word(&a["provider"]))
                    || word(&a["model"]).is_empty()
                {
                    bail!("invalid provider/model in {p}.{c}");
                }
                let old = candidates(inherited).into_iter().find(|old| {
                    old["provider"] == a["provider"]
                        && old["model"] == a["model"]
                        && old["effort"] == a["effort"]
                });
                if old.is_none() && classified_rank(base, a) > rank(base, c) {
                    bail!("model/effort needs host classification at {c} or below");
                }
                let floor = old.unwrap_or(inherited);
                if (inherited["fresh"] == true || floor["fresh"] == true) && a["fresh"] != true {
                    bail!("host candidate freshness cannot be lowered");
                }
                let prior = word(&floor["min_effort"]);
                let new = word(&a["min_effort"]);
                if !prior.is_empty()
                    && prior != new
                    && !effort_rung(word(&a["provider"]), new)
                        .zip(effort_rung(word(&floor["provider"]), prior))
                        .is_some_and(|(n, o)| n >= o)
                {
                    bail!("minimum effort cannot lower the host floor");
                }
            }
        }
    }
    for r in base["roles"].as_object().into_iter().flat_map(|m| m.keys()) {
        let req = json!({"role":r});
        if freshness(base, &req).is_some() && freshness(next, &req).is_none() {
            bail!("role {r}: host freshness floor cannot be lowered");
        }
    }
    if base["thresholds"] != next["thresholds"] {
        let h = &next["thresholds"]["health"];
        let yellow = h["yellow_at_used_pct"].as_f64().unwrap_or(-1.);
        let red = h["red_at_used_pct"].as_f64().unwrap_or(-1.);
        if yellow < 0. || yellow >= red || red > 100. {
            bail!("health thresholds must satisfy 0 <= yellow < red <= 100");
        }
        let s = &next["thresholds"]["spend_down"];
        for (k, max) in [
            ("time_to_reset_minutes", 10080.),
            ("min_remaining_pct", 100.),
            ("max_forecast_pct_of_remaining", 100.),
        ] {
            if !s[k].as_f64().is_some_and(|v| v >= 0. && v <= max) {
                bail!("invalid spend-down threshold {k}");
            }
        }
    }
    Ok(())
}
fn read_optional(path: &Path) -> Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
fn revision(host: &[u8], prefs: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"routing-preferences-v1\0");
    h.update(host);
    h.update([0]);
    h.update(prefs);
    format!("{:x}", h.finalize())
}
fn compose(host: &[u8], patch: &Value) -> Result<(Value, Value)> {
    let mut base = defaults();
    if !host.is_empty() {
        let v: Value = serde_yaml::from_slice(host)?;
        object(&v, "host routing policy")?;
        merge(&mut base, &v);
    }
    validate_matrix_shape(&base)?;
    normalize(&mut base);
    validate_patch(&base, patch)?;
    let mut effective = base.clone();
    merge(&mut effective, &yaml_patch(patch));
    normalize(&mut effective);
    secure(&base, &effective)?;
    effective["_host_authority"] = base.clone();
    Ok((base, effective))
}
#[derive(Clone)]
pub(super) struct Sources {
    host: Vec<u8>,
    revision: String,
    base: Value,
    pub effective: Value,
    patch: Value,
}
pub(super) fn load(directory: &Path) -> Result<Sources> {
    let host = read_optional(&directory.join("routing.yaml"))?;
    let prefs = read_optional(&directory.join("routing-preferences.json"))?;
    let patch = if prefs.is_empty() {
        json!({})
    } else {
        let file: Value = serde_json::from_slice(&prefs)?;
        if file["schemaVersion"] != 1 {
            bail!("unsupported routing preferences schema");
        }
        file["patch"].clone()
    };
    let (base, effective) = compose(&host, &patch)?;
    Ok(Sources {
        revision: revision(&host, &prefs),
        host,
        base,
        effective,
        patch,
    })
}
fn leaves(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    if let Some(m) = v.as_object() {
        for (k, v) in m {
            leaves(
                v,
                &if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                },
                out,
            );
        }
    } else {
        out.insert(prefix.into(), v.clone());
    }
}
fn view(s: &Sources, authorized: bool) -> Value {
    let defaults = safe(&defaults());
    let inherited = safe(&s.base);
    let effective = safe(&s.effective);
    let mut def = BTreeMap::new();
    let mut host = BTreeMap::new();
    let mut managed = BTreeMap::new();
    leaves(&defaults, "", &mut def);
    leaves(&inherited, "", &mut host);
    leaves(&s.patch, "", &mut managed);
    let source: BTreeMap<_, _> = host
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                if managed.contains_key(k) {
                    "managed"
                } else if def.get(k) != Some(v) {
                    "host"
                } else {
                    "shipped"
                },
            )
        })
        .collect();
    json!({"schemaVersion":1,"revision":s.revision,"configurable":authorized,"defaults":defaults,"inherited":inherited,"overrides":s.patch,"effective":effective,"sourceByPath":source,"managedFields":managed.keys().collect::<Vec<_>>(),"catalog":{},"validation":{"valid":true,"catalogChecked":false,"catalogPending":true,"issues":[],"changedPaths":[]}})
}
pub(super) fn handle(
    service: &RoutingService,
    caller: &crate::Caller,
    method: &str,
    params: Value,
) -> Result<Value> {
    let authorized = caller.authenticated_host && caller.trusted && caller.scope == "operator";
    if method != "routing.preferences.get" && !authorized {
        bail!("{method} requires authenticated host authority and operator tier");
    }
    let mut state = service.state.lock().unwrap();
    if method == "routing.preferences.get" && !params.is_null() && params != json!({}) {
        bail!("routing.preferences.get accepts no parameters");
    }
    let source = match load(&service.directory) {
        Ok(source) => source,
        Err(error) if method == "routing.preferences.get" => {
            let last = state.preferences.clone().unwrap_or_else(|| Sources {
                host: vec![],
                revision: state.stamp.clone(),
                base: state.matrix.clone(),
                effective: state.matrix.clone(),
                patch: json!({}),
            });
            let mut out = view(&last, false);
            out["warning"] = format!("{error}; last valid policy retained").into();
            out["validation"]["valid"] = false.into();
            out["catalog"] = state.catalog.clone();
            return Ok(out);
        }
        Err(error) => return Err(error),
    };
    state.matrix = source.effective.clone();
    state.stamp = stamp(&state.matrix);
    state.error = None;
    state.checked = Instant::now();
    state.preferences = Some(source.clone());
    let mut result = json!({"status":"invalid","view":view(&source,authorized),"validation":{"valid":false,"catalogChecked":false,"catalogPending":false,"issues":[],"changedPaths":[]}});
    result["view"]["catalog"] = state.catalog.clone();
    if method == "routing.preferences.get" {
        if !params.is_null() && params != json!({}) {
            bail!("routing.preferences.get accepts no parameters");
        }
        return Ok(result["view"].clone());
    }
    for key in object(&params, "request")?.keys() {
        if key != "baseRevision" && (key != "patch" || method.ends_with("reset")) {
            bail!("unknown preference request field {key}");
        }
    }
    if word(&params["baseRevision"]) != source.revision {
        result["status"] = "conflict".into();
        return Ok(result);
    }
    let mut patch = if method.ends_with("reset") {
        json!({})
    } else {
        source.patch.clone()
    };
    if !method.ends_with("reset") {
        validate_patch(&source.base, &params["patch"])?;
        merge(&mut patch, &params["patch"]);
    }
    let (base, effective) = match compose(&source.host, &patch) {
        Ok(v) => v,
        Err(e) => {
            result["validation"]["issues"] =
                json!([{"where":"preferences","detail":e.to_string()}]);
            return Ok(result);
        }
    };
    let mut old = BTreeMap::new();
    let mut next = BTreeMap::new();
    leaves(&safe(&source.effective), "", &mut old);
    leaves(&safe(&effective), "", &mut next);
    result["validation"]["changedPaths"] = json!(
        next.iter()
            .filter(|(k, v)| old.get(*k) != Some(*v))
            .map(|(k, _)| k)
            .collect::<Vec<_>>()
    );
    // A changed model tuple needs current host-owned catalog evidence. Until the
    // sampler supplies that evidence, do not represent a guessed model as validated.
    let mut pending = false;
    if !method.ends_with("reset") {
        for (p, rows) in effective["profiles"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.iter())
        {
            for (c, row) in rows.as_object().into_iter().flat_map(|m| m.iter()) {
                let before = &source.effective["profiles"][p][c];
                for a in candidates(row) {
                    if candidates(before).iter().any(|b| {
                        ["provider", "model", "effort", "min_effort"]
                            .iter()
                            .all(|k| a[*k] == b[*k])
                    }) {
                        continue;
                    }
                    let catalog = &state.catalog[word(&a["provider"])];
                    if !matches!(word(&catalog["state"]), "available" | "unavailable")
                        || catalog["observedAt"]
                            .as_i64()
                            .is_none_or(|at| chrono::Utc::now().timestamp_millis() - at > 600_000)
                    {
                        pending = true;
                        continue;
                    }
                    let found = catalog["models"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|m| m["id"] == a["model"]);
                    let valid = found.is_some_and(|m| {
                        ["effort", "min_effort"].iter().all(|k| {
                            word(&a[*k]).is_empty()
                                || m["effortLevels"].as_array().is_none_or(|levels| {
                                    levels.is_empty() || levels.iter().any(|v| v == &a[*k])
                                })
                        })
                    });
                    if !valid {
                        result["validation"]["issues"] = json!([{"where":format!("profiles.{p}.{c}"),"detail":"model or effort unavailable in cached provider catalog"}]);
                        return Ok(result);
                    }
                }
            }
        }
    }
    if pending {
        result["validation"]["catalogPending"] = true.into();
        return Ok(result);
    }
    result["validation"]["valid"] = true.into();
    result["validation"]["catalogChecked"] = true.into();
    if method.ends_with("validate") {
        result["status"] = "valid".into();
        return Ok(result);
    }
    if !method.ends_with("save") && !method.ends_with("reset") {
        bail!("unknown preferences method");
    }
    let raw = json!({"schemaVersion":1,"patch":patch});
    use std::io::Write;
    std::fs::create_dir_all(&service.directory)?;
    let mut staged = tempfile::NamedTempFile::new_in(&service.directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        // Repair the empty random temporary file before any policy bytes land.
        // Rename preserves this protected DACL on the installed sidecar.
        drop(super::windows_audit::open(staged.path())?);
    }
    serde_json::to_writer(&mut staged, &raw)?;
    staged.write_all(b"\n")?;
    staged.as_file().sync_all()?;
    // Stage first, then recheck both inputs immediately before rename. A slow
    // fsync must not widen the stale-source acceptance window.
    if load(&service.directory)?.revision != source.revision {
        result["status"] = "conflict".into();
        return Ok(result);
    }
    staged
        .persist(service.directory.join("routing-preferences.json"))
        .map_err(|e| e.error)?;
    #[cfg(unix)]
    if let Ok(directory) = std::fs::File::open(&service.directory) {
        let _ = directory.sync_all();
    }
    let current = load(&service.directory)?;
    state.preferences = Some(current.clone());
    state.matrix = effective;
    state.stamp = stamp(&state.matrix);
    state.error = None;
    state.checked = Instant::now();
    let _ = base;
    result["status"] = "applied".into();
    result["view"] = view(&current, authorized);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broken_host_edit_keeps_last_valid_policy_visible_but_not_editable() {
        let dir = tempfile::tempdir().unwrap();
        let service = RoutingService::open(dir.path().into()).unwrap();
        let caller = crate::Caller {
            call_id: 0,
            activity_seq:0,
            federated: false,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        };
        let before = service
            .preferences(&caller, "routing.preferences.get", json!({}))
            .unwrap();
        std::fs::write(dir.path().join("routing.yaml"), "profiles: [broken").unwrap();
        let after = service
            .preferences(&caller, "routing.preferences.get", json!({}))
            .unwrap();
        assert_eq!(before["effective"], after["effective"]);
        assert_eq!(after["configurable"], false);
        assert_eq!(after["validation"]["valid"], false);
        assert!(after["warning"].is_string());
    }
    #[test]
    fn sidecar_cas_reset_and_authority() {
        let dir = tempfile::tempdir().unwrap();
        let service = RoutingService::open(dir.path().into()).unwrap();
        let mut caller = crate::Caller {
            call_id: 0,
            activity_seq:0,
            federated: false,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        };
        let initial = service
            .preferences(&caller, "routing.preferences.get", json!({}))
            .unwrap();
        let request =
            json!({"baseRevision":initial["revision"],"patch":{"modes":{"global":"conserve"}}});
        caller.authenticated_host = false;
        assert!(
            service
                .preferences(&caller, "routing.preferences.save", request.clone())
                .is_err()
        );
        caller.authenticated_host = true;
        let saved = service
            .preferences(&caller, "routing.preferences.save", request.clone())
            .unwrap();
        assert_eq!(saved["status"], "applied");
        assert_eq!(service.matrix()["modes"]["global"], "conserve");
        assert_eq!(
            service
                .preferences(&caller, "routing.preferences.save", request)
                .unwrap()["status"],
            "conflict"
        );
        let reset = service
            .preferences(
                &caller,
                "routing.preferences.reset",
                json!({"baseRevision":saved["view"]["revision"]}),
            )
            .unwrap();
        assert_eq!(reset["status"], "applied");
        assert_eq!(service.matrix()["modes"]["global"], "auto");
        assert!(!dir.path().join("routing.yaml").exists());
    }
    #[test]
    fn protected_fields_and_freshness_cannot_be_weakened() {
        let b = defaults();
        assert!(validate_patch(&b, &json!({"capabilityRanks":{"cheap":99}})).is_err());
        assert!(
            compose(
                b"",
                &json!({"profiles":{"mixed":{"reviewer":{"fresh":false}}}})
            )
            .is_err()
        );
        assert!(compose(b"",&json!({"roles":{"reviewer":"cheap"},"modeShifts":{"spend_down":{"roles":{"reviewer":"cheap"}}}})).is_err());
    }
    #[test]
    fn preference_key_conversion_does_not_rename_capabilities() {
        let b = defaults();
        let s = safe(&b);
        assert_eq!(
            s["modeShifts"]["spend_down"]["roles"]["reviewer"],
            "deep_reviewer"
        );
        assert_eq!(
            yaml_patch(&s)["mode_shifts"]["spend_down"]["reviewer"],
            "deep_reviewer"
        );
    }
}
