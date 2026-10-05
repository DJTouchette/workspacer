/** Reviewed authority decisions captured from legacy capspec/composition.go.
 * This is a source/authority registry, not a behavioral golden corpus. Guard
 * witnesses are checked against live Rust/retained Electron implementations;
 * parameter witnesses retain the method-specific non-inert decision record. */
export interface Witness {
  kind: 'none' | 'guard' | 'rust-git' | 'params' | 'narrows' | 'topic';
  guard?: string;
  params?: string[];
  widens?: string;
  topic?: string;
}
export interface Claim {
  reason: string;
  witnesses: Witness[];
}
export const claims: Record<string, Claim> = {
  'plugins.prepareLaunch': {
    reason:
      "The callback can return child-local env/argv, but only as part of the exact already-authorized owner spawn on the caller's provider connection. It writes no grant or host configuration. authorizeLaunchPreparation refuses ambient provider use, swapped plugin ids, finished calls and revoked owners, and the resulting patch is validated before launching the same child.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'authorizeLaunchPreparation',
      },
    ],
  },
  'agents.dispatchPrepare': {
    reason:
      'Allocates a remote worktree but executes no agent or repository setup hook. Its canonical `cwd` and `remoteOrigin` nonce are validated before allocation; agents.spawn separately requires the same credential and consumes the durable single-use lease. No local workflow identifier or permission grant is accepted from the origin.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd', 'remoteOrigin'],
      },
    ],
  },
  'fleet.selectDispatchModel': {
    reason:
      'Forwards only a model-selection request and its remote `cwd` to the explicitly paired host. The response is a routing decision, not a spawn; the remote spawn gate separately applies credential and routing ceilings and validates actual provider readiness and cwd. No local path is opened and no worker is launched here.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'routing.preferences.validate': {
    reason:
      'WRITE-THEN-INTERPRET: sparse typed policy only, composed by routing.Service, never host YAML. WIDEN-THEN-USE: host model classification and freshness floors are retained, no ranks, ceilings or tool scope are accepted. routingPreferencesTrusted requires authenticated host operator authority, excludes scoped operator and peer-link callers. CAS validates before atomic install.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'routingPreferencesTrusted',
      },
    ],
  },
  'routing.preferences.save': {
    reason:
      'WRITE-THEN-INTERPRET: sparse typed policy only, composed by routing.Service, never host YAML. WIDEN-THEN-USE: host model classification and freshness floors are retained, no ranks, ceilings or tool scope are accepted. routingPreferencesTrusted requires authenticated host operator authority, excludes scoped operator and peer-link callers. CAS validates before atomic install.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'routingPreferencesTrusted',
      },
    ],
  },
  'routing.preferences.reset': {
    reason:
      'WRITE-THEN-INTERPRET: sparse typed policy only, composed by routing.Service, never host YAML. WIDEN-THEN-USE: host model classification and freshness floors are retained, no ranks, ceilings or tool scope are accepted. routingPreferencesTrusted requires authenticated host operator authority, excludes scoped operator and peer-link callers. CAS validates before atomic install.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'routingPreferencesTrusted',
      },
    ],
  },
  'routing.preview': {
    reason:
      'Pure selection reads bounded usage and cached provider snapshots; writes no audit, events, config or sessions. Canonical `cwd` selects a trusted ceiling but no mappings or paths are returned.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'fs.read': {
    reason:
      'reads the absolute path chosen by an authenticated operator agent. Directory grants and secret-path filters were intentionally removed; the caller already holds host file authority. NOTHING HERE IS MACHINE-CHECKED because ambient host access is the product contract.',
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'fs.write': {
    reason:
      'writes the absolute path chosen by an authenticated operator agent. Directory grants and secret-path filters were intentionally removed; the caller already holds host file authority and no later Workspacer guard treats the write as a lesser trust class. NOTHING HERE IS MACHINE-CHECKED because unrestricted host file access is the product contract.',
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'search.project': {
    reason:
      "searches the caller-chosen absolute directory under the authenticated operator's host authority. Result filtering is intentionally not a second filesystem grant. NOTHING HERE IS MACHINE-CHECKED because unrestricted host file access is the product contract.",
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'providers.listModels': {
    reason:
      'runs the chosen provider in the caller-chosen directory to discover models. The authenticated operator may already launch that provider and access that directory. NOTHING HERE IS MACHINE-CHECKED because unrestricted host access is the product contract.',
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'layout.set': {
    reason: '',
    witnesses: [],
  },
  'layouts.save': {
    reason: '',
    witnesses: [],
  },
  'sessions.save': {
    reason: '',
    witnesses: [],
  },
  'agents.spawn': {
    reason: '',
    witnesses: [],
  },
  'agents.sendMessage': {
    reason: '',
    witnesses: [],
  },
  'claude.approve': {
    reason: '',
    witnesses: [],
  },
  'replay.open': {
    reason: '',
    witnesses: [],
  },
  'replay.read': {
    reason: '',
    witnesses: [],
  },
  'sessions.attachTerminal': {
    reason: '',
    witnesses: [],
  },
  'push.subscribe': {
    reason: '',
    witnesses: [],
  },
  'fs.readImage': {
    reason:
      'returns decoded image bytes from an authenticated caller-selected path canonicalized by assertPathAllowed and writes nothing; no interpreter sits downstream',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'fs.listEntries': {
    reason:
      'returns names and types under an authenticated caller-selected path canonicalized by assertPathAllowed; it writes nothing itself and no guard consults its output',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'fs.listDir': {
    reason:
      'the same ambient authenticated enumeration as fs.listEntries with a different result shape and the same assertPathAllowed canonicalization; identical reasoning',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'fs.unwatch': {
    reason:
      'removes a watcher this caller installed \u2014 the undo of fs.watch over the same ambient path canonicalized by assertPathAllowed. It can only ever SHRINK what fs.changed carries',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
      {
        kind: 'narrows',
        widens: 'fs.watch',
      },
    ],
  },
  'brief.append': {
    reason:
      'appends one line to <project>/.workspacer/brief.md: assertPathAllowed canonicalizes the ambient project directory and the provider composes the basename. It is additive-only and writes prose',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'brief.archive': {
    reason:
      'moves entries from <project>/.workspacer/brief.md into brief.archive.md: assertPathAllowed canonicalizes the ambient project directory and the provider composes both basenames. It cannot name an arbitrary file',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'brief.check': {
    reason:
      'reads <project>/.workspacer/brief.md under an ambient project directory canonicalized by assertPathAllowed and reports stale Now entries. It writes nothing and the provider composes the basename',
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'git.status': {
    reason:
      'runs git status in a caller-selected cwd canonicalized by guardGitCwd and returns text. Writes nothing',
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'rust-git',
        guard: 'canonicalize',
      },
    ],
  },
  'git.log': {
    reason:
      'reads commit metadata from a caller-selected repo canonicalized by guardGitCwd. Writes nothing and changes no state',
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'rust-git',
        guard: 'canonicalize',
      },
    ],
  },
  'git.numstat': {
    reason:
      'reads per-file change counts from a caller-selected repo canonicalized by guardGitCwd. Nothing is written',
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'rust-git',
        guard: 'canonicalize',
      },
    ],
  },
  'git.commitDiff': {
    reason:
      "reads one commit's patch text from a caller-selected repo canonicalized by guardGitCwd. Authenticated path access is ambient; it writes nothing",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
    ],
  },
  'git.commitNumstat': {
    reason:
      "reads one commit's change counts from a caller-selected repo canonicalized by guardGitCwd; it writes nothing",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
    ],
  },
  'files.upload': {
    reason:
      "writes caller bytes to a FRESHLY CREATED, hub-named 0600 file under os.TempDir()/workspacer-uploads \u2014 a directory nothing in the host reads as config, code, argv or policy, with the caller's `name` param reduced to its allowlisted image/pdf extension (its per-param decision is on the record) so no executable class lands. WRITE-THEN-INTERPRET: the only downstream reader is an agent, and only if a caller also names the path via agents.sendMessage \u2014 which is the tier's one AcceptedIn pair, whose excuse (the agent's own tool approvals gate what a message makes it read) covers an uploaded image exactly as it covers any pre-existing host path a message names. WIDEN-THEN-USE: it changes no grant, root set, permission mode, approval gate or session, and no guard consults the upload directory; the returned path is information, not authority",
    witnesses: [
      {
        kind: 'params',
        params: ['name'],
      },
    ],
  },
  'fleetWorkflows.request': {
    reason:
      'WRITE-THEN-INTERPRET: definitions contribute only template text and result contracts to existing agents.spawn, whose router model/cost ceilings still apply. The `cwd` selects project policy; `callerSessionId` is facade-stamped and task ownership is checked. Host-known step kind derives worktree isolation. Generic config writers cannot change selections. WIDEN-THEN-USE: no executable paths, models or capabilities in definitions; edits only affect new pinned tasks and never launch agents.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd', 'callerSessionId'],
      },
    ],
  },
  'agents.reportProgress': {
    reason:
      "WRITE-THEN-INTERPRET: `note` is read as instruction, by an AGENT \u2014 but that crossing is already fully available to any caller holding agents.sendMessage, whose recorded pair covers it, and this method reaches strictly less of it. The caller cannot pick the reader (the host derives it from the caller's own parentSessionId), cannot suppress the host-composed header that says the sender is still running, and cannot exceed one 500-char line per 60s. WIDEN-THEN-USE: it changes no grant, root set, permission mode, approval gate or session \u2014 the only state it touches is its own in-memory per-session budget, which nothing else consults, and `callerSessionId` selects the caller rather than a target",
    witnesses: [
      {
        kind: 'params',
        params: ['note', 'callerSessionId'],
      },
    ],
  },
  'routing.select': {
    reason:
      'routing.select returns information from the host-owned matrix and cached usage. WRITE-THEN-INTERPRET: its audit/event writes are not executable policy; authenticated ambient filesystem authority may independently edit host files. WIDEN-THEN-USE: the `cwd` selects a canonical trusted ceiling, but a returned model grants no spawn authority and agents.spawn applies its own current admission checks.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'claude.sessionsForDir': {
    reason:
      "lists claudemon's known sessions for a directory. Read-only, its `cwd` carries a per-parameter decision on the record, and the ids it returns are already handed out by agents.list and sessions.snapshots",
    witnesses: [
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'claude.handoffBrief': {
    reason:
      "renders a deterministic handoff brief into ~/.workspacer/handoffs/ and returns its path. The successor agent's composer is PRE-FILLED with 'read this file' rather than instructed by it, and the file is prose, not argv \u2014 the interpreter is a human reading a chat box. Its argv/profile fields come from the handoff builder, not the caller. NOTHING HERE IS MACHINE-CHECKED: the claim is about what a human does with the text",
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'claude.handoffAgentBrief': {
    reason:
      'the per-agent variant of claude.handoffBrief; same builder, same output location, same reasoning. NOTHING HERE IS MACHINE-CHECKED either, for the same reason: the interpreter is a human reading a chat box',
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'replay.diff': {
    reason:
      "reads a diff out of a worktree replay.open cut. Its containment is the recorded replay.open\u2192replay.read pair's, re-run per call by guardReplaySession('replay.diff', \u2026)",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardReplaySession',
      },
    ],
  },
  'replay.seek': {
    reason:
      "moves a cursor inside a replay session, behind the same guardReplaySession('replay.seek', \u2026) re-containment; no bytes leave the worktree that replay.read would not also return",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardReplaySession',
      },
    ],
  },
  'library.list': {
    reason:
      "enumerates prompt/skill markdown under a directory derived from a cwd assertPathAllowed('library.list', \u2026) confines, with guardLibraryFile('library.list', \u2026) over the item paths themselves. The items are inserted into a composer for a human to send, not executed",
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
    ],
  },
  'library.save': {
    reason:
      "writes a .md prompt/skill into the library directory guardLibraryCwd('library.save', \u2026) confines. The bytes become CHAT TEXT a human sends, never argv or config \u2014 and the one place a library item IS interpreted (a skill file) is generated by the host, not by this call",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardLibraryCwd',
      },
    ],
  },
  'library.remove': {
    reason:
      "deletes one library item, inside the same guardLibraryCwd('library.remove', \u2026) confinement \u2014 the undo of library.save. Removal cannot introduce an interpreter, and nothing consults the item set as policy",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardLibraryCwd',
      },
      {
        kind: 'narrows',
        widens: 'library.save',
      },
    ],
  },
  'sessions.delete': {
    reason:
      "deletes a saved session document. It can only remove a boot-restore document, never add one \u2014 the ADD direction is sessions.save, whose recorded pair with agents.spawn is where this document's composition lives",
    witnesses: [
      {
        kind: 'narrows',
        widens: 'sessions.save',
      },
    ],
  },
  'layouts.delete': {
    reason:
      'deletes a saved layout template; same direction, same reasoning as sessions.delete, with layouts.save as the widening twin',
    witnesses: [
      {
        kind: 'narrows',
        widens: 'layouts.save',
      },
    ],
  },
  'sessions.load': {
    reason:
      "reads a saved session document back; its `filename` carries a per-parameter decision on the record. The document's dangerous direction is its WRITER (the recorded pair); reading it hands the caller bytes it could have written",
    witnesses: [
      {
        kind: 'params',
        params: ['filename'],
      },
    ],
  },
  'sessions.transcript': {
    reason:
      "returns a session's transcript text for a `cwd` that carries a per-parameter decision on the record. Read-only, and the transcript is rendered, never executed",
    witnesses: [
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'config.save': {
    reason:
      'writes config.yaml, which IS re-read as argv by the host \u2014 and that is exactly why every such key (agents.binaries, claude.profiles, terminal.shell, terminal.shells, editor.terminalCommand, scripts, updates) is classified per-parameter in capspec and STRIPPED from a bus write by dropHostTrusted, held equal to contracts/host-trusted-config-cases.json. The composition is real, it is closed per-key rather than per-pair, and the per-key record is the one that cannot drift silently',
    witnesses: [
      {
        kind: 'params',
        params: [
          'agents.binaries',
          'claude.profiles',
          'terminal.shell',
          'terminal.shells',
          'editor.terminalCommand',
          'scripts',
          'updates',
        ],
      },
    ],
  },
  'jobs.upsert': {
    reason: '',
    witnesses: [],
  },
  'jobs.run': {
    reason: '',
    witnesses: [],
  },
  'jobs.propose': {
    reason:
      'the agent-facing half of jobs.upsert, deliberately weaker: it can only CREATE, what it creates is forced disabled and stamped proposedBy, and jobs.run refuses a stamped row \u2014 so the argv it persists cannot execute until a trusted caller writes that row back with the stamp cleared. Gated by jobsTrusted like every other jobs.* method (only the actual trusted host operator passes; scoped tokens and plugins do not); the restraint that matters here is not the identity gate but the method \u2014 the MCP facade hands agents a tool for this and none for jobs.upsert',
    witnesses: [
      {
        kind: 'guard',
        guard: 'jobsTrusted',
      },
    ],
  },
  'jobs.list': {
    reason:
      'returns the stored specs \u2014 which DISCLOSE shell commands and agent prompts, which is why jobsTrusted refuses every non-host caller \u2014 but writes nothing and executes nothing',
    witnesses: [
      {
        kind: 'guard',
        guard: 'jobsTrusted',
      },
    ],
  },
  'jobs.remove': {
    reason:
      'deletes a stored job and its history behind the same jobsTrusted gate \u2014 the undo of jobs.upsert; it cannot introduce argv, only retire it',
    witnesses: [
      {
        kind: 'narrows',
        widens: 'jobs.upsert',
      },
      {
        kind: 'guard',
        guard: 'jobsTrusted',
      },
    ],
  },
  'jobs.history': {
    reason:
      'returns run records (shell output tails included \u2014 disclosure, which is what jobsTrusted gates) for a stored job id; writes nothing and executes nothing',
    witnesses: [
      {
        kind: 'guard',
        guard: 'jobsTrusted',
      },
    ],
  },
  'nodes.wake': {
    reason:
      "the only caller value is an `id` SELECTING a row the hub already holds in nodes.json. Everything the call then acts on \u2014 the cloud app, the machine id, the API endpoint, the credential \u2014 comes from that file, so there is no caller path to confine and no caller argv to interpret. Nothing it writes is read back as config, code or policy by anything: it writes nothing at all, and the state it changes (a node's state field, in memory only, never persisted) is CONSULTED by exactly one thing, nodes.list, which reports it to a human. The widen-then-use shape it does have is honest and bounded: a woken node becomes a capability PROVIDER, and everything that provider then serves is governed by the same bus authorization every other provider is \u2014 first-registration-wins, per-caller tiers, per-method allowlists \u2014 with nothing about it derived from who pressed wake. What the call really is, is an act with a BILL attached, and the answer to that is identity rather than confinement: nodesTrusted refuses plugin tokens and the view/triage tiers, and the method is admitted to no scoped tier",
    witnesses: [
      {
        kind: 'guard',
        guard: 'nodesTrusted',
      },
    ],
  },
  'remote.tokensList': {
    reason:
      'remotePairingTrusted gates the fixed token store behind authenticated host authority. Only ordinary pairing records are exposed or changed; no paths, provider records, or caller privilege grants. Minted scopes are enforced by the existing bus token lookup.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'remotePairingTrusted',
      },
    ],
  },
  'remote.tokenGetOrCreate': {
    reason:
      'remotePairingTrusted gates the fixed token store behind authenticated host authority. Only ordinary pairing records are exposed or changed; no paths, provider records, or caller privilege grants. Minted scopes are enforced by the existing bus token lookup.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'remotePairingTrusted',
      },
    ],
  },
  'remote.tokenRevoke': {
    reason:
      'remotePairingTrusted gates the fixed token store behind authenticated host authority. Only ordinary pairing records are exposed or changed; no paths, provider records, or caller privilege grants. Minted scopes are enforced by the existing bus token lookup.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'remotePairingTrusted',
      },
    ],
  },
  'machine.stop': {
    reason:
      'Stops only this configured host, no caller-controlled targets or commands. machinePowerTrusted requires operator authority and no state is interpreted as code or policy.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'machinePowerTrusted',
      },
    ],
  },
  'nodes.sleep': {
    reason:
      "the inverse of nodes.wake and the half that closes its bill: the only caller value is an `id` SELECTING a row the hub already holds in nodes.json, and everything the call acts on \u2014 the cloud app, the machine id, the API endpoint, the credential \u2014 comes from that file. It writes nothing, and the state it changes (a node's state field, in memory only, never persisted) is CONSULTED by exactly one thing, nodes.list, which reports it to a human. What it has that its twin does not is two values with real authority in them, the stop SIGNAL and the drain window, and neither is on the wire: both are the supervisor's own tunables, so a caller cannot name SIGKILL and thereby destroy the exit record that distinguishes this deliberate stop from a crash on the next wake. The composition worth naming is with nodes.wake itself and it is bounded rather than open: sleep can only stop a machine wake could start, both are refused to the same callers by the same gate, and neither reads anything the other wrote \u2014 the state field is not config, code or policy to anything. The act with consequences here is not a widening at all, it is a DESTRUCTION: it ends work in flight on a machine somebody may be using. The answer to that is identity, and nodesTrusted is it",
    witnesses: [
      {
        kind: 'guard',
        guard: 'nodesTrusted',
      },
    ],
  },
  'sessionArchive.set': {
    reason:
      "adds or removes one opaque `sessionId` in the hub's own session-archive.json, which every client reads only to decide which sidebar rows to LIST. WRITE-THEN-INTERPRET: nothing reads that file as config, code, argv, path or policy; the id is validated as bounded text and never joined into a path or matched against a live process. WIDEN-THEN-USE: it changes no grant, no root, no permission mode, no approval gate, and it is deliberately not a lifecycle action — no stop, signal, close or delete is reachable from it, and an archived session keeps running. Restoring is the same call with archived:false",
    witnesses: [
      {
        kind: 'params',
        params: ['sessionId'],
      },
    ],
  },
  'usage.setPacingSchedule': {
    reason:
      "the only caller value is `schedule`, a closed two-word enum validated before anything is written, and what it SELECTS is which of two curve words internal/limits already implements. WRITE-THEN-INTERPRET: the one thing it writes is the hub's own usage-pacing.json, and the only reader of that file is usageprefs.Open at boot plus usage.report's projection \u2014 no interpreter reads it as config, code, argv or policy, and it can hold nothing but one of two words this build compiled in. WIDEN-THEN-USE: it changes no grant, no root set, no permission mode, no approval gate and no session, and the one guard that might have consulted it does not \u2014 routing.select reads Matrix.PaceConfig() straight off routing.yaml, which this method cannot write, so the ceilings that govern spawns are untouched by it. Its gate is identity rather than confinement, because there is no path here to confine: usagePrefsTrusted refuses plugin tokens and the view/triage tiers, and the method is admitted to no scoped tier",
    witnesses: [
      {
        kind: 'guard',
        guard: 'usagePrefsTrusted',
      },
    ],
  },
  'claude.profiles.add': {
    reason:
      'Persists configDir (CLAUDE_CONFIG_DIR), extraArgs (provider argv), and mcpItemIds (selected library MCP IDs), each with a non-inert parameter decision. These values can influence a later spawn under authenticated operator authority; provider permission choices are deliberately not a separate Workspacer grant, and no removed bypass-profile scrub is claimed.',
    witnesses: [
      {
        kind: 'params',
        params: ['configDir', 'extraArgs', 'mcpItemIds'],
      },
    ],
  },
  'claude.profiles.update': {
    reason:
      'Persists configDir (CLAUDE_CONFIG_DIR), extraArgs (provider argv), and mcpItemIds (selected library MCP IDs), each with a non-inert parameter decision. These values can influence a later spawn under authenticated operator authority; provider permission choices are deliberately not a separate Workspacer grant, and no removed bypass-profile scrub is claimed.',
    witnesses: [
      {
        kind: 'params',
        params: ['configDir', 'extraArgs', 'mcpItemIds'],
      },
    ],
  },
  'claude.profiles.remove': {
    reason:
      'removes a profile \u2014 the undo of claude.profiles.add. It cannot introduce a configDir or extraArgs; a spawn naming a removed profile falls back to the default',
    witnesses: [
      {
        kind: 'narrows',
        widens: 'claude.profiles.add',
      },
    ],
  },
  'notifications.post': {
    reason:
      'renders a title/body into an OS notification and an in-app card; its one value the host acts on is `url`, which carries a per-parameter decision. Text to a human otherwise: no argv, no file, and nothing consults the notification set',
    witnesses: [
      {
        kind: 'params',
        params: ['url'],
      },
    ],
  },
  'push.unsubscribe': {
    reason:
      "drops a push subscription, proven by possession of the subscription's own `endpoint` auth secret \u2014 the undo of push.subscribe, whose recorded pair carries that sink's composition. Removal only",
    witnesses: [
      {
        kind: 'narrows',
        widens: 'push.subscribe',
      },
      {
        kind: 'params',
        params: ['endpoint'],
      },
    ],
  },
  'push.revoke': {
    reason:
      "drops another credential's push subscriptions by `id` \u2014 a revocation, i.e. the direction that can only narrow what push.subscribe widened",
    witnesses: [
      {
        kind: 'narrows',
        widens: 'push.subscribe',
      },
      {
        kind: 'params',
        params: ['id'],
      },
    ],
  },
  'claude.setModel': {
    reason:
      "asks the owner to switch a running agent's model. Managed effort may ride structurally; Claude PTY effort is refused because its validated `/model` command cannot apply it. The owner records canonical requested_selection and reports queued versus accepted without claiming provider execution. No guard reads the model; permission mode is a separate method",
    witnesses: [
      {
        kind: 'params',
        params: ['effort'],
      },
    ],
  },
  'claude.setEffort': {
    reason:
      "switches a running agent's reasoning `effort`, classified per-parameter. Like the model, effort is a parameter of generation that no guard anywhere consults \u2014 it is not the permission mode, which several do",
    witnesses: [
      {
        kind: 'params',
        params: ['effort'],
      },
    ],
  },
  'claude.signal': {
    reason:
      'sends an interrupt/stop signal to a running agent. It can only ever STOP work; nothing consults it. NOTHING HERE IS MACHINE-CHECKED: the method carries no classified value and no guard names it, so this sentence is the whole of the evidence',
    witnesses: [
      {
        kind: 'none',
      },
    ],
  },
  'claude.gate': {
    reason:
      'parks a tool call for human approval \u2014 it only ADDS a gate, and its `on` value is classified per-parameter. Removing one is claude.approve, the recorded half this method is the undo of',
    witnesses: [
      {
        kind: 'narrows',
        widens: 'claude.approve',
      },
      {
        kind: 'params',
        params: ['on'],
      },
    ],
  },
  'claude.answer': {
    reason:
      "answers an agent's question prompt: `text`, `answers` and `option` are each classified as PTY bytes, the same primitive sessions.terminalInput carries. Gated by the agent's own approvals exactly as agents.sendMessage is \u2014 and that pair (agents.sendMessage + claude.approve) is recorded and accepted for the triage tier, which is where this method's composition risk already lives",
    witnesses: [
      {
        kind: 'params',
        params: ['text', 'answers', 'option'],
      },
    ],
  },
  'claude.setPermissionMode': {
    reason:
      "changes the running provider's `mode`. An authenticated agent caller may choose any provider-supported mode; no Workspacer grant or later guard depends on it",
    witnesses: [
      {
        kind: 'params',
        params: ['mode'],
      },
    ],
  },
  'sessions.terminalInput': {
    reason:
      "types bytes into a session's PTY \u2014 `data` and `bytesB64`, both classified as exactly that. Its OUTPUT side (sessions.attachTerminal / pty.bytes.*) is a recorded pair; the input side reaches a shell that is already running as the user, which is what terminals.create's own allow-list governs",
    witnesses: [
      {
        kind: 'params',
        params: ['data', 'bytesB64'],
      },
    ],
  },
  'terminals.create': {
    reason:
      'starts a shell: `shell` is argv[0] and `cwd` is where it runs, both classified per-parameter, and argv[0] is closed by the shell allow-list (lib/shellAllowlist.ts + cmd/brain/shellallow.go, one list held equal by a corpus). The config keys that could redirect that argv are stripped by dropHostTrusted \u2014 the two halves of that chain are classified where they live',
    witnesses: [
      {
        kind: 'params',
        params: ['shell', 'cwd'],
      },
    ],
  },
  'terminals.open': {
    reason:
      'Publishes a visible-terminal intent containing `cwd` and command, both classified per-parameter. A UI with terminal panes interprets the command inside the host default shell; an unsupported native or headless UI does not spawn a hidden shell or claim a pane acknowledgment. The intent changes no credential or caller authority.',
    witnesses: [
      {
        kind: 'params',
        params: ['cwd', 'command'],
      },
    ],
  },
  'git.stage': {
    reason:
      "stages paths inside a repo guardGitCwd('git.stage', \u2026) confines. The index is git's own state; no capability here reads it as policy, and the commit that consumes it is git.commit",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
    ],
  },
  'git.unstage': {
    reason:
      "removes paths from the index of a repo guardGitCwd('git.unstage', \u2026) confines \u2014 the undo of git.stage, so it can only ever shrink what a later git.commit records, and nothing reads the index as policy",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'narrows',
        widens: 'git.stage',
      },
    ],
  },
  'git.commit': {
    reason:
      "guardGitCwd selects the repository named by `cwd`; a commit message is not interpreted by Workspacer, and git owns hook/config behavior under the authenticated user's ambient host authority",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'params',
        params: ['cwd'],
      },
    ],
  },
  'git.push': {
    reason:
      "Publishes commits from the repository selected by guardGitCwd('git.push', ...). It writes no Workspacer policy or grant; remote URL and Git hooks/configuration execute under the authenticated user's existing ambient repository authority, with owned process limits and repository object containment.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
    ],
  },
  'git.diff': {
    reason:
      'guardGitCwd selects the repository named by `cwd`; untracked operands are anchored inside that selected work tree so a derived path cannot switch semantic objects',
    witnesses: [
      {
        kind: 'guard',
        guard: 'guardGitCwd',
      },
      {
        kind: 'rust-git',
        guard: 'canonicalize',
      },
    ],
  },
  'fs.watch': {
    reason:
      "installs a change watcher on a path assertPathAllowed('fs.watch', \u2026) confines. Its OUTPUT is the fs.changed topic, and that is where its composition lives: the event registry names this capability as that topic's gate, so a credential refused fs.watch is refused the change feed it produces. The call itself writes nothing and no guard consults the watcher set",
    witnesses: [
      {
        kind: 'guard',
        guard: 'assertPathAllowed',
      },
      {
        kind: 'topic',
        topic: 'fs.changed',
      },
    ],
  },
  'desktop.worktreeInfo': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.worktreeCreate': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.worktreeRemove': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.pricingGetRates': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.pricingSaveOverrides': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesAccounts': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesLoginStatus': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesAddAccount': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.toolsStatus': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.fleetReviewRead': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.fleetReviewForget': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.taskInspectorEdit': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.taskInspectorOpen': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.dispatchHistoryRead': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.htmlCardReadDiff': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.fleetWorkflowRequest': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.managerRequestPrepare': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.managerRequestSend': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.loadBriefBoard': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.moveBriefCard': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesAdd': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesUpdate': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.claudeProfilesRemove': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.saveConfig': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.agentSuggestTitle': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.providerReadiness': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.agentRuntimeStatus': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.keepWarmHeartbeats': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.workflowAgentTranscript': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.workflowAgentConversation': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.installUiFont': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.downloadProjectIcon': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.managerReplacement': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.sessionGrantReconcile': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.readFileBytes': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'desktop.filePickerList': {
    reason:
      'authenticatedDesktopUser preserves native owner authority before shared desktop services run. Writes can produce account configuration, worktrees, rates and evidence, but only the actual local host may access this dispatcher; caller parameters supply neither filesystem roots nor session identity.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedDesktopUser',
      },
    ],
  },
  'ui.asset': {
    reason:
      'Fixed cache bytes are selected by file basename. canonicalize resolves the selected object under its cache root before the bounded descriptor read, which writes neither configuration nor authority.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'asset-containment',
      },
    ],
  },
  'files.receiveUpload': {
    reason:
      "authenticatedUploadReceiver permits only the hub owner's forwarding call. The same fixed extension/size policy as files.upload applies; bytes become active only if explicitly referenced in an agent message under that tier's existing approval contract. It changes no grant or filesystem root.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'authenticatedUploadReceiver',
      },
    ],
  },
  'federation.peersConfig': {
    reason:
      'peerConfigTrusted requires authenticated local owner identity. The saved peer credentials and dispatch selection can authorize later peer calls, but this write is restricted to the same server owner who owns that authority; providers, scoped operators, plugins and peer links cannot acquire it.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'peerConfigTrusted',
      },
    ],
  },
  'federation.savePeersConfig': {
    reason:
      'peerConfigTrusted requires authenticated local owner identity. The saved peer credentials and dispatch selection can authorize later peer calls, but this write is restricted to the same server owner who owns that authority; providers, scoped operators, plugins and peer links cannot acquire it.',
    witnesses: [
      {
        kind: 'guard',
        guard: 'peerConfigTrusted',
      },
    ],
  },
  'remote.tailscaleInfo': {
    reason:
      "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'networkTrusted',
      },
    ],
  },
  'remote.tailscaleServe': {
    reason:
      "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'networkTrusted',
      },
    ],
  },
  'remote.setSharing': {
    reason:
      "networkTrusted confines listener changes to the authenticated local owner. These changes can expose an already token-protected server, but cannot grant a scoped worker or peer host credentials or root commands. The private combined-node supervisor exposes only Tailscale status and this node's fixed HTTPS proxy.",
    witnesses: [
      {
        kind: 'guard',
        guard: 'networkTrusted',
      },
    ],
  },
};
export const parameterDecisions: Record<
  string,
  Record<string, { kind: string; reason: string }>
