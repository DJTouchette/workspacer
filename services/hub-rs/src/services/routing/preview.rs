//! Read-only preview is a public projection, never the private decision record.
use super::*;

pub(super) fn validate_request(params: &Value) -> Result<()> {
    // Embedded callers have no raw JSON duplicates, but still share the null,
    // object, size and typed-field boundary enforced by the wire decoder.
    raw::validate_preferences_raw(&serde_json::to_vec(params)?)?;
    for (key, value) in params
        .as_object()
        .context("routing.preview requires an object")?
    {
        match key.as_str() {
            "ticketId" | "role" | "difficulty" | "risk" | "decisionDensity"
            | "previousProvider" | "profile" | "provider" | "preferredProvider" | "account"
            | "profileId" | "cwd" => {
                if !value.is_string() {
                    bail!("routing.preview {key} must be text");
                }
            }
            "requireIndependentFamily" => {
                if !value.is_boolean() {
                    bail!("routing.preview {key} must be a boolean");
                }
            }
            "forecastDemandBeforeResetPct" => {
                if !value.as_f64().is_some_and(f64::is_finite) {
                    bail!("routing.preview {key} must be a number");
                }
            }
            "expectedWork" => {
                for work in value
                    .as_array()
                    .context("routing.preview expectedWork must be an array")?
                {
                    for (key, value) in work
                        .as_object()
                        .context("routing.preview work must be an object")?
                    {
                        match key.as_str() {
                            "phase" if value.is_string() => (),
                            "count" if value.as_i64().is_some() => (),
                            _ => bail!("invalid routing.preview work field {key}"),
                        }
                    }
                }
            }
            _ => bail!("unknown routing.preview field {key}"),
        }
    }
    if word(&params["role"]).is_empty() {
        bail!("role is required");
    }
    Ok(())
}

pub(super) fn project(decision: &Value, observed_at: i64, usage_known: bool) -> Value {
    // Reuse the existing allowlisted nested assignment/effort-step projection;
    // copying a raw host assignment could re-expose undeclared policy fields.
    let public = events::projection(decision);
    let capped = ["capabilityRefused", "toolScopeRefused", "denied"]
        .iter()
        .any(|key| decision["ceiling"][key] == true);
    let mut output = json!({"fresh":decision["fresh"]==true,"eligible":decision["eligible"]==true,
        "capped":capped,"observedAt":observed_at,"usageState":if usage_known {"live-at-time"}else{"unknown"}});
    for key in [
        "role",
        "profile",
        "provider",
        "model",
        "effort",
        "capability",
        "baseCapability",
        "mode",
    ] {
        output[key] = json!(word(&decision[key]));
    }
    let mut reasons: Vec<Value> = public["reason"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|reason| !reason.starts_with("directory ceiling "))
        .map(|reason| json!(reason))
        .collect();
    if capped {
        reasons.push(json!("Project safety limits constrained this route"));
    }
    output["reason"] = json!(reasons);
    for key in ["fellOverFrom", "effortStep"] {
        if public[key].is_object() {
            output[key] = public[key].clone();
        }
    }
    output
}
