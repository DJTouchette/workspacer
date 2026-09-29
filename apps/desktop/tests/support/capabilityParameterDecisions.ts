/** Authority classification metadata. Path kinds describe caller values; they
 * do not create retired filesystem grants or provider permission clamps. */
export const pathParameters: Record<string, string> = {
  'fs.read': 'path',
  'fs.readImage': 'path',
  'fs.write': 'path',
  'fs.listEntries': 'path',
  'fs.listDir': 'path',
  'fs.watch': 'path',
  'fs.unwatch': 'path',
  'search.project': 'cwd',
  'providers.listModels': 'cwd',
  'library.list': 'cwd',
  'library.save': 'cwd',
  'library.remove': 'cwd',
  'git.diff': 'cwd',
  'brief.append': 'project',
  'brief.archive': 'project',
  'brief.check': 'project',
};
export const methodDecisions: Record<string, string> = {
  'plugins.prepareLaunch':
    "provider callback, never ambient authority: AuthorizeLaunchPreparation binds caller.ConnID and callId to a live authenticated-owner agents.spawn with the exact selected plugin id; the hub exposes only that plugin's declared launch preparation and returns no configuration or credentials. Other provider calls, stale ids, peers and swapped plugins are refused",
  'remote.tokensList':
    'remotePairingTrusted requires authenticated host authority; lists only ordinary Remote Control pairings, excludes worker/provider/session credentials.',
  'remote.tokenGetOrCreate':
    'remotePairingTrusted requires authenticated host authority; scope is an enum of view/triage/operator. Label and path are host-owned. No caller privilege grants, provider scope or paths accepted.',
  'remote.tokenRevoke':
    'remotePairingTrusted requires authenticated host authority; exact token selects only a Remote Control pairing in the configured store. Host, provider and session credentials cannot be revoked here.',
  'machine.stop':
    'Operator-only self stop through a configured power provider. No caller coordinates, paths, tokens or commands; external HTTP wake must be configured by the host.',
  'agents.dispatchPrepare':
    'Operator-only remote admission: cwd must exactly match this host discovery and canonical filesystem, provider must be authenticated here. Allocates a fresh isolated worktree under this host config root with no repository clone or setup hook. The authenticated origin credential and nonce bind its expiring, single-use lease.',
  'fleet.selectDispatchModel':
    'Operator-only explicit paired routing. The host forwards a routing request over its stored pairing connection and the remote host applies its own provider readiness and routing policy. No local credential is forwarded.',
  'routing.preferences.validate':
    'Typed allowlisted model policy only, fixed hub-owned sidecar, combined host/sidecar revision CAS. routingPreferencesTrusted requires an authenticated host-token operator connection, excluding scoped operator tokens, untokened and peer links. No caller paths, raw YAML, capability ranks, ceilings or tool scope. Host classifications and freshness floors cannot be weakened; canonical spawn enforcement remains authoritative.',
  'routing.preferences.save':
    'Typed allowlisted model policy only, fixed hub-owned sidecar, combined host/sidecar revision CAS. routingPreferencesTrusted requires an authenticated host-token operator connection, excluding scoped operator tokens, untokened and peer links. No caller paths, raw YAML, capability ranks, ceilings or tool scope. Host classifications and freshness floors cannot be weakened; canonical spawn enforcement remains authoritative.',
  'routing.preferences.reset':
    'Typed allowlisted model policy only, fixed hub-owned sidecar, combined host/sidecar revision CAS. routingPreferencesTrusted requires an authenticated host-token operator connection, excluding scoped operator tokens, untokened and peer links. No caller paths, raw YAML, capability ranks, ceilings or tool scope. Host classifications and freshness floors cannot be weakened; canonical spawn enforcement remains authoritative.',
  'routing.preview':
    'Pure routing.Select projection with bounded usage and cached availability; cwd is canonicalized only for the existing ceiling lookup. No spawn, audit id, log, event, provider launch or policy write. Security mapping and paths are redacted.',
  'agents.spawn':
    "starting an agent is a separate authorization decision — the cwd picks where a process runs, and confining it would need the spawn paths to learn root containment first (see cmd/brain's TestSpawnStaysDeliberatelyUnscoped)",
  'fleetWorkflows.request':
    'Local desktop-only declarative policy and pinned task management. The bus refuses scoped/plugin/federated connections before provider dispatch. The facade stamps callerSessionId from its authenticated session; task access additionally verifies live manager and exact project ownership. Definitions accept text/templates and typed steps only, never privileges. Revisioned locked writes do not launch agents. Headless returns unavailable.',
  'terminals.create':
    "cwd is a process working directory and holding the capability at all is the gate (as agents.spawn); the OTHER caller string, `shell`, is argv[0] and is confined by an ALLOWLIST of the host's login shells rather than by fsRoots — resolveTerminalShell in both providers",
  'terminals.open':
    "opens a VISIBLE terminal pane in the desktop and (optionally) runs `command` inside the host's DEFAULT login shell — no caller argv[0]: the shell is the host's, and the command runs under that shell's own tool/PTY rules exactly as a user-typed command would. cwd is a process working directory, so holding the capability is the gate, as terminals.create. Desktop-only (it needs the renderer to surface the pane); a headless brain has no pane to open and simply does not register it",
  'sessions.transcript':
    'cwd only selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller',
  'files.upload':
    "the landing pad for remote-client attachments (/m photos): the caller supplies BYTES and an advisory filename of which only an allowlisted image/pdf extension survives — the directory (os.TempDir()/workspacer-uploads) and basename are chosen by the hub, so there is no caller path to confine. Size-capped (24 MiB decoded) and written 0600. The file only ever ACTS if the caller also references it to an agent via agents.sendMessage — a capability the same triage tier already holds, and one whose excuse (the agent's own tool approvals are the gate) covers reading an uploaded image exactly as it covers any other path a message names",
  'nodes.wake':
    'the one parameter is an `id` SELECTING a row the hub already holds in nodes.json — every value the call acts on (the cloud app, the machine id, the API endpoint, the credential) comes from that file and none of it from the caller, so there is no path and no argv here to confine. The gate is therefore IDENTITY, exactly as jobs.* is: the handler wraps itself in nodesTrusted("nodes.wake", …), which refuses plugin tokens and the view/triage tiers, and the method appears in no scoped tier either. What makes identity the right gate rather than a path scope is what the call DOES: it starts a billable machine, so the act is unbounded in cost in a way agents.spawn — which spends the caller\'s own tokens on a machine already running — is not. Its inverse, nodes.sleep, now exists and is gated identically; that closes the cost of a FAILED wake (the hub stops what its own wake started rather than leaving it billing) and it does not soften this gate, because a wake is still a meter somebody has to notice. It is idempotent per node (a wake already in flight is reported back rather than re-issued), which is also what keeps three clients tapping one button from earning a 429 from the cloud API',
  'nodes.sleep':
    'the one parameter is an `id` SELECTING a row the hub already holds in nodes.json, exactly as nodes.wake — the cloud app, the machine id, the API endpoint and the credential all come from that file and none of it from the caller. What this method adds over its twin, and the reason it is worth its own paragraph rather than "the inverse of nodes.wake", is the two values a stop has that a start does not: the SIGNAL and the DRAIN WINDOW. Both are the supervisor\'s (nodes.Tunables), never the wire\'s, and that is authority pinned at the call site rather than declared by a caller: a caller that could name the signal could name SIGKILL, which skips claudemon\'s flush AND denies the node\'s entrypoint the chance to write /data/state/last-exit.json — the one artefact that lets the next wake tell a deliberate sleep from a crash. A stop that destroys its own evidence is not a sleep. The gate is IDENTITY, as jobs.* is: the handler wraps itself in nodesTrusted("nodes.sleep", …), which refuses plugin tokens and the view/triage tiers, and the method appears in no scoped tier either. Identity rather than confinement because of what the call DOES: it lands on a machine somebody may be typing at and ends the work in flight on it. It SAVES money, and that earns it nothing — "it only turns things off" is not a smaller act than turning them on, it is a destructive one',
  'jobs.list':
    'no params; trusted-only at the handler — the rows disclose stored shell commands and prompts, which is exactly why scoped tiers are refused',
  'jobs.upsert':
    'the job spec IS the parameter — persisted argv (shell command / spawn cwd+prompt / capability call). Trusted-only at the handler; spawn actions additionally re-enter agents.spawn over the bus and inherit its clamps',
  'jobs.propose':
    'the job spec IS the parameter, exactly as jobs.upsert — but the handler disarms it: the row is forced disabled, stamped proposedBy, given a fresh id (so it can never overwrite an approved job), and refused by jobs.run until a human clears the stamp. Same trusted-only gate; it exists because the MCP facade gives operator AGENTS a tool for this method and none for jobs.upsert, so agent-written argv can be reviewed before it is ever armed',
  'jobs.remove': 'an id naming a stored job; trusted-only at the handler',
  'jobs.run':
    'an id naming a stored job to fire now; the authority is the stored spec, the gate is the trusted-only handler',
  'jobs.history':
    'an id naming a stored job; returns run records (output tails included), trusted-only at the handler',
  'routing.select':
    "the caller supplies a WORK DESCRIPTION — a role, a ticket id, a difficulty/risk/decision-density classification, an optional provider and account, and a cwd — and gets back a (provider, model, effort, capability, mode) with the reasons. Nothing it carries reaches a sink: `cwd` is not opened, joined or statted (it selects which per-directory entry of the hub's OWN routing.yaml applies, exactly as nodes.wake's `id` selects a row of the hub's own nodes.json), `provider`/`account`/`profileId` SELECT rows of the usage document claudemon already serves, and `role` selects a key of the matrix file. The call starts nothing and writes nothing — it is a table lookup over a hub-owned file plus one read of claudemon's /usage/report, and every action anybody takes on the answer happens through a SEPARATE, refusable capability (agents.spawn), which is why an advisory answer is not an authorization. What it DISCLOSES is model names, capability names, a routing mode and a per-window utilization percentage for the caller's own subscription: no credential, no path, no argv. Deliberately NOT in any scoped tier (authtoken viewMethods/triageMethods) — §18 and §40 of the design are explicit that only the supervisor/control plane invokes a routing decision, and the tier lists are how this repo says that; ScopeOperator.Methods() returns [\"*\"], so the Fleet Manager gets it and a phone token does not",
  'claude.sessionsForDir':
    "cwd is encoded into a ~/.claude/projects slug by the provider (claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component); the caller's string is never opened as a path",
  'replay.open':
    'canonicalizes the caller-selected repository before cutting a disposable worktree; authenticated path access is ambient rather than workspace-root confined',
  'replay.read':
    'the path is a repo-relative coordinate inside a worktree the replay service itself created and keyed by sessionId; containment is structural (resolveInside), while guardReplaySession re-canonicalizes the recorded origin',
  'replay.diff':
    'same as replay.read — a coordinate inside a service-owned worktree, not an arbitrary host path',
  'replay.seek':
    'the ops carry a file_path and content, but both are re-anchored inside the service-owned worktree by containInWorktree (timelineReplayService), which resolves per component and writes the RESULT; guardReplaySession also re-canonicalizes the recorded origin',
  'claude.setPermissionMode':
    'mode is provider permission configuration, not a path. Authenticated agents may select any provider-supported mode without a second Workspacer grant',
  'claude.setEffort':
    'effort reaches a live claude session as the message `/effort <level>` (applyLiveEffort), i.e. exactly the reach agents.sendMessage already has and no more; there is no path and nothing to confine, so holding the capability is the gate',
  'claude.setModel':
    '`model` is normalized into a canonical identity/window pair before the daemon owns the switch. Managed drivers receive that structure; Claude PTY alone receives a daemon-built `/model <legacy compatibility spelling>` command derived from the validated pair, with control bytes refused before construction. PTY effort is refused because that command cannot deliver it. The answer distinguishes queued from accepted delivery, and only owner-authored requested_selection is durable',
  'claude.answer':
    "types the answer into the session's PTY, which is sessions.terminalInput's primitive under another name — see the per-param decisions, which say so rather than implying this method is narrower than it is",
  'agents.reportProgress':
    "`note` is prompt text for an agent that is already running — agents.sendMessage's reach — and the containment is that the caller cannot choose WHO reads it. There is no recipient param: the caller supplies `callerSessionId`, the host looks that session up in its own store and delivers to its parentSessionId or refuses, so the only pair this can ever connect is (a tracked session, whatever dispatched it). `callerSessionId` is not a caller value on the path an agent actually uses either — the MCP facade stamps it from the per-request token record's `session:<id>` label, and the hub bus deletes it from every untrusted caller's params (sanitizeReportProgressParams), so a scoped or plugin token cannot name a session at all and lands on the no-identity refusal. Bounded in volume as well as reach: one line, flattened, capped at 500 chars, one per 60s, 20 per session for life",
  'agents.sendMessage':
    "text is a prompt for an agent that is already running; there is no path to confine, so holding the capability is the gate. The older wording — 'the agent's own tool approvals are the gate' — named a bound that only holds for a caller which cannot also RESOLVE those approvals, and the triage tier holds claude.approve. See Compositions(): agents.sendMessage + claude.approve is recorded, accepted for triage, and machine-checked against every other tier",
  'git.status':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.log':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.numstat':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.commitDiff':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.commitNumstat':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.stage':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.unstage':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.commit':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'git.push':
    'guardGitCwd canonicalizes cwd before git opens it; authenticated agent/plugin path access is ambient',
  'sessions.load':
    "filename is a bare basename resolved and confined to <configDir>/sessions by both providers (paths::selected_path / resolveWithinSessionsDir); pinned by the corpus's sessionFilenames block",
  'sessions.save':
    "same as sessions.load: the filename is derived from the session name by the provider's slug and re-checked by the same resolver",
  'sessions.delete': 'same as sessions.load',
  'layouts.save':
    'id is a bare name slugged into <configDir>/layouts/<slug>.yaml and re-contained there by both providers (layoutFilePath / layoutService), never a caller-chosen directory',
  'layouts.delete': 'same as layouts.save',
  'config.save':
    "takes no path; the config file is the provider's own and every key that the host later hands to a process — agents.binaries, claude.profiles, terminal.shell, terminal.shells, editor.terminalCommand, scripts — is stripped from a bus write by dropHostTrusted. That list is held equal to contracts/host-trusted-config-cases.json, so a newly dangerous key cannot be classified on one side only",
  'claude.profiles.remove':
    'id selects a row in the single claude-profiles.json; it is never joined into a path, so there is nothing to confine',
  'notifications.post':
    "the only dangerous param is `url`, which the host opens on click through openExternalUrl's scheme allowlist — the same gate the renderer's open-external path uses",
  'claude.profiles.add':
    'Provider configDir, extraArgs and mcpItemIds persist as explicit authenticated operator choices. These may affect later provider execution; no obsolete workspace-root grant or bypass-profile scrub is applied.',
  'claude.profiles.update':
    'Updates the same configDir, extraArgs and mcpItemIds choices as claude.profiles.add under authenticated operator authority. Profile identity is a selected row, not a caller-selected store filename.',
  'claude.approve':
    "resolves a tool-approval prompt for a session that is ALREADY running: there is no path and no subtree to confine, so holding the capability is the gate, at the same level as sessions.terminalInput. NOTE that this is the RESOLVER of the approvals agents.sendMessage's own excuse rests on, that claude.gate can arm the parked-hook path it answers, and that the sessionId is ownership-checked on neither provider, so it reaches the local user's own agent too",
  'claude.gate':
    'turns the PreToolUse parking gate on or off for a session that is already running. Same shape as claude.approve — no path, no subtree, holding it is the gate — and the two are meant to be read together: gate ON parks every tool call, and claude.approve is what then releases them',
  'claude.signal':
    'the signal name is deserialized into a three-variant enum by claudemon (protocol.rs Signal) before anything is sent, so the caller chooses among interrupt/stop/kill and cannot compose a value; the sessionId is not ownership-checked on either provider',
  'claude.handoffBrief':
    'writes a deterministic brief the PROVIDER composes into ~/.workspacer/handoffs/<generated>.md — the caller supplies a session id, never the filename and never the directory — so there is no caller path to confine',
  'claude.handoffAgentBrief':
    'same as claude.handoffBrief, plus it injects the resulting read-this instruction into a live agent; that half is the agents.sendMessage primitive and is bounded the same way',
  'sessions.attachTerminal':
    "binds a PTY stream to the caller's connection. No path and no subtree: what it grants is the OUTPUT side of sessions.terminalInput, so holding it is the gate at the same level",
  'layout.set':
    'The shared workspace document can be adopted by a later UI. Non-trusted writers have boot-agent escalation fields and executable pane metadata scrubbed at this persisted boundary; ordinary explicit agents.spawn provider permission choices retain their separate ambient contract.',
  'push.subscribe':
    "the endpoint is a URL the hub itself POSTs to, so it is a request the host makes on the caller's behalf rather than a path or an argv. This reason used to stop at what the ENDPOINT learns (the payload is encrypted to the subscription's own keys) and said nothing about what the HOST is made to DO — and the host's network position is Tailscale-reachable, loopback-reachable and cloud-metadata-reachable, for a TRIAGE tier holding no fetch, no exec, no fs and no config capability, on a trigger that same tier can pull at will. Bounded now by validatePushEndpoint (https only, no loopback/private/link-local/unique-local host: the shape a browser PushManager actually produces), plus RPCSubscribeAs recording WHICH credential asked so revoking a phone's token revokes its notifications",
  'push.unsubscribe':
    'the endpoint selects a stored subscription row to delete — the narrowing direction, and never joined into a path',
  'push.revoke':
    'deletes a stored subscription by id. Operator-only by construction (absent from both scoped tiers), and narrowing',
  'usage.setPacingSchedule':
    'the only caller value is `schedule`, a CLOSED two-word enum (five_day | seven_day) validated by usageprefs.Validate before anything is written; an unrecognised word is an error and stores nothing. It is never opened as a path, never becomes argv, and never reaches an interpreter: it SELECTS which of two curve words internal/limits already implements is used for the seven-day window of the next usage.report. There is no path to confine, so the gate is IDENTITY — usagePrefsTrusted("usage.setPacingSchedule", …) refuses plugin tokens and the view/triage tiers, exactly as jobsTrusted does — and the method appears in no scoped tier. What it emphatically does NOT touch is routing: routing.select reads Matrix.PaceConfig() straight off routing.yaml and never consults this preference, so nothing here can move a spawn ceiling, a model choice or a routing mode',
  'sessions.terminalInput':
    "writes raw bytes into an existing session's PTY: there is no path and no subtree to confine, so holding the capability is the gate, exactly as for terminals.create. NOTE that this makes terminals.create's shell allowlist a boundary only against callers that do not ALSO hold this method — allowlisted /bin/bash plus typed bytes is full argv[0] freedom — and that the sessionId is not ownership-checked on either provider, so it reaches the local user's own agent PTY too",
  'desktop.worktreeInfo':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.worktreeCreate':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.worktreeRemove':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.pricingGetRates':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.pricingSaveOverrides':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesAccounts':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesLoginStatus':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesAddAccount':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.toolsStatus':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.fleetReviewRead':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.fleetReviewForget':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.taskInspectorEdit':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.taskInspectorOpen':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.dispatchHistoryRead':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.htmlCardReadDiff':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.fleetWorkflowRequest':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.managerRequestPrepare':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.managerRequestSend':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.loadBriefBoard':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.moveBriefCard':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesAdd':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesUpdate':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.claudeProfilesRemove':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.saveConfig':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.agentSuggestTitle':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.providerReadiness':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.agentRuntimeStatus':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.keepWarmHeartbeats':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.workflowAgentTranscript':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.workflowAgentConversation':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.installUiFont':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.downloadProjectIcon':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.managerReplacement':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.sessionGrantReconcile':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.readFileBytes':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'desktop.filePickerList':
    'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
  'ui.asset':
    'Fixed cache bytes are selected by file basename. canonicalize resolves the selected object under its cache root before the bounded descriptor read, which writes neither configuration nor authority.',
  'files.receiveUpload':
    "authenticatedUploadReceiver permits only the hub owner's forwarding call. The same fixed extension/size policy as files.upload applies; bytes become active only if explicitly referenced in an agent message under that tier's existing approval contract. It changes no grant or filesystem root.",
  'federation.peersConfig':
    'peerConfigTrusted requires authenticated local owner identity. The saved peer credentials and dispatch selection can authorize later peer calls, but this write is restricted to the same server owner who owns that authority; providers, scoped operators, plugins and peer links cannot acquire it.',
  'federation.savePeersConfig':
    'peerConfigTrusted requires authenticated local owner identity. The saved peer credentials and dispatch selection can authorize later peer calls, but this write is restricted to the same server owner who owns that authority; providers, scoped operators, plugins and peer links cannot acquire it.',
  'remote.tailscaleInfo':
    "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
  'remote.tailscaleServe':
    "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
  'remote.setSharing':
    "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
};
export const inertMethods: Record<string, string> = {
  'fleet.dispatchTargets':
    'Read-only discovery of explicitly enabled paired or linked worker targets. Returns remote protocol, actual provider readiness and remote repository choices, never the pairing credential.',
  'fleet.dispatches':
    'Read-only local durable dispatch records. No caller-selected recipient, process, path or credential is accepted; records describe already admitted work.',
  'routing.preferences.get':
    'No arguments; returns only typed safe policy, defaults, inherited values, source badges, cached catalog and opaque revision. No security fields, paths or writes.',
  'usage.report':
    "no parameters; reads only the hub-owned usage watcher, installed pace configuration and the hub's own pacing-schedule preference, returning account identity, provenance, quota windows and sampled pace. No credentials, tokens, spend, config, decisions, probing or writes",
  'usage.pacingSchedule':
    'no parameters; returns the hub\'s own Overview pacing-schedule preference (five_day, seven_day, or empty for "nobody has chosen") and whether this hub can store one. It reads a single enum out of the hub\'s own 0600 preference file and discloses no path, no credential, no usage figure and no account. The WRITE half, usage.setPacingSchedule, is a separate method behind a separate trusted-only gate',
  'agents.list': 'no caller params at all; it returns the session rows the provider already holds',
  'agents.close':
    "one sessionId selecting an existing session row, and the effect is REMOVAL — the narrowing direction, like sessions.detachTerminal and push.unsubscribe. It composes no path and no argv. Its one side effect beyond forgetting the row is claudemonSessionClient.close (viewers stopped + SIGTERM), which is exactly claude.signal's own reach and is skipped entirely for a row that had already ended. It cannot be aimed at a WORKING session at all: the provider refuses one, because hiding a running agent from list_agents while it kept spending is the only outcome worse than the lingering row this replaces",
  'agents.reparent':
    "two session ids selecting existing session rows, and NOTHING else — no path, no argv, no caller text. The effect is internal routing: it re-points the `parentSessionId` field the provider itself reads when it decides which manager to wake, from a retiring manager to its successor, and every message that later travels that route is composed by the host from the worker's own output (buildFleetMessage), exactly as agents.notifyWhen's is. Its one disclosure — the successor now receives reports about workers it did not dispatch — is available to the same operator tier through sessions.conversation already. The provider refuses a destination no wake can reach (unknown, ended, or not a supervisor), so it cannot be used to silence a fleet either",
  'agents.orphans':
    "no caller params at all. It returns the DEAD parents that still have live children — the `fromSessionId` agents.reparent needs when the manager it replaces crashed and wrote no handoff file. What it discloses about each is a label, a cwd, a time of death and the ids of its live children, and every one of those is on the caller's own agents.list already for a session that is still running: the tombstone only makes them outlive the row by as long as something it dispatched is still alive. It performs no move and names no destination — an id it hands back is an ARGUMENT for a separate, refusable call, which is precisely why the discovery is a read rather than a no-argument mode on agents.reparent (the host would have to guess which dead manager the caller is replacing, and a wrong guess re-points a live worker's wakes silently)",
  'agents.dispatchReplay':
    "one opaque dispatch id, minted by the hub that DISPATCHED the work and known only to the two machines. It selects a record this node already holds and RE-PUBLISHES the terminal update this node itself composed, with its original sequence number — a re-send, never a re-derivation, so an origin whose federation link was down reconciles instead of polling and a replay of an update that already arrived is deduped into a no-op. Nothing in the answer comes from the caller: the update body is host-composed from the worker's own output (the same finishEntry the local wake path uses), and the three states it discloses are 'running', 'replayed' and 'unknown' about an id the caller already had. It starts nothing, moves nothing and names no recipient — a peer cannot address a callback at a dispatch it was not given the id for, and an unknown id is answered rather than guessed at",
  'fleet.dispatchCapabilities':
    "no caller params at all. It is the READINESS answer a machine reads before dispatching work here: the remote-dispatch protocol number, whether this node can execute dispatched work at all, its registration scope, each managed provider's installed/signed-in state on THIS machine, and the absolute directories on THIS filesystem a dispatched worker may be pointed at (the projects this node has configured plus the cwds its live agents already hold). Every field is measured locally, which is the entire point — the alternative is a dispatching machine inferring a peer's Claude login from its own and sending work to a harness that is installed but not authenticated. It reads credential files for PRESENCE only and returns no token, no account, no address and no path outside the two committed sources; each cwd it names is already on that node's own agents.list rows",
  'agents.notifyWhen':
    "two session ids selecting existing rows plus four NUMBERS (tokens, usd, idleSeconds, contextUsedPct), each finite/range-checked before it is stored. Nothing composes a path, an argv or a query, and the call starts nothing: it records an intention to send a message LATER, and the message body is composed entirely by the host (buildFleetMessage over the provider's own snapshot fields) with no caller text in it. Everything it can ever report — cumulative throughput, cost, idle time, runtime-confirmed active context occupancy — is already in the sessions.snapshot the VIEW tier reads, so it discloses nothing new; what it removes is the caller's need to poll for it",
  'analytics.recent':
    'the only params are a row limit and a time window, both coerced to numbers before they reach the store; nothing composes a path, an argv or a query the host runs',
  'analytics.summary': 'same as analytics.recent — numeric window only',
  'app.getCwd': "no params; returns the provider's own working directory",
  'remote.pairingInfo':
    'No params; returns caller scope and whether authenticated host pairing administration is available. No credentials.',
  'machine.power':
    'No parameters; reports this host power capabilities without credentials or infrastructure coordinates.',
  'brain.info':
    "no params. It returns the brain's own registration scope, plus — on a REMOTE NODE only — the node id that brain was started with (WKS_NODE_ID) and the node's own record of how its PREVIOUS run ended (reason, exit code, timestamp, read from a file the node's entrypoint writes; the record's bootId and machine id are projected out). It exists so `workspacer status` can ask whether a brain is on the bus at all — a question the previous probe (app.getCwd, which the DESKTOP registers) answered wrong whenever the desktop was running — and so the hub's node registry can ask the same question on a timer. Nothing in the answer comes from the caller, and nothing in it is a credential: the node id names a row in the hub's OWN nodes.json and grants nothing (an unrecognised name is ignored rather than trusted), and the exit record is a process exit status and a reason string. All three fields are absent everywhere that is not a node",
  'app.supervisorHome':
    'no params. It DOES create ~/.workspacer and a README there, but both are fixed literals the provider composes — no part of the location comes from the caller',
  'claude.listModels': "no params; the answer is the provider's own model catalog",
  'claude.profiles.list':
    'no params; the mutating siblings (add/update/remove) carry their own decisions',
  'config.get':
    'no params. It hands back the whole config document, which is a disclosure decision rather than a confinement one: the keys a bus caller must not WRITE are the host-trusted list, and the keys it must not READ would be a different mechanism (there are none today — the config holds no credential; those live in remote-token, tokens.json and the plugin .settings.json files, none of which is in config.yaml)',
  'config.getPath':
    "no params; returns the config file's location, which the caller can already derive from the platform rules",
  'config.reload': "no params; re-reads the provider's own config file from disk",
  'layouts.list':
    'no params; the entry names come from a readdir of the layouts store and are re-contained by the store resolver before anything is opened',
  'providers.checkAll':
    "no params; it stats a fixed set of provider binary names against the process's own PATH, and the answer is a boolean per provider rather than a path the caller chose",
  'agents.summarizeStatus':
    'sessionId selects a visible session on the owning desktop hub; the daemon projects a bounded data-only source and the no-tools completion receives no paths or argv from the caller',
  'sessions.conversation':
    'sessionId selects an existing session row; it is never joined into a path (the transcript location is derived by the provider) and never becomes argv',
  'sessions.subagentConversation':
    'sessionId plus agentId select an existing provider-owned child thread already attached to that session; neither value is joined into a path by the hub, and claudemon validates the child belongs to the parent before reading its rollout',
  'sessions.detachTerminal':
    'sessionId only, and the effect is to STOP streaming — the narrowing direction. The attach half carries its own decision',
  'sessions.list':
    'no params; entry names come from a readdir of the sessions store and are re-contained by the store resolver',
  'sessions.recent': 'a numeric limit over rows the provider already holds',
  'sessions.snapshot':
    'sessionId selects an existing session row; the snapshot is assembled by the provider from state it already holds',
  'sessions.snapshots':
    'no params at all; it returns the whole snapshot set the provider already holds in memory, with nothing composed from a caller string',
  'sessions.terminalKeepalive': 'sessionId only; it refreshes an idle timer and moves no bytes',
  'sessions.terminalResize':
    'sessionId plus cols/rows, both coerced to integers before they reach the PTY ioctl; there is no string the host acts on',
  'replay.close':
    "an opaque handle the provider minted; closing it releases the provider's own reader",
  'layout.get':
    "no params; returns the shared workspace document the hub already holds. A disclosure decision rather than a confinement one, like config.get: the document names agent cwds and pane URLs, and the one credential that ever rode in it (a plugin pane's busToken) is redacted on the way in and out",
  'fleet.quiescence':
    "no params at all. It reports whether this machine's fleet is at rest, and a named blocker per reason when it is not. Every value in the answer is derived by the hub from state it already holds or already serves — session rows (the same ones agents.list and sessions.snapshots return to this tier), the job SCHEDULE (times and action kinds, never a spec: the argv stays behind the trusted-only jobs.* RPCs), the peer names federation.peers already discloses, and a description of each live bus connection that names no credential and no address. It composes nothing: the answer is a reading, and every action anyone takes on it happens outside this process. Admitted to the VIEW tier (authtoken viewMethods) for the same reason federation.peers is — the phone and the web renderer can already derive most of it by polling the snapshot feed they receive anyway, one call at a time",
  'nodes.list':
    "no params. It returns the hub's REMOTE NODE registry — one row per node with a label, a state (available / waking / stopped / unreachable), the time it entered that state, the time its provider last answered, a sentence of detail, whether this hub can wake it, and a count of consecutive failed wakes. It composes nothing and it is a projection rather than a redaction: nodes.NodeView is a separate struct built by naming what goes IN, so the registry record's credential-bearing half (the cloud API token, the path of the file holding it, the app name, the machine id, the API endpoint) is absent by construction rather than by a strip list that re-opens itself every time the record grows a field. Admitted to the VIEW tier (authtoken viewMethods) for the reason federation.peers is: it is the tombstone signal for a target that has gone quiet, and withholding it makes a sleeping node read to the user as a broken one — which is the exact failure the four-state model exists to prevent",
  'federation.peers':
    "no params; returns each configured peer hub's name, connected bit, and last-seen timestamp. Registered only when federation is configured. A disclosure decision: the peer NAMES are already stamped on every forwarded agent.* event the same callers receive, and the connected bit is the tombstone signal hub.peer.* broadcasts anyway. Admitted to the VIEW tier (authtoken viewMethods) so the /m PWA and web renderer can seed the federated fleet — the same tier already receives the stamped events it explains",
  'plugins.tools':
    "no params; returns facade-tool metadata (tool name/description/schema + the plugin bus method each forwards to) for enabled plugins. A disclosure decision, not a confinement one: the MCP facade reads it over its trusted connection and exposes the enabled catalog ambiently to authenticated agent sessions. Provider registration remains confined to each plugin's own namespace",
  'push.key':
    'no params; returns the VAPID PUBLIC key, which every subscriber needs and which discloses nothing',
  'push.list':
    'no params; lists stored subscriptions. Operator-only by construction — it appears in neither scoped tier',
  'push.test':
    'no params; sends one canned notification to every registered subscription so a phone can answer "is push reaching me at all" without reading hub logs. Nothing about the message is caller-supplied — title and body are literals in RPCTest — so there is no text a caller can put on someone\'s lock screen, which is the shape that made a forged agent.snapshot worth closing. It is a SEND trigger available to the triage tier, bounded by the same recipient set every other push already goes to: subscriptions this hub stored, still-valid credential, endpoint already validated by validatePushEndpoint at subscribe time. The tier that may subscribe may already provoke real pushes by approving or answering, so this adds no reach it lacked — only a way to test it deliberately',
  'ui.fonts': 'No caller parameters; reads uploaded font labels from the fixed display cache.',
  'remote.sharingInfo':
    'No caller parameters; reports configured sharing state and caller manageability without changing the listener.',
};
export const parameterDecisions: Record<
  string,
  Record<string, { kind: string; reason: string }>