> = {
  'sessionArchive.set': {
    sessionId: {
      kind: 'id',
      reason:
        'selects which opaque id the archive document lists; bounded text, never a path, and never resolved to a process',
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
  'fleet.selectDispatchModel': {
    cwd: {
      kind: 'path',
      reason:
        'remote routing input sent only to the explicitly paired host; no desktop filesystem operation or credential propagation',
    },
  },
  'routing.preview': {
    cwd: {
      kind: 'path',
      reason:
        'Canonicalized only to select the existing trusted directory ceiling; no caller path is written or returned.',
    },
  },
  'files.upload': {
    name: {
      kind: 'filename',
      reason:
        'advisory only: the basename is discarded and the extension must be on the image/pdf allowlist; the written path is hub-composed (os.TempDir()/workspacer-uploads/m-<ts>-<rand>.<ext>)',
    },
  },
  'fleetWorkflows.request': {
    cwd: {
      kind: 'path',
      reason:
        'Canonical project selection for workflow/task ownership, never a task-store filename or process argv. Rust resolves real directory aliases before comparing selected project identity; offline exact-record history remains readable.',
    },
    callerSessionId: {
      kind: 'id',
      reason:
        'Facade-stamped caller session identity. Raw owner operations retain actual host authority, and task transitions validate the live local manager plus selected task/project ownership before mutation.',
    },
  },
  'agents.reportProgress': {
    note: {
      kind: 'shell',
      reason:
        "prompt text for an already-running agent, like claude.setEffort's value and unlike claude.answer's \u2014 it is delivered with claudemonSessionClient.message (the queued /message endpoint every other [fleet] wake uses), never written to a PTY, and never composed into argv. The caller controls the SENTENCE and nothing around it: the host flattens it to one line, refuses it over 500 chars, and wraps it in a header and tail it composes itself (buildFleetMessage('progress')), which state that the sender is still running and that this is not a completion",
    },
    callerSessionId: {
      kind: 'id',
      reason:
        "selects the CALLER, not a target: the host reads this session out of its own store and delivers to that row's parentSessionId, so the value can only ever pick a (session, its own parent) pair that already exists \u2014 it can name no recipient, and a session with no parent or a dead parent is refused rather than routed anywhere. On the path an agent actually uses it is not a caller value at all: the MCP facade overwrites it from the request token's `session:<id>` label, and the hub bus strips it from every untrusted caller (sanitizeReportProgressParams in internal/bus/rpc.go)",
    },
  },
  'routing.select': {
    cwd: {
      kind: 'path',
      reason:
        'Canonicalized existing path used to select a directory row from the trusted routing ceiling matrix. The answer is only provider/model/effort information; spawn performs its own current cwd and admission validation.',
    },
  },
  'claude.sessionsForDir': {
    cwd: {
      kind: 'path',
      reason:
        "encoded into a ~/.claude/projects slug by claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component; the caller's string is never opened as a path",
    },
  },
  'sessions.load': {
    filename: {
      kind: 'filename',
      reason:
        "a bare basename resolved and confined to <configDir>/sessions by both providers (sessionFilePath / resolveWithinSessionsDir); pinned by the corpus's sessionFilenames block",
    },
  },
  'sessions.transcript': {
    cwd: {
      kind: 'path',
      reason:
        'selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller',
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
    updates: {
      kind: 'url',
      reason:
        "updates.channel is concatenated into the electron-updater feed URL the desktop downloads and installs from, so one '../' relocates the updater to somebody else's repo; the whole section is stripped from a bus write",
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
  },
  'claude.profiles.update': {
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
  'notifications.post': {
    url: {
      kind: 'url',
      reason:
        "opened on click by the HOST, so a bus caller chooses a destination the desktop user's browser then visits; it goes through openExternalUrl, the same scheme allowlist the renderer's open-external path uses, rather than straight to the OS",
    },
  },
  'push.unsubscribe': {
    endpoint: {
      kind: 'id',
      reason:
        'selects a stored subscription row to delete \u2014 narrowing, and never joined into a path',
    },
  },
  'push.revoke': {
    id: {
      kind: 'id',
      reason:
        'selects a stored subscription row to delete. Operator-only by construction: push.revoke is in neither scoped tier',
    },
  },
  'claude.setModel': {
    effort: {
      kind: 'shell',
      reason:
        'delivered structurally only for managed providers. A Claude PTY request that includes effort is refused before queue/persistence mutation because its daemon-built `/model` command cannot apply effort; callers must use claude.setEffort, whose separate `/effort` message path genuinely delivers it',
    },
  },
  'claude.setEffort': {
    effort: {
      kind: 'shell',
      reason:
        "sent to a live claude session as the message `/effort <level>` (applyLiveEffort), so the value is prompt text for an already-running agent \u2014 the reach agents.sendMessage has, not the raw PTY write claude.answer has. Managed providers take the structural /model endpoint instead, where it selects among the provider's own levels",
    },
  },
  'claude.gate': {
    on: {
      kind: 'permission',
      reason:
        "arms or disarms the PreToolUse parking gate for a running session: with it ON every tool call stops and waits for claude.approve, and with it OFF the agent's own configured permissions apply. It changes what the host will do without asking, which is what KindPermission means",
    },
  },
  'claude.answer': {
    text: {
      kind: 'shell',
      reason:
        'typed into the session\'s PTY as `text + "\\r"` on both providers \u2014 byte-for-byte sessions.terminalInput\'s primitive, with no pending question required and no ownership check on the sessionId, so it reaches a terminals.create shell as readily as an agent. Holding the capability is the gate, exactly as for sessions.terminalInput; it buys nothing that granting THAT method does not, and grants nothing less',
    },
    answers: {
      kind: 'shell',
      reason:
        "the multi-part spelling of `text`: each element is typed into the same PTY in turn, so excusing only `text` would leave the identical primitive unclassified under a second name (the mistake sessions.terminalInput's `bytesB64` records)",
    },
    option: {
      kind: 'shell',
      reason:
        'the numeric spelling \u2014 `option + "\\r"` into the same PTY. It is the narrowest of the three (a number), and it is listed because the decision has to cover every param that reaches the sink, not the one that looks worst',
    },
  },
  'claude.setPermissionMode': {
    mode: {
      kind: 'permission',
      reason:
        "selects the running provider's supported permission mode; authenticated agent access is the trust boundary and Workspacer applies no separate grant",
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
        'the base64 half of the same PTY byte stream \u2014 the brain accepts either encoding, so excusing only `data` would leave the identical primitive unclassified under a second name',
    },
  },
  'terminals.create': {
    shell: {
      kind: 'executable',
      reason:
        "argv[0] of a host process, taken from a bus caller. There is no subtree to confine it to that the same caller cannot also fill in with fs.write, so it is an ALLOWLIST of the host's login shells instead: resolveTerminalShell in both providers (cmd/brain/shellallow.go, lib/shellAllowlist.ts)",
    },
    cwd: {
      kind: 'path',
      reason:
        "a process working directory, as agents.spawn's \u2014 holding the capability is the gate",
    },
  },
  'terminals.open': {
    cwd: {
      kind: 'path',
      reason:
        "a process working directory for the visible terminal pane, as terminals.create's \u2014 holding the capability is the gate",
    },
    command: {
      kind: 'executable',
      reason:
        'Text delivered in a visible-terminal intent for execution inside a UI-selected host default shell. It is never used as argv[0]; unsupported UI owners report unavailable without creating a hidden shell.',
    },
  },
  'git.commit': {
    cwd: {
      kind: 'path',
      reason:
        'guardGitCwd canonicalizes cwd before git runs; authenticated agent/plugin path access is ambient',
    },
  },
};
export const actors: string[] = [
  'agents.dispatchPrepare',
  'agents.reportProgress',
  'agents.sendMessage',
  'agents.spawn',
  'brief.append',
  'brief.archive',
  'brief.check',
  'claude.answer',
  'claude.approve',
  'claude.gate',
  'claude.handoffAgentBrief',
  'claude.handoffBrief',
  'claude.profiles.add',
  'claude.profiles.remove',
  'claude.profiles.update',
  'claude.sessionsForDir',
  'claude.setEffort',
  'claude.setModel',
  'claude.setPermissionMode',
  'claude.signal',
  'config.save',
  'desktop.agentRuntimeStatus',
  'desktop.agentSuggestTitle',
  'desktop.claudeProfilesAccounts',
  'desktop.claudeProfilesAdd',
  'desktop.claudeProfilesAddAccount',
  'desktop.claudeProfilesLoginStatus',
  'desktop.claudeProfilesRemove',
  'desktop.claudeProfilesUpdate',
  'desktop.dispatchHistoryRead',
  'desktop.downloadProjectIcon',
  'desktop.filePickerList',
  'desktop.fleetReviewForget',
  'desktop.fleetReviewRead',
  'desktop.fleetWorkflowRequest',
  'desktop.htmlCardReadDiff',
  'desktop.installUiFont',
  'desktop.keepWarmHeartbeats',
  'desktop.loadBriefBoard',
  'desktop.managerReplacement',
  'desktop.managerRequestPrepare',
  'desktop.managerRequestSend',
  'desktop.moveBriefCard',
  'desktop.pricingGetRates',
  'desktop.pricingSaveOverrides',
  'desktop.providerReadiness',
  'desktop.readFileBytes',
  'desktop.saveConfig',
  'desktop.sessionGrantReconcile',
  'desktop.taskInspectorEdit',
  'desktop.taskInspectorOpen',
  'desktop.toolsStatus',
  'desktop.workflowAgentConversation',
  'desktop.workflowAgentTranscript',
  'desktop.worktreeCreate',
  'desktop.worktreeInfo',
  'desktop.worktreeRemove',
  'federation.peersConfig',
  'federation.savePeersConfig',
  'files.receiveUpload',
  'files.upload',
  'fleet.selectDispatchModel',
  'fleetWorkflows.request',
  'fs.listDir',
  'fs.listEntries',
  'fs.read',
  'fs.readImage',
  'fs.unwatch',
  'fs.watch',
  'fs.write',
  'git.commit',
  'git.commitDiff',
  'git.commitNumstat',
  'git.diff',
  'git.log',
  'git.numstat',
  'git.push',
  'git.stage',
  'git.status',
  'git.unstage',
  'jobs.history',
  'jobs.list',
  'jobs.propose',
  'jobs.remove',
  'jobs.run',
  'jobs.upsert',
  'layout.set',
  'layouts.delete',
  'layouts.save',
  'library.list',
  'library.remove',
  'library.save',
  'machine.stop',
  'nodes.sleep',
  'nodes.wake',
  'notifications.post',
  'plugins.prepareLaunch',
  'providers.listModels',
  'push.revoke',
  'push.subscribe',
  'push.unsubscribe',
  'remote.setSharing',
  'remote.tailscaleInfo',
  'remote.tailscaleServe',
  'remote.tokenGetOrCreate',
  'remote.tokenRevoke',
  'remote.tokensList',
  'replay.diff',
  'replay.open',
  'replay.read',
  'replay.seek',
  'routing.preferences.reset',
  'routing.preferences.save',
  'routing.preferences.validate',
  'routing.preview',
  'routing.select',
  'search.project',
  'sessionArchive.set',
  'sessions.attachTerminal',
  'sessions.delete',
  'sessions.load',
  'sessions.save',
  'sessions.terminalInput',
  'sessions.transcript',
  'terminals.create',
  'terminals.open',
  'ui.asset',
  'usage.setPacingSchedule',
];

export const crossings = [
  {
    Name: 'jobs.upsert persists argv (a shell command or an agent spawn); the scheduler and jobs.run execute it later, unattended',
    Shape: 'ShapeWriteThenInterpret',
    A: 'jobs.upsert',
    B: 'jobs.run',
    Crossing:
      "the job spec is BUILT to be interpreted: a shell action's `command` goes to /bin/sh -c on the hub's machine, a spawn action re-enters agents.spawn with a cwd and a prompt, and the trigger fires with nobody watching. Storage is the hub-owned 0600 jobs.json \u2014 deliberately NOT the library (agent-writable) or the layout (world-readable, broadcast) \u2014 so the file itself is out of reach; the bus surface is the remaining door.",
    ClosedBy:
      'identity, not paths \u2014 jobsTrusted refuses non-trusted callers. Spawn actions re-enter the ordinary agents.spawn path with provider permission choices passed through.',
  },
  {
    Name: "layout.set writes the shared document; the desktop's next launch respawns it through the LOCAL spawn door",
    Shape: 'ShapeWidenThenUse',
    A: 'layout.set',
    B: 'agents.spawn',
    Crossing:
      "the hub stores the document verbatim because it 'does not interpret' it, and the desktop adopts it on hydration and respawns every stopped agent in it through window.electronAPI.spawnClaude \u2014 the LOCAL IPC door, which scrubs nothing. The bus's own agents.spawn refuses skipPermissions, an escalating permissionMode, a bypassing profile and caller-supplied mcpItemIds; all four arrived at the spawn anyway, from a caller that may not spawn at all.",
    ClosedBy:
      "layout.scrubAdoptedSpawnFields, applied to every NON-TRUSTED layout.set (the hub registers it through RegisterLocalIdent so the writer's identity is known), stripping exactly the four fields agents.spawn strips",
  },
  {
    Name: 'replay coordinates remain structurally inside the service-owned worktree',
    Shape: 'ShapeWidenThenUse',
    A: 'replay.open',
    B: 'replay.read',
    Crossing:
      'replay.open accepts an ambient authenticated repository path, while read/diff/seek accept only coordinates inside the disposable worktree it created.',
    ClosedBy:
      'resolveInside/containInWorktree provide object containment and guardReplaySession re-canonicalizes the recorded origin before each operation',
  },
  {
    Name: 'the capability plane refuses sessions.attachTerminal to a view token; the event plane delivered its entire output',
    Shape: 'ShapeWidenThenUse',
    A: 'sessions.attachTerminal',
    B: 'pty.bytes.*',
    Crossing:
      "two authorization planes answering the same question differently. mayCall denies the method to a scoped tier; mayConsume read `cn.trusted || cn.scopeMethods != nil || \u2026`, whose middle clause waved every topic through for any scoped user token. terminals.* is in neither scoped tier at all, so the event plane was the only door onto a terminal's screen \u2014 raw PTY bytes with the ring-buffer replay attaching deliberately restarts.",
    ClosedBy:
      'the event-topic registry (eventtopics.go) consulted by mayConsume via EventTopicSpec, whose DEFAULT IS CLOSED for a scoped user token and which now also filters the plugin arm \u2014 plus the enqueue-time admission filter, so a refused stream no longer even leaves a drop record to escape as pty.desync',
  },
  {
    Name: "sessions.save writes the boot-restore document; the desktop's next launch respawns it through the LOCAL spawn door",
    Shape: 'ShapeWidenThenUse',
    A: 'sessions.save',
    B: 'agents.spawn',
    Crossing:
      'layout.set\'s recorded pair, reached through a DIFFERENT writer that was never scrubbed. sessions.save stamps `timestamp: now` into <configDir>/sessions/<slug>.yaml, which makes it sessions[0]; useSessionLifecycle loads it on boot, migrateSessionData passes the modern format through as-is, and reconcileAgents{respawnStopped:true} hands every card claudemon no longer holds to respawnFromRecord \u2014 which forwards profileId, permissionMode, skipPermissions and mcpItemIds to window.electronAPI.spawnClaude, the LOCAL IPC door that scrubs nothing. capspec excused the method as a PATH question ("the filename is derived from the session name by the provider\'s slug") and nothing in either provider looked at what the document CONTAINS.',
    ClosedBy:
      "scrubBootDocumentAgents, applied unconditionally on both providers (cmd/brain/bootdoc.go and main/lib/bootDocumentScrub.ts) because caller identity does not reach a bus provider \u2014 stripping exactly the four fields internal/layout's scrubAdoptedSpawnFields strips, with the three lists held equal by a test",
  },
  {
    Name: 'layouts.save writes the same agents array into the layout template the Layouts menu restores',
    Shape: 'ShapeWidenThenUse',
    A: 'layouts.save',
    B: 'agents.spawn',
    Crossing:
      'the third copy of the boot-restore shape: <configDir>/layouts/<slug>.yaml holds "the caller\'s whole agents array" and is restored from the Layouts menu into the same loadAgentsFromSession -> reconcileAgents -> respawnFromRecord path as a saved session. One document shape, three writers, and the composition record named one of them \u2014 which is precisely how a closed chain stays reachable through a second door.',
    ClosedBy:
      'scrubBootDocumentAgents on both providers, the same call the sessions.save pair is closed by',
  },
  {
    Name: 'push.subscribe records an outbound network sink; agents.sendMessage pulls the trigger that makes the host use it',
    Shape: 'ShapeWidenThenUse',
    A: 'push.subscribe',
    B: 'agents.sendMessage',
    Crossing:
      "push.subscribe stores a row; a DIFFERENT subsystem (push.Watch -> onSnapshot -> sendOne) consults that row to issue POST <endpoint> with a VAPID header from the HOST's network position \u2014 Tailscale-reachable, loopback-reachable, cloud-metadata-reachable \u2014 for a tier holding no fetch, no exec, no fs and no config capability. The trigger is the un-blocked -> blocked edge, and the same triage tier holds agents.sendMessage and claude.approve, so it can drive an agent into and out of that state on demand. capspec's excuse reasoned entirely about what the ENDPOINT learns (\"the payload is encrypted to the subscription's own keys\") and never about what the HOST is made to do.",
    ClosedBy:
      'validatePushEndpoint (internal/push/endpoint.go): https only, and no loopback, private, link-local (169.254.169.254) or unique-local host \u2014 the shape a browser PushManager actually produces',
  },
  {
    Name: 'agents.sendMessage injects the instruction and claude.approve resolves the prompt it raises',
    Shape: 'ShapeWidenThenUse',
    A: 'agents.sendMessage',
    B: 'claude.approve',
    Crossing:
      "agents.sendMessage's own excuse is that 'the agent's own tool approvals are the gate', and claude.approve is the RESOLVER of exactly those approvals \u2014 its own entry says so. A tier holding both can tell an agent that may run a shell to run one, and then approve it, without holding terminals.create, sessions.terminalInput, fs.write, git.* or agents.spawn. claude.gate is NOT a prerequisite: gate only ADDS parking, and the agent's own prompts already exist. `decision:\"always\"` persists a standing allow, so subsequent calls of that tool are not parked at all.",
  },
];
