//! Pure launch resolution, before any token, worktree, or process is created.
//! The runtime intentionally withholds agents.spawn until its admission,
//! workflow, tool-facade and plugin preparation stages are connected.
use super::{
    models::resolve_spawn_binary,
    paths,
    profiles::{Profile, environment},
};
use crate::model_selection::{context_for_new_spawn, manager_preferences, normalize_model_input};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Plan {
    pub provider: String,
    pub session_id: String,
    pub endpoint: &'static str,
    pub request: Value,
    pub metadata: Value,
    pub full_access: bool,
    pub mcp_item_ids: Vec<String>,
}
impl Plan {
    pub fn receipt(&self, response: &Value) -> Result<Value> {
        let id = response["session_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("spawn response missing session_id"))?;
        let mut receipt = json!({"sessionId":id,"fullAccess":self.full_access});
        if self.request["first_message"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        {
            receipt["messageQueued"] =
                json!(response["first_message_queued"].as_bool().unwrap_or(false));
        }
        if let Some(routing) = self.metadata.get("routing") {
            receipt["routing"] = routing.clone();
        }
        if let Some(scrubbed) = self
            .metadata
            .get("escalationScrubbed")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        {
            receipt["escalationScrubbed"] = scrubbed.clone();
        }
        Ok(receipt)
    }
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn pinned(args: &[String], flag: &str) -> bool {
    args.iter()
        .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
}
fn profile_model(args: &[String]) -> Option<String> {
    let mut model = None;
    for (index, arg) in args.iter().enumerate() {
        let value = if arg == "--model" {
            args.get(index + 1)
                .filter(|s| !s.starts_with("--"))
                .map(String::as_str)
        } else {
            arg.strip_prefix("--model=")
        };
        if let Some(value) = value.map(str::trim).filter(|s| !s.is_empty()) {
            model = Some(value.into());
        }
    }
    model
}

/// What the host knows about a launch's recorded parent. Permission
/// preference only: no token scope or host authority is inferred from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lineage {
    /// A Fleet Manager is an ancestor (`agents.fleetFullAccess`).
    pub fleet: bool,
    /// The parent is a known session of this hub (`agents.childFullAccess`).
    pub local_child: bool,
}

impl From<bool> for Lineage {
    fn from(fleet: bool) -> Self {
        Self {
            fleet,
            local_child: false,
        }
    }
}

pub fn resolve(
    params: &Value,
    config: &Value,
    profile: Option<&Profile>,
    home: &Path,
    new_id: &str,
    lineage: impl Into<Lineage>,
) -> Result<Plan> {
    let lineage = lineage.into();
    let fleet_parent = lineage.fleet;
    if !params.is_object() {
        bail!("spawn parameters must be an object");
    }
    for key in [
        "provider",
        "transport",
        "cwd",
        "model",
        "modelIdentity",
        "effort",
        "profileId",
        "permissionMode",
        "resumeSessionId",
        "label",
        "parentSessionId",
        "role",
        "capability",
        "decisionId",
        "message",
    ] {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            bail!("{key} must be a string");
        }
    }
    for key in [
        "manager",
        "exactModel",
        "skipPermissions",
        "mcpFacade",
        "fleetFullAccess",
    ] {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            bail!("{key} must be a boolean");
        }
    }
    for key in ["cols", "rows"] {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
        {
            bail!("{key} must be a nonnegative integer");
        }
    }
    if params["exactModel"] == true
        && params["escalationScrubbed"]
            .as_array()
            .is_some_and(|fields| {
                fields.iter().any(|f| {
                    [
                        "model",
                        "modelIdentity",
                        "contextWindow",
                        "effort",
                        "capability",
                    ]
                    .iter()
                    .any(|s| f == s)
                })
            })
    {
        bail!("the hub changed the explicitly requested model; no substitute was launched");
    }
    let manager = params["manager"] == true;
    let resume = text(params, "resumeSessionId");
    let provider = if !text(params, "provider").is_empty() {
        text(params, "provider").to_owned()
    } else if manager {
        let configured = text(&config["agents"], "managerProvider").trim();
        if ["claude", "codex", "copilot", "opencode", "pi"].contains(&configured) {
            configured.into()
        } else {
            "claude".into()
        }
    } else {
        "claude".into()
    };
    if provider == "pi" {
        bail!(
            "Pi is not supported for Workspacer agent spawning: Pi has no MCP bridge, so it cannot receive the required Workspacer tools"
        );
    }
    if !["claude", "codex", "copilot", "opencode"].contains(&provider.as_str()) {
        bail!("unknown provider");
    }
    let requested_cwd = text(params, "cwd").trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let cwd = paths::canonicalize(if requested_cwd.is_empty() {
        home
    } else {
        Path::new(requested_cwd)
    })?;
    let mut model = text(params, "model").to_owned();
    let mut identity = text(params, "modelIdentity").to_owned();
    let mut effort = text(params, "effort").to_owned();
    let mut window_set = params.get("contextWindow").is_some();
    let mut window = match params.get("contextWindow") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|w| *w > 0)
                .ok_or_else(|| anyhow!("invalid-context-window"))?,
        ),
    };
    if manager && resume.is_empty() {
        let preferences = manager_preferences(&config["agents"], false)?;
        if model.trim().is_empty() && identity.trim().is_empty() {
            model = text(&preferences["managerModels"], &provider).trim().into();
        }
        if effort.trim().is_empty() {
            effort = text(&preferences["managerEfforts"], &provider)
                .trim()
                .into();
        }
        if !window_set && let Some(value) = preferences["managerContextWindows"].get(&provider) {
            window = value.as_u64();
            window_set = true;
        }
    }
    let profile = profile.filter(|p| {
        if provider == "claude" {
            p.provider.is_empty()
        } else {
            p.provider == provider
        }
    });
    let mut mcp_item_ids = match params.get("mcpItemIds") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("mcpItemIds must contain strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => bail!("mcpItemIds must be an array"),
    };
    if let Some(profile) = profile.filter(|p| p.provider.is_empty()) {
        mcp_item_ids.extend(profile.mcp_item_ids.clone());
    }
    if provider != "claude" && !mcp_item_ids.is_empty() {
        bail!("selected library MCP servers require the Claude provider");
    }
    let mut extras = profile.map(|p| p.extra_args.clone()).unwrap_or_default();
    if provider == "claude" {
        if let Some(pinned) = profile_model(&extras) {
            model = pinned;
            identity.clear();
            window = None;
        }
        if !(manager && !resume.is_empty())
            && model.trim().is_empty()
            && identity.trim().is_empty()
            && window.is_none()
        {
            model = text(&config["claude"], "defaultModel").into();
            window = config["claude"]["contextWindow"].as_u64();
        }
    } else if !window_set {
        window = context_for_new_spawn(&provider, window, !resume.is_empty());
    }
    let selection = normalize_model_input(
        &provider,
        (!model.trim().is_empty()).then_some(model.as_str()),
        (!identity.trim().is_empty()).then_some(identity.as_str()),
        window,
    )
    .map_err(|e| anyhow!("invalid {provider} model selection: {}", e.code()))?;
    if let Some(selection) = selection {
        model = selection.legacy_model;
        identity = selection.selection.model;
        window = selection.selection.context_window;
    }
    let mut skip = params["skipPermissions"].as_bool().unwrap_or(
        config["claude"]["skipPermissionsDefault"] == true
            || matches!(
                text(&config["claude"], "defaultPermissionMode"),
                "bypassPermissions" | "yolo"
            ),
    );
    let mut mode = text(params, "permissionMode").to_owned();
    if config["agents"]["fleetFullAccess"] == true && (manager || fleet_parent) {
        skip = true;
        mode.clear();
    }
    // The user's explicit "child agents start with full access" choice. It
    // is the provider's own approval policy (Claude bypassPermissions, Codex
    // full access), never a Workspacer approval gate, facade scope or token
    // grant. Only NEW launches whose recorded parent is a session of this hub
    // take it; resumes and running sessions keep what they were started with.
    if config["agents"]["childFullAccess"] == true
        && lineage.local_child
        && resume.is_empty()
        && !manager
    {
        skip = true;
        mode.clear();
    }
    let transport = if matches!(text(params, "transport"), "pty" | "stream") {
        text(params, "transport")
    } else if matches!(text(&config[&provider], "transport"), "pty" | "stream") {
        text(&config[&provider], "transport")
    } else if provider == "claude" {
        "pty"
    } else {
        "stream"
    };
    let session_id = if resume.is_empty() { new_id } else { resume };
    if session_id.is_empty() {
        bail!("launch requires a session identity");
    }
    let binary = resolve_spawn_binary(&provider, config);
    let env = profile.map(|p| environment(p, home)).unwrap_or_default();
    let managed = provider != "claude" || transport == "stream";
    let mut request = if managed {
        let mut request =
            json!({"provider":provider,"cwd":cwd,"bin":binary,"session_id":session_id,"yolo":skip});
        if provider == "claude" {
            request["permission_mode"] = json!(if mode.is_empty() { "default" } else { &mode });
        }
        if provider == "codex" {
            request["transport"] = json!(transport);
            if let Some(profile) = profile.filter(|p| !p.preset.is_empty()) {
                extras.extend(["-p".into(), profile.preset.clone()]);
            }
        }
        if matches!(provider.as_str(), "claude" | "codex") && !resume.is_empty() {
            request["resume"] = json!(resume);
        }
        if !effort.is_empty() {
            request["effort"] = json!(effort);
        }
        if !extras.is_empty() {
            request["extra_args"] = json!(extras);
        }
        request
    } else {
        let mut argv = vec![binary];
        argv.extend(extras.clone());
        for (flag, value) in [("--model", model.trim()), ("--effort", effort.trim())] {
            if !value.is_empty() && !pinned(&extras, flag) {
                argv.extend([flag.into(), value.into()]);
            }
        }
        let bypass = skip || mode == "bypassPermissions";
        if bypass && !pinned(&extras, "--dangerously-skip-permissions") {
            argv.push("--dangerously-skip-permissions".into());
        }
        if !mode.is_empty()
            && mode != "default"
            && mode != "bypassPermissions"
            && !bypass
            && !pinned(&extras, "--permission-mode")
        {
            argv.extend(["--permission-mode".into(), mode.clone()]);
        }
        if !resume.is_empty() {
            argv.extend(["--resume".into(), resume.into()]);
        } else if !pinned(&extras, "--session-id") {
            argv.extend(["--session-id".into(), session_id.into()]);
        }
        json!({"argv":argv,"cwd":cwd,"session_id":session_id,"cols":params["cols"].as_u64().filter(|n|*n>0).unwrap_or(120),"rows":params["rows"].as_u64().filter(|n|*n>0).unwrap_or(32)})
    };
    if !model.is_empty() {
        request["model"] = json!(model);
    }
    if !identity.is_empty() {
        request["model_identity"] = json!(identity);
    }
    if let Some(window) = window {
        request["context_window"] = json!(window);
    }
    if !env.is_empty() {
        request["env"] = json!(env);
    }
    if !text(params, "message").is_empty() {
        request["first_message"] = params["message"].clone();
    }
    let permission = if provider == "claude" {
        if skip {
            "bypassPermissions"
        } else if mode.is_empty() {
            "default"
        } else {
            &mode
        }
    } else if skip {
        "yolo"
    } else {
        "ask"
    };
    let mut metadata = json!({"settings":{"permissionMode":permission,"bypassAvailable":skip},"isWakeTarget":manager});
    if !model.is_empty() {
        metadata["settings"]["model"] = json!(model);
    }
    if let Some(profile) = profile {
        metadata["settings"]["profileId"] = json!(profile.id);
    }
    if !mcp_item_ids.is_empty() {
        metadata["settings"]["mcpItemIds"] = json!(mcp_item_ids);
    }
    for key in ["label", "parentSessionId"] {
        if !text(params, key).is_empty() {
            metadata[key] = params[key].clone();
        }
    }
    // Opt-in automatic naming (a client that has no name to give). A label the
    // user typed always wins, so a labelled launch never owes a title. The
    // opening request is kept, clipped, in the private journal: for providers
    // that prepend instructions to the first turn, the conversation's first
    // user message is not what the user asked.
    match params.get("autoTitle") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            if text(params, "label").trim().is_empty() {
                metadata["autoTitle"] = json!({"state":"pending",
                    "prompt":super::provider_utilities::clip_prompt(text(params, "message"))});
            }
        }
        Some(_) => bail!("autoTitle must be a boolean"),
    }
    let mut routing = json!({});
    for key in ["role", "capability", "decisionId"] {
        let value = text(params, key).trim();
        if !value.is_empty() {
            routing[key] = json!(value);
        }
    }
    if !routing.as_object().unwrap().is_empty() {
        metadata["routing"] = routing;
    }
    if let Some(scrubbed) = params
        .get("escalationScrubbed")
        .filter(|value| value.is_array())
    {
        metadata["escalationScrubbed"] = scrubbed.clone();
    }
    let mut contracts = Vec::new();
    if !manager && !text(params, "parentSessionId").trim().is_empty() {
        contracts.push(super::worker_results::ESCALATION_CONTRACT.to_owned());
    }
    if let Some(schema) = params.get("resultSchema").filter(|v| !v.is_null()) {
        contracts
            .push(super::worker_results::result_contract(schema).map_err(|error| anyhow!(error))?);
        metadata["resultSchema"] = schema.clone();
    }
    if !contracts.is_empty() {
        if managed {
            request["instructions"] = json!(contracts.join("\n\n"));
        } else {
            let argv = request["argv"].as_array_mut().unwrap();
            argv.extend([
                json!("--append-system-prompt"),
                json!(contracts.join("\n\n")),
            ]);
            super::session_facade::compose_instructions(argv)?;
        }
    }
    for key in ["taskId", "stage", "workflowStepId", "afterDispatchId"] {
        if let Some(value) = params.get(key).filter(|v| !v.is_null()) {
            if !value.is_string() {
                bail!("{key} must be a string");
            }
            if !value.as_str().unwrap().is_empty() {
                metadata[key] = value.clone();
            }
        }
    }
    Ok(Plan {
        provider,
        session_id: session_id.into(),
        endpoint: if managed {
            "/sessions/spawn-managed"
        } else {
            "/sessions/spawn"
        },
        request,
        metadata,
        full_access: skip,
        mcp_item_ids,
    })
}