> = {
  'fleet.selectDispatchModel': {
    cwd: {
      kind: 'path',
      reason:
        'remote routing input sent only to the explicitly paired host; no desktop filesystem operation or credential propagation',
    },
  },
  'agents.dispatchPrepare': {
    cwd: {
      kind: 'path',
      reason:
        'exact canonical remote repository selected from this host discovery; worktree allocation is remote and never falls back',
    },
    remoteOrigin: {
      kind: 'id',
      reason:
        'nonce and protocol with ownerKey replaced from the authenticated connection; single-use lease cannot be claimed by a different credential',
    },
  },
  'agents.spawn': {
    launchIntegrationId: {
      kind: 'id',
      reason:
        'selects an installed, consented launch integration. Bus owner identity is required; native prepares locally and headless callbacks are bound to that same pending spawn and plugin id. Untrusted boot documents cannot plant it',
    },
    launchIntegrationGranted: {
      kind: 'permission',
      reason:
        'hub-only stamp: sanitizeSpawnParams deletes incoming copies and adds true only for an authenticated local owner selecting an integration. Both providers refuse unstamped selections; headless preparation independently requires the active owner spawn callback',
    },
    dispatchOwnerSessionId: {
      kind: 'id',
      reason: 'private facade stamp from the session credential; bus strips non-host copies',
    },
    retrySourceSessionId: {
      kind: 'id',
      reason:
        'private respawn_with stamp, excluded from spawn_agent schema; bus strips non-host copies and desktop validates known source owner/project',
    },
    cwd: {
      kind: 'path',
      reason:
        'the working directory of a process the caller is already authorized to start; holding agents.spawn is the gate, and confining it needs the spawn paths to learn root containment first (TestSpawnStaysDeliberatelyUnscoped)',
    },
    mcpItemIds: {
      kind: 'id',
      reason:
        'selects Library MCP entries merged beside the automatic authenticated Workspacer facade. Holding agents.spawn is the trust boundary; Workspacer no longer applies a second per-session grant',
    },
    profileId: {
      kind: 'id',
      reason:
        'selects a stored provider profile whose config root and provider arguments flow to the launched harness. Holding agents.spawn is the trust boundary; legacy profile grant stamps are ignored',
    },
    skipPermissions: {
      kind: 'permission',
      reason:
        'provider permission configuration forwarded by an authenticated agents.spawn caller; no separate Workspacer grant or clamp',
    },
    permissionMode: {
      kind: 'permission',
      reason:
        'provider-native permission mode forwarded by an authenticated agents.spawn caller; no separate Workspacer grant or clamp',
    },
    executionTarget: {
      kind: 'id',
      reason:
        'explicit paired route handled on local desktop after authenticated manager and workflow admission; unknown targets refuse without fallback',
    },
    remoteCwd: {
      kind: 'path',
      reason:
        'remote filesystem choice checked by remote discovery and admission, never opened or granted on the desktop',
    },
    remoteOrigin: {
      kind: 'id',
      reason:
        "router-stamped remote-dispatch provenance: a protocol number and an opaque per-dispatch id, and nothing else. It is not caller-settable at all — internal/bus sanitizeSpawnParams DELETES the key from every non-federated caller, so on the dispatching machine nobody can pre-seed the id its own router is about to mint, and on the executing machine no local client can manufacture a dispatch whose callbacks would be addressed at another machine's manager. It grants nothing: the federation link token remains the ceiling on everything the spawn may do, and the id's only effect is that the worker's host-composed progress/blocked/finished bullets are published back over the link the origin opened instead of dropped",
    },
    effort: {
      kind: 'shell',
      reason:
        "a reasoning-effort level handed to the daemon at spawn (codex model_reasoning_effort, claude's /effort); it selects among the provider's own levels and never becomes argv the caller composes",
    },
  },
  'terminals.create': {
    cwd: {
      kind: 'path',
      reason: "a process working directory, as agents.spawn's — holding the capability is the gate",
    },
    shell: {
      kind: 'executable',
      reason:
        "argv[0] of a host process, taken from a bus caller. There is no subtree to confine it to that the same caller cannot also fill in with fs.write, so it is an ALLOWLIST of the host's login shells instead: resolveTerminalShell in both providers (cmd/brain/shellallow.go, lib/shellAllowlist.ts)",
    },
  },
  'terminals.open': {
    cwd: {
      kind: 'path',
      reason:
        "a process working directory for the visible terminal pane, as terminals.create's — holding the capability is the gate",
    },
    command: {
      kind: 'executable',
      reason:
        'Text delivered in a visible-terminal intent for execution inside a UI-selected host default shell. It is never used as argv[0]; unsupported UI owners report unavailable without creating a hidden shell.',
    },
  },
  'layout.set': {
    data: {
      kind: 'permission',
      reason:
        'A saved workspace document that can later restore agents. Non-trusted writes scrub boot-agent escalation fields and executable pane metadata before persistence; its embedded directory strings are descriptions rather than a direct filesystem operation.',
    },
  },
  'routing.preview': {
    cwd: {
      kind: 'path',
      reason:
        'Canonicalized only to select the existing trusted directory ceiling; no caller path is written or returned.',
    },
  },
  'routing.select': {
    cwd: {
      kind: 'path',
      reason:
        'Canonicalized existing path used to select a directory row from the trusted routing ceiling matrix. The answer is only provider/model/effort information; spawn performs its own current cwd and admission validation.',
    },
    effort: {
      kind: 'shell',
      reason:
        "not a caller value at all on this method — it is REPORTED, not accepted: the level comes out of routing.yaml's profile table and is validated at matrix-load time against the provider's own ladder (ValidateAgainstCatalog). It appears here because the response carries the name and the vocabulary scans on names",
    },
  },
  'files.upload': {
    name: {
      kind: 'filename',
      reason:
        'advisory only: the basename is discarded and the extension must be on the image/pdf allowlist; the written path is hub-composed (os.TempDir()/workspacer-uploads/m-<ts>-<rand>.<ext>)',
    },
  },
  'push.subscribe': {
    endpoint: {
      kind: 'url',
      reason:
        "a push-service URL the HOST posts to on the caller's behalf — a NETWORK SINK, not a string, and stored by one call for a different subsystem (push.Watch) to use later. Constrained by validatePushEndpoint to https at a non-private host; the payload is encrypted to the subscription's own keys, and RPCSubscribeAs records which credential asked so a revoked token stops being notified",
    },
  },
  'push.unsubscribe': {
    endpoint: {
      kind: 'id',
      reason:
        'selects a stored subscription row to delete — narrowing, and never joined into a path',
    },
  },
  'push.revoke': {
    id: {
      kind: 'id',
      reason:
        'selects a stored subscription row to delete. Operator-only by construction: push.revoke is in neither scoped tier',
    },
  },
  'sessions.transcript': {
    cwd: {
      kind: 'path',
      reason:
        'selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller',
    },
  },
  'claude.sessionsForDir': {
    cwd: {
      kind: 'path',
      reason:
        "encoded into a ~/.claude/projects slug by claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component; the caller's string is never opened as a path",
    },
  },
  'replay.open': {
    cwd: {
      kind: 'path',
      reason:
        'canonicalized by the provider before it cuts a disposable worktree from the selected repository; authenticated path access is ambient',
    },
  },
  'replay.read': {
    path: {
      kind: 'path',
      reason:
        'a repo-relative coordinate inside a worktree the replay service itself created and keyed by sessionId; containment is structural (resolveInside)',
    },
  },
  'replay.diff': {
    path: {
      kind: 'path',
      reason: 'same as replay.read — a coordinate inside a service-owned worktree, not a host path',
    },
  },
  'replay.seek': {
    ops: {
      kind: 'path',
      reason:
        'the wrapper the scanners can see: each op carries input.file_path and input.content, and the service re-anchors the path inside the worktree it owns — path.relative against the repo root, then containInWorktree, which resolves per component and writes the RESULT rather than the join. Written down here because the path is a level deeper than any scan reaches, so its confinement can only be pinned by naming the helper',
    },
  },
  'claude.setPermissionMode': {
    mode: {
      kind: 'permission',
      reason:
        "selects the running provider's supported permission mode; authenticated agent access is the trust boundary and Workspacer applies no separate grant",
    },
  },
  'claude.setEffort': {
    effort: {
      kind: 'shell',
      reason:
        "sent to a live claude session as the message `/effort <level>` (applyLiveEffort), so the value is prompt text for an already-running agent — the reach agents.sendMessage has, not the raw PTY write claude.answer has. Managed providers take the structural /model endpoint instead, where it selects among the provider's own levels",
    },
  },
  'claude.setModel': {
    effort: {
      kind: 'shell',
      reason:
        'delivered structurally only for managed providers. A Claude PTY request that includes effort is refused before queue/persistence mutation because its daemon-built `/model` command cannot apply effort; callers must use claude.setEffort, whose separate `/effort` message path genuinely delivers it',
    },
  },
  'fleetWorkflows.request': {
    callerSessionId: {
      kind: 'id',
      reason:
        'Facade-stamped caller session identity. Raw owner operations retain actual host authority, and task transitions validate the live local manager plus selected task/project ownership before mutation.',
    },
    cwd: {
      kind: 'path',
      reason:
        'Canonical project selection for workflow/task ownership, never a task-store filename or process argv. Rust resolves real directory aliases before comparing selected project identity; offline exact-record history remains readable.',
    },
  },
  'agents.reportProgress': {
    note: {
      kind: 'shell',
      reason:
        "prompt text for an already-running agent, like claude.setEffort's value and unlike claude.answer's — it is delivered with claudemonSessionClient.message (the queued /message endpoint every other [fleet] wake uses), never written to a PTY, and never composed into argv. The caller controls the SENTENCE and nothing around it: the host flattens it to one line, refuses it over 500 chars, and wraps it in a header and tail it composes itself (buildFleetMessage('progress')), which state that the sender is still running and that this is not a completion",
    },
    callerSessionId: {
      kind: 'id',
      reason:
        "selects the CALLER, not a target: the host reads this session out of its own store and delivers to that row's parentSessionId, so the value can only ever pick a (session, its own parent) pair that already exists — it can name no recipient, and a session with no parent or a dead parent is refused rather than routed anywhere. On the path an agent actually uses it is not a caller value at all: the MCP facade overwrites it from the request token's `session:<id>` label, and the hub bus strips it from every untrusted caller (sanitizeReportProgressParams in internal/bus/rpc.go)",
    },
  },
  'claude.answer': {
    text: {
      kind: 'shell',
      reason:
        'typed into the session\'s PTY as `text + "\\r"` on both providers — byte-for-byte sessions.terminalInput\'s primitive, with no pending question required and no ownership check on the sessionId, so it reaches a terminals.create shell as readily as an agent. Holding the capability is the gate, exactly as for sessions.terminalInput; it buys nothing that granting THAT method does not, and grants nothing less',
    },
    answers: {
      kind: 'shell',
      reason:
        "the multi-part spelling of `text`: each element is typed into the same PTY in turn, so excusing only `text` would leave the identical primitive unclassified under a second name (the mistake sessions.terminalInput's `bytesB64` records)",
    },
    option: {
      kind: 'shell',
      reason:
        'the numeric spelling — `option + "\\r"` into the same PTY. It is the narrowest of the three (a number), and it is listed because the decision has to cover every param that reaches the sink, not the one that looks worst',
    },
  },
  'agents.sendMessage': {
    text: {
      kind: 'shell',
      reason:
        'Prompt text delivered to an already-running provider through its owned message endpoint. Provider tool approvals and permission modes apply, while Workspacer adds no removed independent provider permission clamp.',
    },
  },
  'git.status': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'inert',
      reason:
        'The common Rust git dispatcher reads this optional field, but this method does not consult the value or pass it to git. Its repository remains selected only by canonical cwd.',
    },
  },
  'git.log': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'inert',
      reason:
        'The common Rust git dispatcher reads this optional field, but this method does not consult the value or pass it to git. Its repository remains selected only by canonical cwd.',
    },
  },
  'git.diff': {
    path: {
      kind: 'path',
      reason:
        'an optional pathspec contained to the work-tree root git will actually resolve it in (workRoot via anchorGitPathspec), not the raw cwd spelling',
    },
  },
  'git.numstat': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'path',
      reason:
        'Optional file operand in the shared diff/numstat implementation, passed after -- only after operand canonicalizes and contains it within the selected work-tree root.',
    },
  },
  'git.commitDiff': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    hash: {
      kind: 'argv',
      reason:
        "lands in `git show` argv, where a leading '-' would be an option: gitService.assertCommitHash refuses anything that is not 4-40 hex digits before it gets there",
    },
    path: {
      kind: 'path',
      reason: 'a pathspec after `--`, interpreted by git inside the already-confined repo',
    },
  },
  'git.commitNumstat': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    hash: {
      kind: 'argv',
      reason:
        'same assertCommitHash gate as git.commitDiff — hex only, so it can never be an option-shaped argv element',
    },
    path: {
      kind: 'inert',
      reason:
        'The common Rust git dispatcher reads this optional field, but this method does not consult the value or pass it to git. Its repository remains selected only by canonical cwd.',
    },
  },
  'git.stage': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'path',
      reason:
        'anchored and contained to the work-tree root git will actually resolve it in; with no path the call is bounded to the canonical cwd instead of widening to the root',
    },
  },
  'git.unstage': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'path',
      reason:
        'same repository object containment as git.stage; the path-less form is likewise bounded to the canonical cwd by cwdPathspec',
    },
  },
  'git.commit': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'inert',
      reason:
        'The common Rust git dispatcher reads this optional field, but this method does not consult the value or pass it to git. Its repository remains selected only by canonical cwd.',
    },
  },
  'git.push': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
    path: {
      kind: 'inert',
      reason:
        'The common Rust git dispatcher reads this optional field, but this method does not consult the value or pass it to git. Its repository remains selected only by canonical cwd.',
    },
  },
  'sessions.load': {
    filename: {
      kind: 'filename',
      reason:
        "a bare basename resolved and confined to <configDir>/sessions by both providers (paths::selected_path / resolveWithinSessionsDir); pinned by the corpus's sessionFilenames block",
    },
  },
  'sessions.save': {
    filename: {
      kind: 'filename',
      reason:
        'same as sessions.load: the provider derives it from the session name and re-checks it with the same resolver',
    },
    name: {
      kind: 'filename',
      reason:
        'the value one step BEFORE the filename: both providers slug it (slug in the Rust saved-state service, slugSession in sessionService.ts, pinned against each other by contracts/filename-slug-cases.json) and then run the result through the same sessions-dir containment as a caller-supplied filename',
    },
  },
  'sessions.delete': {
    filename: {
      kind: 'filename',
      reason:
        "same bare-basename rule as sessions.load, and it matters more here: the desktop copy of this resolver read and UNLINKED through a symlink until both were held to the corpus's sessionFilenames block (paths::selected_path / resolveWithinSessionsDir)",
    },
  },
  'sessions.terminalInput': {
    data: {
      kind: 'shell',
      reason:
        "raw bytes into an existing session's PTY: there is no path and no subtree to confine, so holding the capability is the gate, exactly as for terminals.create",
    },
    bytesB64: {
      kind: 'shell',
      reason:
        'the base64 half of the same PTY byte stream — the brain accepts either encoding, so excusing only `data` would leave the identical primitive unclassified under a second name',
    },
  },
  'layouts.save': {
    id: {
      kind: 'id',
      reason:
        'a bare name slugged into <configDir>/layouts/<slug>.yaml and re-contained there by both providers (layoutFilePath / layoutService), never a caller-chosen directory',
    },
    name: {
      kind: 'filename',
      reason:
        'when `id` is absent the provider slugs `name` into the id, so it reaches the same layoutFilePath containment',
    },
  },
  'layouts.delete': {
    id: {
      kind: 'id',
      reason:
        'same as layouts.save — re-slugged and re-contained to the layouts directory before anything is unlinked',
    },
  },
  'config.save': {
    'agents.binaries': {
      kind: 'executable',
      reason:
        "the launcher path handed to claudemon's Command::new for every spawned agent, i.e. argv[0]; stripped from a bus write by dropHostTrusted as a dotted PATH so its sibling agents.* settings stay writable",
    },
    'claude.profiles': {
      kind: 'executable',
      reason:
        'The structured profile list can carry configDir and extraArgs into future launches. Generic non-owner config writes cannot replace this host-trusted section; explicit profile methods preserve the ambient operator contract separately.',
    },
    updates: {
      kind: 'url',
      reason:
        "updates.channel is concatenated into the electron-updater feed URL the desktop downloads and installs from, so one '../' relocates the updater to somebody else's repo; the whole section is stripped from a bus write",
    },
    'terminal.shell': {
      kind: 'shell',
      reason:
        'argv[0] of the next terminal the LOCAL user opens: TerminalPane passes `shell || termCfg.shell` to IPC.TERMINAL_CREATE, which spawns argv:[resolvedShell]. The BUS door onto that primitive (terminals.create) has a shell allow-list; the local IPC door deliberately has none, so this is stripped from a bus write rather than allow-listed',
    },
    'terminal.shells': {
      kind: 'shell',
      reason:
        'the same argv[0] as terminal.shell, reached through the NavBar "+" menu (shells[].path). Stripped as a dotted PATH so the sibling terminal.* settings a bus client legitimately edits stay writable',
    },
    'editor.terminalCommand': {
      kind: 'shell',
      reason:
        "not argv[0] but raw shell TEXT: ScrollContainer builds \"<cmd> <file>\" and TerminalPane types it into the user's own shell with a trailing CR, so ';' and '|' need no planted binary. Live when editor.engine is \"terminal\", which the same call can set",
    },
    scripts: {
      kind: 'shell',
      reason:
        "a map of agent cwd -> [{name,command}] the desktop renders as top-bar buttons and runs as a terminal's initialCommand, verbatim. The attacker picks the LABEL too and the cwd key comes free from agents.list; stripped as a whole SECTION because every key under it is a caller-chosen directory",
    },
  },
  'claude.profiles.add': {
    configDir: {
      kind: 'path',
      reason:
        'Persisted CLAUDE_CONFIG_DIR can select provider settings and hooks. This executable configuration choice remains deliberate authenticated operator authority; no obsolete Workspacer workspace-root or bypass-profile clamp is claimed.',
    },
    extraArgs: {
      kind: 'argv',
      reason:
        'Provider argv persisted for a later selected-profile spawn. The authenticated operator can choose provider permission options; structural launch/identity ownership remains enforced separately from that choice.',
    },
    mcpItemIds: {
      kind: 'id',
      reason:
        'IDs select stored library MCP definitions at later launch preparation. The library validates selected definitions and the launch owner controls integration; this field is not a fabricated credential or implicit proof of provider acceptance.',
    },
    name: {
      kind: 'inert',
      reason:
        "a display label. Profiles live in ONE claude-profiles.json keyed by a generated id (the fixed configured profile file / claudeProfiles.ts), so unlike sessions.save's `name` this one never becomes a filename",
    },
  },
  'claude.profiles.update': {
    name: {
      kind: 'inert',
      reason:
        'a display label on the profile row, exactly as in claude.profiles.add: profiles live in ONE claude-profiles.json keyed by a generated id (the fixed configured profile file), so this never becomes a filename',
    },
    id: {
      kind: 'id',
      reason:
        'Selects an existing row in the fixed profile file. The selected patch carries authenticated operator provider configuration; the ID is never joined into a filesystem path.',
    },
    updates: {
      kind: 'argv',
      reason:
        'Patch object carrying reviewed configDir, extraArgs and mcpItemIds choices. Each field is validated by the profile service; authenticated operator provider configuration remains ambient rather than a separate Workspacer permission grant.',
    },
    configDir: {
      kind: 'path',
      reason: 'same as claude.profiles.add, reached through `updates`',
    },
    extraArgs: {
      kind: 'argv',
      reason: 'same as claude.profiles.add, reached through `updates`',
    },
    mcpItemIds: {
      kind: 'id',
      reason: 'same as claude.profiles.add, reached through `updates`',
    },
  },
  'claude.profiles.remove': {
    id: {
      kind: 'id',
      reason:
        'selects a row to delete from the single claude-profiles.json; never joined into a path, so there is no store directory to escape',
    },
  },
  'claude.approve': {
    decision: {
      kind: 'permission',
      reason:
        "'yes'|'no'|'always' -> claudemon answers Claude Code's PreToolUse hook with {\"decision\":\"approve\"} on stdout, or sends allow=true down a managed adapter's can_use_tool channel; nothing downstream re-asks, so this value alone decides whether a queued tool call runs on the host",
    },
  },
  'claude.gate': {
    on: {
      kind: 'permission',
      reason:
        "arms or disarms the PreToolUse parking gate for a running session: with it ON every tool call stops and waits for claude.approve, and with it OFF the agent's own configured permissions apply. It changes what the host will do without asking, which is what KindPermission means",
    },
  },
  'notifications.post': {
    url: {
      kind: 'url',
      reason:
        "opened on click by the HOST, so a bus caller chooses a destination the desktop user's browser then visits; it goes through openExternalUrl, the same scheme allowlist the renderer's open-external path uses, rather than straight to the OS",
    },
  },
  'library.save': {
    id: {
      kind: 'filename',
      reason:
        'slugged (slugLibrary) into <libraryDir>/<id>.md and re-confined by assertLibraryItemPath / guardLibraryFile before the write, so it names a file inside the item roots and cannot compose one outside them',
    },
    command: {
      kind: 'executable',
      reason:
        'argv[0] of a Library MCP server. It executes only when an authenticated agents.spawn caller explicitly selects that item; the spawn capability is the trust boundary',
    },
    args: {
      kind: 'argv',
      reason:
        "the MCP server's argv[1:], stored beside `command` and gated the same way: unreachable without a spawn that selects the item, and a bus spawn refuses to",
    },
    env: {
      kind: 'env',
      reason:
        "the MCP server's environment, stored beside `command`. Same gate — an env is code execution by another route (PATH, LD_PRELOAD), so it is refused at spawn selection rather than trusted at write time",
    },
    url: {
      kind: 'url',
      reason:
        'an SSE/HTTP MCP endpoint the agent would connect to instead of spawning a process; same selection gate as `command`, and the item file itself stays confined to the library item roots',
    },
  },
  'library.list': {
    id: {
      kind: 'id',
      reason:
        'an exact-match filter applied to the ALREADY-BUILT listing — the same files are opened, the same per-file guard confines them, and the filter can only remove rows. It is never joined into a path, so there is no directory for it to escape',
    },
  },
  'library.remove': {
    id: {
      kind: 'id',
      reason:
        "names the item file to unlink under the library dir derived from the already-confined cwd; the unlink target is re-checked by guardLibraryFile('library.remove', libraryItemRoots(canonicalCwd))",
    },
  },
  'search.project': {
    regex: {
      kind: 'regex',
      reason:
        "a boolean MODE selector, not a value: false (the default) makes the provider pass ripgrep -F so `query` is a fixed string, true means `query` is a real pattern. It carries nothing of its own, and the pattern's own safety is argued in the `query` decision",
    },
    query: {
      kind: 'regex',
      reason:
        "passed after `--` in ripgrep's argv, so it can never become an option; with regex:true it is a real pattern, but rg's engine is linear-time (Rust regex, no backtracking) and the exec is timeout-bounded on both providers",
    },
  },
  'files.receiveUpload': {
    name: {
      kind: 'filename',
      reason:
        'advisory only: the basename is discarded and the extension must be on the image/pdf allowlist; the written path is hub-composed (os.TempDir()/workspacer-uploads/m-<ts>-<rand>.<ext>)',
    },
  },
  'ui.asset': {
    file: {
      kind: 'filename',
      reason:
        'A single selected cache basename, bounded and contained under the fixed font/icon roots.',
    },
  },
  'desktop.readFileBytes': {
    path: {
      kind: 'path',
      reason:
        'Authenticated desktop owner-selected file bytes. The shared file service canonicalizes the path before a bounded read; this is ambient owner filesystem authority, never a plugin or guest transport.',
    },
  },
  'desktop.installUiFont': {
    name: {
      kind: 'filename',
      reason:
        'One validated font basename under the fixed UI font cache. basename refuses traversal, font restricts extension and the service validates magic bytes and length before writing.',
    },
  },
};
export const dangerousNames: Record<string, string> = {
  launchIntegrationGranted: 'permission',
  remoteOrigin: 'id',
  executionTarget: 'id',
  remoteCwd: 'path',
  dispatchOwnerSessionId: 'id',
  retrySourceSessionId: 'id',
  path: 'path',
  cwd: 'path',
  dir: 'path',
  directory: 'path',
  filePath: 'path',
  root: 'path',
  paths: 'path',
  configDir: 'path',
  workdir: 'path',
  filename: 'filename',
  fileName: 'filename',
  file: 'filename',
  name: 'filename',
  shell: 'executable',
  command: 'executable',
  cmd: 'executable',
  binary: 'executable',
  binaries: 'executable',
  bin: 'executable',
  executable: 'executable',
  interpreter: 'executable',
  program: 'executable',
  args: 'argv',
  argv: 'argv',
  extraArgs: 'argv',
  flags: 'argv',
  hash: 'argv',
  ref: 'argv',
  rev: 'argv',
  data: 'shell',
  bytesB64: 'shell',
  stdin: 'shell',
  script: 'shell',
  keys: 'shell',
  text: 'shell',
  answers: 'shell',
  option: 'shell',
  effort: 'shell',
  note: 'shell',
  mode: 'permission',
  permissionMode: 'permission',
  skipPermissions: 'permission',
  decision: 'permission',
  on: 'permission',
  env: 'env',
  envVars: 'env',
  environment: 'env',
  url: 'url',
  uri: 'url',
  endpoint: 'url',
  href: 'url',
  webhook: 'url',
  port: 'port',
  id: 'id',
  itemId: 'id',
  mcpItemIds: 'id',
  profileId: 'id',
  launchIntegrationId: 'id',
  callerSessionId: 'id',
  updates: 'argv',
  ops: 'path',
  query: 'regex',
  pattern: 'regex',
  regex: 'regex',
  glob: 'regex',
};
export const pathNamespaces: string[] = [
  'fs.',
  'search.',
  'library.',
  'git.',
  'providers.',
  'brief.',
];
