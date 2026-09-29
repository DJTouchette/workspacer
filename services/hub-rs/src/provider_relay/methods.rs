//! Compatibility registration sets from Go brain/handlers.go. New local
//! handlers do not become remotely callable just because they were installed.
use super::Scope;
use std::collections::{BTreeMap, BTreeSet};
const CATALOG: &[&str] = &[
    "config.get",
    "config.reload",
    "config.getPath",
    "config.save",
    "claude.listModels",
    "claude.profiles.list",
    "claude.profiles.add",
    "claude.profiles.update",
    "claude.profiles.remove",
    "library.list",
    "library.save",
    "library.remove",
    "layouts.list",
    "layouts.save",
    "layouts.delete",
    "sessions.list",
    "sessions.load",
    "sessions.save",
    "sessions.delete",
    "claude.sessionsForDir",
    "fs.listDir",
    "fs.read",
    "fs.write",
    "fs.listEntries",
];
const FULL: &[&str] = &[
    "files.receiveUpload",
    "agents.list",
    "fleetWorkflows.request",
    "agents.spawn",
    "agents.sendMessage",
    "terminals.create",
    "claude.approve",
    "claude.answer",
    "claude.signal",
    "claude.gate",
    "claude.setPermissionMode",
    "claude.setEffort",
    "claude.setModel",
    "claude.handoffBrief",
    "claude.handoffAgentBrief",
    "sessions.transcript",
    "sessions.conversation",
    "sessions.subagentConversation",
    "sessions.snapshots",
    "sessions.snapshot",
    "sessions.terminalInput",
    "sessions.terminalResize",
    "sessions.attachTerminal",
    "sessions.terminalKeepalive",
    "sessions.detachTerminal",
    "sessions.recent",
    "agents.reportProgress",
    "agents.notifyWhen",
    "agents.close",
    "agents.orphans",
    "agents.reparent",
    "agents.dispatchReplay",
    "agents.dispatchPrepare",
    "fleet.dispatchCapabilities",
    "fleet.dispatchTargets",
    "fleet.selectDispatchModel",
    "brief.append",
    "brief.check",
    "brief.archive",
    "terminals.open",
    "providers.listModels",
    "providers.checkAll",
    "app.getCwd",
    "app.supervisorHome",
    "fs.readImage",
    "fs.watch",
    "fs.unwatch",
    "search.project",
    "git.status",
    "git.log",
    "git.diff",
    "git.numstat",
    "git.commitDiff",
    "git.commitNumstat",
    "git.stage",
    "git.unstage",
    "git.commit",
    "git.push",
    "notifications.post",
    "analytics.summary",
    "analytics.recent",
    "ui.fonts",
    "ui.asset",
];
pub(super) const TOPICS: &[&str] = &[
    "agent.snapshot",
    "agent.statusline",
    "pty.bytes.*",
    "fs.changed",
    "library.changed",
    "facade.openTerminal",
    "workflow.started",
    "workflow.completed",
    "workflow.failed",
    "workflow.agent.finished",
    "agent.dispatch.update",
];
pub(super) fn offered(scope: Scope, available: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut allowed: BTreeSet<String> = CATALOG.iter().map(|method| (*method).into()).collect();
    if scope == Scope::Full {
        allowed.extend(FULL.iter().map(|method| (*method).into()));
        let vocabulary: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/hub-vocabulary.json")).unwrap();
        allowed.extend(
            vocabulary["methods"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|method| {
                    method
                        .as_str()
                        .filter(|method| method.starts_with("desktop."))
                        .map(str::to_string)
                }),
        );
    }
    let mut methods = allowed
        .into_iter()
        .filter_map(|method| {
            let local = if method == "files.receiveUpload"
                && !available.contains(&method)
                && available.contains("files.upload")
            {
                "files.upload".into()
            } else {
                method.clone()
            };
            available.contains(&local).then_some((method, local))
        })
        .collect::<BTreeMap<_, _>>();
    methods.insert("brain.info".into(), "brain.info".into());
    methods
}
pub(super) fn event_allowed(
    topic: &str,
    accepted: &BTreeSet<String>,
    demand: &BTreeSet<String>,
) -> bool {
    if topic.starts_with("agent.conversation.") {
        return demand.contains(topic) && accepted.contains("sessions.conversation");
    }
    if !TOPICS
        .iter()
        .any(|pattern| crate::protocol::matches(pattern, topic))
    {
        return false;
    }
    static SPEC: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let spec = SPEC.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/hub-vocabulary.json")).unwrap()
    });
    let publisher = spec["topics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|spec| {
            spec["Pattern"]
                .as_str()
                .is_some_and(|pattern| crate::protocol::matches(pattern, topic))
        })
        .max_by_key(|spec| spec["Pattern"].as_str().unwrap().len())
        .and_then(|spec| spec["Publisher"].as_str())
        .unwrap_or("");
    if !publisher.is_empty() {
        accepted.contains(publisher)
    } else {
        accepted.contains("fleetWorkflows.request")
            || accepted.contains("desktop.fleetWorkflowRequest")
    }
}
