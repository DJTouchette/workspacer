// Package capspec is the small, dependency-free vocabulary shared between the
// bus (which enforces capability grants) and the plugin loader (which validates
// manifests and translates them into grants). Keeping it here avoids a bus↔plugin
// import cycle and keeps the list of filesystem-scoped capabilities in exactly
// one place, so enforcement and validation can never drift apart.
package capspec

import (
	"sort"
	"strconv"
	"strings"
	"unicode"
)

// PathParam maps a capability method to the params field that carries the
// filesystem path it operates on. A method present here is "path-scoped": a
// plugin must declare path roots to be granted it, and the bus confines each
// call to those roots (see the bus's path-containment policy).
//
// This is the single source of truth for "which capabilities touch the
// filesystem". Add a method here the moment it grows a path argument, or it will
// be grantable to plugins without any path confinement.
var PathParam = map[string]string{
	"fs.read":        "path",
	"fs.readImage":   "path",
	"fs.write":       "path",
	"fs.listEntries": "path",
	"fs.listDir":     "path",
	"fs.watch":       "path",
	"fs.unwatch":     "path",
	"search.project": "cwd",
	// providers.listModels RUNS a provider CLI in the caller's cwd. Its excuse
	// used to live in unscopedByDecision and read "cwd picks which project's
	// provider config to READ; the provider resolves the file itself" — one
	// wrong word carrying the whole boundary. opencode does not merely read that
	// directory: it loads and RUNS every `<cwd>/.opencode/plugin/*.js` at
	// startup, before it prints a model list, with no manifest and no other file
	// required. A cwd that only selects data needs no confinement; a cwd that
	// selects an interpreter's plugin directory needs the same containment
	// git.*/replay.open got.
	//
	// `browse`, not `workspace`, for library.list's reason: the Spawn dialog
	// lists a provider's models for the directory the user is ABOUT to spawn in,
	// which by definition is not yet a live agent cwd.
	"providers.listModels": "cwd",
	// Library items (reusable prompts/skills) live in a directory derived from
	// the caller's cwd: `list` walks it, `save` writes a .md file into it, and
	// `remove` deletes one. That is filesystem reach by any other name, and it
	// went unconfined until this entry existed.
	"library.list":   "cwd",
	"library.save":   "cwd",
	"library.remove": "cwd",
	// git.diff reads file contents out of the repository at `cwd`. The field is
	// `cwd`, not `path`: `path` is an optional pathspec interpreted by git
	// *inside* that repo (a repo-relative coordinate, meaningless to resolve
	// against a host root), while `cwd` is the absolute location the command
	// actually runs in — the one a plugin could aim at someone else's checkout.
	"git.diff": "cwd",
	// brief.append writes into <project>/.workspacer/brief.md. The caller's
	// only path input is `project`, and the BASENAME is composed by the
	// provider, so this reaches strictly less than fs.write does within the
	// same root — but it is still a write to a caller-chosen directory, which
	// is exactly the shape this table exists to confine. Entered here rather
	// than excused in unscopedByDecision precisely because "it can only write
	// one filename" is an argument about the file, not about the directory.
	"brief.append": "project",
	// brief.archive is the same shape one verb over: it takes the same `project`
	// directory, composes both .workspacer basenames itself, and moves entries
	// from the brief into the archive beside it. Two files rather than one, both
	// still named by the provider, so the caller-chosen value to confine is again
	// the directory and nothing else.
	"brief.archive": "project",
}

// unscopedByDecision names methods that sit under a path-bearing namespace, or
// carry a path-ish param, yet are deliberately NOT confined by the bus — each
// with the reason, so the next reader doesn't have to re-derive it and the drift
// detector below can stay strict about everything else. Being listed here is a
// decision on the record, not an oversight; [MissingSpec] treats it as
// classified rather than missing.
var unscopedByDecision = map[string]string{
	"agents.spawn": "starting an agent is a separate authorization decision — the cwd picks where a process runs, and confining it would need the spawn paths to learn root containment first (see cmd/brain's TestSpawnStaysDeliberatelyUnscoped)",
	// The reason used to stop at `cwd`, and that was the whole record for a
	// capability taking TWO process identifiers: `shell` is argv[0], handed
	// straight to Command::new / claudemonSessionClient.spawn with no existence
	// check, no PATH resolution and no containment. Combined with a
	// mode-preserving fs.write over an existing executable in the caller's own
	// agent cwd, terminals.create alone was arbitrary host code execution. It is
	// now an ALLOWLIST (the host's login shells) rather than a path scope, because
	// there is no subtree we could confine argv[0] to that the same caller cannot
	// also fill in — see cmd/brain/shellallow.go and lib/shellAllowlist.ts.
	"terminals.create":    "cwd is a process working directory and holding the capability at all is the gate (as agents.spawn); the OTHER caller string, `shell`, is argv[0] and is confined by an ALLOWLIST of the host's login shells rather than by fsRoots — resolveTerminalShell in both providers",
	"terminals.open":      "opens a VISIBLE terminal pane in the desktop and (optionally) runs `command` inside the host's DEFAULT login shell — no caller argv[0]: the shell is the host's, and the command runs under that shell's own tool/PTY rules exactly as a user-typed command would. cwd is a process working directory, so holding the capability is the gate, as terminals.create. Desktop-only (it needs the renderer to surface the pane); a headless brain has no pane to open and simply does not register it",
	"sessions.transcript": "cwd only selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller",
	"files.upload":        "the landing pad for remote-client attachments (/m photos): the caller supplies BYTES and an advisory filename of which only an allowlisted image/pdf extension survives — the directory (os.TempDir()/workspacer-uploads) and basename are chosen by the hub, so there is no caller path to confine. Size-capped (24 MiB decoded) and written 0600. The file only ever ACTS if the caller also references it to an agent via agents.sendMessage — a capability the same triage tier already holds, and one whose excuse (the agent's own tool approvals are the gate) covers reading an uploaded image exactly as it covers any other path a message names",
	// jobs.* — the hub's job system (internal/jobs). A job is PERSISTED argv:
	// upsert carries a shell command, a spawn cwd/prompt, or a capability call
	// verbatim, and run/remove act on that stored authority. There is no
	// subtree to confine any of it to (the terminals.create argument), so the
	// gate is IDENTITY, not paths: every jobs.* registration in cmd/hub wraps
	// its handler in an IsTrusted() check — plugin tokens and view/triage
	// tiers are refused at call time, the host token and operator pairings
	// pass. The CAP_LABELS rows exist so a plugin manifest that declares
	// jobs.* shows an honest consent line before that refusal.
	"jobs.list":    "no params; trusted-only at the handler — the rows disclose stored shell commands and prompts, which is exactly why scoped tiers are refused",
	"jobs.upsert":  "the job spec IS the parameter — persisted argv (shell command / spawn cwd+prompt / capability call). Trusted-only at the handler; spawn actions additionally re-enter agents.spawn over the bus and inherit its clamps",
	"jobs.propose": "the job spec IS the parameter, exactly as jobs.upsert — but the handler disarms it: the row is forced disabled, stamped proposedBy, given a fresh id (so it can never overwrite an approved job), and refused by jobs.run until a human clears the stamp. Same trusted-only gate; it exists because the MCP facade gives operator AGENTS a tool for this method and none for jobs.upsert, so agent-written argv can be reviewed before it is ever armed",
	"jobs.remove":  "an id naming a stored job; trusted-only at the handler",
	"jobs.run":     "an id naming a stored job to fire now; the authority is the stored spec, the gate is the trusted-only handler",
	"jobs.history": "an id naming a stored job; returns run records (output tails included), trusted-only at the handler",
	// The sentence used to stop at "never opened as a path", and it was false:
	// the encoder maps only '/', '\' and ':' to '-', so a cwd of ".." survived
	// verbatim, became a real path COMPONENT, and joined to ~/.claude — one
	// level out of the sandbox this exemption assumes. Both providers now run
	// the encoded name through claudeProjectDirName, which refuses "", "." and
	// "..", so the slug really is a single plain component and the reason below
	// is true rather than aspirational.
	"claude.sessionsForDir": "cwd is encoded into a ~/.claude/projects slug by the provider (claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component); the caller's string is never opened as a path",
	"replay.open":           "confined by the provider to the same workspace roots git.* uses (assertPathAllowed in hubCapabilities.ts), because it cuts a worktree from the repo at cwd",
	// The sentence used to stop at "containment is structural", and the structure
	// was standing on a grant nobody re-consulted. sessionId is not an ownership
	// token: it is CALLER-CHOSEN at open, the entries map is process-global, its
	// only eviction is an explicit replay.close, and replay.* sits outside the
	// bus's per-plugin fsRoots scoping. So a worktree cut while a session was
	// live went on serving that repository's bytes AFTER the session stopped —
	// at which point fs.read on the same directory is refused and a fresh
	// replay.open on it is refused — to any caller that knew the id, and
	// agents.list / sessions.snapshots hand ids out while classified inert. The
	// bus handlers now re-run replay.open's own containment on the entry's
	// recorded ORIGIN cwd before every read/diff/seek (guardReplaySession).
	"replay.read": "the path is a repo-relative coordinate inside a worktree the replay service itself created and keyed by sessionId; containment is structural (resolveInside), and fsRoots would be scoping the wrong namespace — but the WORKTREE's own authorization is re-checked per call, because the grant that authorized replay.open is not a grant that lasts: guardReplaySession re-runs assertPathAllowed on the recorded origin cwd (timelineReplayService.originCwd)",
	"replay.diff": "same as replay.read — a coordinate inside a service-owned worktree, not a host path, with the same per-call re-check of the origin cwd's grant",
	// replay.seek is the WRITE leg of the block above and the only one of the
	// four that had no entry at all. `ops` is not a path-shaped name, its file
	// path lives a level deeper (ops[].input.file_path) behind a NAMED type, and
	// replay.* is not a path-bearing prefix — so nothing on either side could
	// reach it, and "decided and safe" was indistinguishable from "nobody
	// looked" for the one leg that puts caller-supplied bytes on disk.
	"replay.seek": "the ops carry a file_path and content, but both are re-anchored inside the service-owned worktree by containInWorktree (timelineReplayService), which resolves per component and writes the RESULT — the escape it closed was a committed symlink that made the join and the write two different files. Like replay.read/diff it also re-checks the origin cwd's grant per call (guardReplaySession), because the worktree outlives the session that authorized cutting it",
	// The claude.* CONTROL family. None of these carries a path, which is why
	// LooksPathBearing is false for all of them and MissingSpec never asked —
	// and until `mode`, `effort`, `text`, `answers` and `option` entered the
	// shared vocabulary, the params scans could not ask either. They were
	// classified NOWHERE while being, between them, a live approval-policy
	// switch and a second door onto sessions.terminalInput's raw-PTY write.
	"claude.setPermissionMode": "mode is an approval POLICY, not a path: it decides whether the host asks before the agent's next tool call. agents.spawn refuses to let a bus caller start a bypassing agent, so this method — which reaches an ALREADY RUNNING one, including a session the local user started in ask mode — applies the same clamp (assertNoPermissionBypass in hubCapabilities.ts); de-escalating modes stay open, because tightening is not an escalation",
	"claude.setEffort":         "effort reaches a live claude session as the message `/effort <level>` (applyLiveEffort), i.e. exactly the reach agents.sendMessage already has and no more; there is no path and nothing to confine, so holding the capability is the gate",
	"claude.setModel":          "same shape as claude.setEffort: `model` names a model the daemon resolves and `effort` rides the same live-switch endpoint; neither becomes a path or an argv element the caller controls",
	"claude.answer":            "types the answer into the session's PTY, which is sessions.terminalInput's primitive under another name — see the per-param decisions, which say so rather than implying this method is narrower than it is",
	// The excuse names a gate, and the gate is a capability. "The agent's own
	// tool approvals are the gate" is true of a caller that cannot RESOLVE those
	// approvals — and the triage tier, the credential minted for a phone, holds
	// claude.approve. So for that tier the bound this sentence names is not a
	// bound: inject the instruction, then approve the prompt it raises, with
	// decision:"always" persisting a standing allow. claude.gate is not needed
	// (gate only ADDS parking). That pair is a deliberate product decision and is
	// on the record in Compositions() as the one AcceptedIn entry, with a test
	// that fails if any tier acquires both halves of a pair it was not accepted
	// for — which is the guarantee this sentence could not give by itself.
	// agents.reportProgress is agents.sendMessage's primitive — caller text into
	// a running agent's conversation — with the RECIPIENT taken away from the
	// caller. It gets its own entry rather than leaning on the sentence below
	// because the difference is the whole capability: the caller names no
	// session but the one it claims to BE (`callerSessionId`), and the host
	// derives the destination from that session's own parentSessionId.
	"agents.reportProgress": "`note` is prompt text for an agent that is already running — agents.sendMessage's reach — and the containment is that the caller cannot choose WHO reads it. There is no recipient param: the caller supplies `callerSessionId`, the host looks that session up in its own store and delivers to its parentSessionId or refuses, so the only pair this can ever connect is (a tracked session, whatever dispatched it). `callerSessionId` is not a caller value on the path an agent actually uses either — the MCP facade stamps it from the per-request token record's `session:<id>` label, and the hub bus deletes it from every untrusted caller's params (sanitizeReportProgressParams), so a scoped or plugin token cannot name a session at all and lands on the no-identity refusal. Bounded in volume as well as reach: one line, flattened, capped at 500 chars, one per 60s, 20 per session for life",
	"agents.sendMessage":    "text is a prompt for an agent that is already running; there is no path to confine, so holding the capability is the gate. The older wording — 'the agent's own tool approvals are the gate' — named a bound that only holds for a caller which cannot also RESOLVE those approvals, and the triage tier holds claude.approve. See Compositions(): agents.sendMessage + claude.approve is recorded, accepted for triage, and machine-checked against every other tier",
	// The rest of git.*: every one takes a mandatory absolute `cwd` and the
	// desktop provider already contains it to the workspace roots (guardGitCwd).
	// Per-plugin root confinement is the right end state, but two shipped
	// catalog plugins declare git.status / git.numstat with no `paths`, so
	// speccing the namespace today would deny calls that work now. Retire these
	// entries once those manifests declare their roots.
	"git.status":        "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.log":           "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.numstat":       "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.commitDiff":    "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.commitNumstat": "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.stage":         "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.unstage":       "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.commit":        "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	"git.push":          "provider-confined to the workspace roots (guardGitCwd); per-plugin scoping pending a catalog manifest update",
	// sessions.* take a `filename`, not a path. It is a BARE BASENAME inside
	// <configDir>/sessions — never absolute, never a caller-chosen directory —
	// so PathParam is the wrong tool (the bus canonicalizes a PathParam value
	// and contains it against the plugin's roots, and a basename is not
	// absolute, so every call would simply be denied). Both providers instead
	// resolve it against the sessions dir themselves and confine it there:
	// cmd/brain stores.go sessionFilePath and the desktop's
	// sessionService.resolveWithinSessionsDir, held to each other by the
	// `sessionFilenames` block of contracts/path-containment-cases.json. Listed
	// here because `filename` IS path-ish and the classification scan must see a
	// decision rather than silence — that silence is how the two copies drifted
	// far enough apart for one of them to read and unlink through a symlink.
	"sessions.load":   "filename is a bare basename resolved and confined to <configDir>/sessions by both providers (sessionFilePath / resolveWithinSessionsDir); pinned by the corpus's sessionFilenames block",
	"sessions.save":   "same as sessions.load: the filename is derived from the session name by the provider's slug and re-checked by the same resolver",
	"sessions.delete": "same as sessions.load",
	// layouts.* are the sessions.* shape exactly: a caller string that selects a
	// file inside a config store a bus caller can also fs.write into. They were
	// classified nowhere — `id` is not in the params scanner's path-ish set and
	// layouts.* is not a path-bearing prefix — which is the same silence that let
	// sessions.* ship as an unpinned fourth copy of path containment. Their
	// resolver (layoutFilePath) was the FIFTH copy and the only purely lexical
	// one; it now canonicalizes and re-contains like sessionFilePath.
	"layouts.save":   "id is a bare name slugged into <configDir>/layouts/<slug>.yaml and re-contained there by both providers (layoutFilePath / layoutService), never a caller-chosen directory",
	"layouts.delete": "same as layouts.save",
	// config.save writes <configDir>/config.yaml — the file fs.write is refused
	// on by the secret gate in all three containment copies. It takes no path at
	// all; what it needs is not confinement but a HOST-TRUSTED key list, because
	// two of its keys are process identifiers rather than settings:
	// agents.binaries (argv[0] of every spawned agent) and claude.profiles
	// (configDir → CLAUDE_CONFIG_DIR, extraArgs → --dangerously-skip-permissions).
	// Both are stripped from a bus write by dropHostTrusted, pinned by
	// contracts/host-trusted-config-cases.json.
	"config.save": "takes no path; the config file is the provider's own and every key that the host later hands to a process — agents.binaries, claude.profiles, terminal.shell, terminal.shells, editor.terminalCommand, scripts — is stripped from a bus write by dropHostTrusted. That list is held equal to contracts/host-trusted-config-cases.json, so a newly dangerous key cannot be classified on one side only",
	// claude.profiles.* persist a `configDir` that becomes CLAUDE_CONFIG_DIR (the
	// settings.json supplying permissions.allow and hooks) and `extraArgs`. Not
	// confined to roots — there is no subtree we could allow that the same caller
	// cannot fill in with fs.write — but SCRUBBED at write time on both bus
	// providers, so a bus caller cannot plant one for the local user to pick.
	// claude.profiles.remove and notifications.post were classified NOWHERE:
	// neither carries an fs./git. prefix, so LooksPathBearing is false, and the
	// params scan could not reach them either until the vocabulary grew past
	// path-shaped names. `id` selects a row in a single JSON file (no store
	// directory to escape) and `url` is a destination the HOST opens on click —
	// both harmless once written down, which is the point: the record has to
	// distinguish "decided and safe" from "nobody looked".
	"claude.profiles.remove": "id selects a row in the single claude-profiles.json; it is never joined into a path, so there is nothing to confine",
	"notifications.post":     "the only dangerous param is `url`, which the host opens on click through openExternalUrl's scheme allowlist — the same gate the renderer's open-external path uses",
	"claude.profiles.add":    "configDir/extraArgs/mcpItemIds are scrubbed at write time on both bus providers (scrubBypassProfile), so nothing a bus caller persists can carry a CLAUDE_CONFIG_DIR, a permission bypass, or an MCP server definition into a later LOCAL spawn",
	"claude.profiles.update": "same as claude.profiles.add",
	// sessions.terminalInput writes raw bytes into a session's PTY. It was
	// classified NOWHERE — `sessionId` and `data` are not path-shaped, and
	// sessions.* is not a path-bearing prefix — while being, by construction, the
	// most direct process-control primitive on the surface: bytes into a shell
	// are argv of whatever the caller types. That silence also made
	// terminals.create's shell ALLOWLIST look like a boundary it is not, since a
	// caller holding both can spawn /bin/bash and then type anything.
	//
	// There is nothing to confine: a PTY byte stream has no path and no subtree.
	// What the reason has to say honestly is that holding this capability IS the
	// gate, at the same level as terminals.create — and that the allowlist buys
	// its protection only against a caller that does NOT hold this one.
	// claude.approve + claude.gate are an approval-OVERRIDE pair, and neither was
	// classified anywhere: claude.* is not a path-verb prefix, so MissingSpec is
	// false for both, and `decision`/`on` were in no vocabulary, so no scan could
	// ask. Composed with agents.sendMessage they are arbitrary host command
	// execution — gate on, send "run: <cmd>", approve — and agents.sendMessage's
	// own excuse names precisely these approvals as its only bound. For a managed
	// / stream session (the shipping default transport) the gate step is not even
	// needed: post_approve calls submit_managed_decision, which sends allow=true
	// down the adapter's can_use_tool channel.
	//
	// There is nothing to CONFINE — an approval verdict has no path and no
	// subtree — so, exactly as for terminals.create and sessions.terminalInput,
	// holding the capability is the gate. What was missing was saying so, in a
	// form a scan can check, so that the next param either of them grows is
	// visible.
	"claude.approve":           "resolves a tool-approval prompt for a session that is ALREADY running: there is no path and no subtree to confine, so holding the capability is the gate, at the same level as sessions.terminalInput. NOTE that this is the RESOLVER of the approvals agents.sendMessage's own excuse rests on, that claude.gate can arm the parked-hook path it answers, and that the sessionId is ownership-checked on neither provider, so it reaches the local user's own agent too",
	"claude.gate":              "turns the PreToolUse parking gate on or off for a session that is already running. Same shape as claude.approve — no path, no subtree, holding it is the gate — and the two are meant to be read together: gate ON parks every tool call, and claude.approve is what then releases them",
	"claude.signal":            "the signal name is deserialized into a three-variant enum by claudemon (protocol.rs Signal) before anything is sent, so the caller chooses among interrupt/stop/kill and cannot compose a value; the sessionId is not ownership-checked on either provider",
	"claude.handoffBrief":      "writes a deterministic brief the PROVIDER composes into ~/.workspacer/handoffs/<generated>.md — the caller supplies a session id, never the filename and never the directory — so there is no caller path to confine",
	"claude.handoffAgentBrief": "same as claude.handoffBrief, plus it injects the resulting read-this instruction into a live agent; that half is the agents.sendMessage primitive and is bounded the same way",
	"sessions.attachTerminal":  "binds a PTY stream to the caller's connection. No path and no subtree: what it grants is the OUTPUT side of sessions.terminalInput, so holding it is the gate at the same level",
	// ── HUB-NATIVE capabilities ───────────────────────────────────────────
	// The three registries are not two. cmd/hub registers seven capabilities of
	// its own with RegisterLocal, and NOTHING looked at them: they are absent
	// from both providers' method lists, so TestBrainMethodsAllClassified and
	// TestDesktopCapabilitiesAllClassified never asked; they carry no
	// fs./search./library./git./providers. prefix, so MissingSpec was false; and
	// with Classified false, RegisterPluginToken did not refuse them and no
	// CAP_LABELS row warned about them. layout.set was one of them, and it is a
	// second door onto every clamp agents.spawn applies.
	"layout.set":             "the shared workspace document, which the hub deliberately does not interpret — except for the four per-agent fields that stop being DESCRIPTION and become ARGUMENTS TO A SPAWN when the desktop adopts the document on its next launch (skipPermissions, permissionMode, profileId, mcpItemIds; App.tsx adoptSharedLayout → reconcileAgents{respawnStopped} → respawnFromRecord → the LOCAL spawnClaude IPC, which scrubs nothing). Those four are stripped from a NON-TRUSTED write by layout.scrubAdoptedSpawnFields, exactly as agents.spawn strips them from a bus spawn; everything else round-trips byte for byte. There is no path to confine — the document names cwds but never opens one",
	"push.subscribe":         "the endpoint is a URL the hub itself POSTs to, so it is a request the host makes on the caller's behalf rather than a path or an argv. This reason used to stop at what the ENDPOINT learns (the payload is encrypted to the subscription's own keys) and said nothing about what the HOST is made to DO — and the host's network position is Tailscale-reachable, loopback-reachable and cloud-metadata-reachable, for a TRIAGE tier holding no fetch, no exec, no fs and no config capability, on a trigger that same tier can pull at will. Bounded now by validatePushEndpoint (https only, no loopback/private/link-local/unique-local host: the shape a browser PushManager actually produces), plus RPCSubscribeAs recording WHICH credential asked so revoking a phone's token revokes its notifications",
	"push.unsubscribe":       "the endpoint selects a stored subscription row to delete — the narrowing direction, and never joined into a path",
	"push.revoke":            "deletes a stored subscription by id. Operator-only by construction (absent from both scoped tiers), and narrowing",
	"sessions.terminalInput": "writes raw bytes into an existing session's PTY: there is no path and no subtree to confine, so holding the capability is the gate, exactly as for terminals.create. NOTE that this makes terminals.create's shell allowlist a boundary only against callers that do not ALSO hold this method — allowlisted /bin/bash plus typed bytes is full argv[0] freedom — and that the sessionId is not ownership-checked on either provider, so it reaches the local user's own agent PTY too",
}

// inertMethods are the capabilities that carry NO caller value which becomes
// anything on the host: no path, no filename, no argv, no bytes into a process,
// no approval verdict, no destination the host opens. They are classified HERE,
// with a reason each, rather than by silence.
//
// The distinction matters because the ONLY method-level drift detector this
// package had was [MissingSpec], a name-prefix heuristic over
// {fs., search., library., git.}. It returns false for every claude.*,
// sessions.*, config.*, layouts.*, app.*, analytics.*, providers.* and replay.*
// method no matter what that method does — so 27 of the 73 registered
// capabilities were classified nowhere at all, and six of those were ones the
// app's OWN consent list (CAP_LABELS in pluginPermissions.ts) marks
// `sensitive: true`. claude.approve and claude.gate — an approval-OVERRIDE pair
// that composes with agents.sendMessage into arbitrary host command execution —
// were two of them, and a brand-new `claude.autoApprove` capability, registered
// and dispatched and byte-for-byte claude.approve under another name, could be
// added with the whole Go module green.
//
// So "somebody looked at this" is now a REQUIREMENT rather than an accident of
// naming: [Classified] is PathParam ∪ unscopedByDecision ∪ this map, and
// TestBrainMethodsAllClassified / TestDesktopCapabilitiesAllClassified hold the
// registries to it. A read-only method belongs here; a method that takes a value
// the host acts on belongs in unscopedByDecision with a per-param decision.
//
// The bar for an entry is one sentence naming what the caller may supply and why
// none of it reaches a sink. "It is read-only" on its own is the shrug this map
// replaced.
var inertMethods = map[string]string{
	"agents.list":                "no caller params at all; it returns the session rows the provider already holds",
	"agents.close":               "one sessionId selecting an existing session row, and the effect is REMOVAL — the narrowing direction, like sessions.detachTerminal and push.unsubscribe. It composes no path and no argv. Its one side effect beyond forgetting the row is claudemonSessionClient.close (viewers stopped + SIGTERM), which is exactly claude.signal's own reach and is skipped entirely for a row that had already ended. It cannot be aimed at a WORKING session at all: the provider refuses one, because hiding a running agent from list_agents while it kept spending is the only outcome worse than the lingering row this replaces",
	"agents.reparent":            "two session ids selecting existing session rows, and NOTHING else — no path, no argv, no caller text. The effect is internal routing: it re-points the `parentSessionId` field the provider itself reads when it decides which manager to wake, from a retiring manager to its successor, and every message that later travels that route is composed by the host from the worker's own output (buildFleetMessage), exactly as agents.notifyWhen's is. Its one disclosure — the successor now receives reports about workers it did not dispatch — is available to the same operator tier through sessions.conversation already. The provider refuses a destination no wake can reach (unknown, ended, or not a supervisor), so it cannot be used to silence a fleet either",
	"agents.orphans":             "no caller params at all. It returns the DEAD parents that still have live children — the `fromSessionId` agents.reparent needs when the manager it replaces crashed and wrote no handoff file. What it discloses about each is a label, a cwd, a time of death and the ids of its live children, and every one of those is on the caller's own agents.list already for a session that is still running: the tombstone only makes them outlive the row by as long as something it dispatched is still alive. It performs no move and names no destination — an id it hands back is an ARGUMENT for a separate, refusable call, which is precisely why the discovery is a read rather than a no-argument mode on agents.reparent (the host would have to guess which dead manager the caller is replacing, and a wrong guess re-points a live worker's wakes silently)",
	"agents.notifyWhen":          "two session ids selecting existing rows plus three NUMBERS (tokens, usd, idleSeconds), each coerced with Number() and range-checked before it is stored. Nothing composes a path, an argv or a query, and the call starts nothing: it records an intention to send a message LATER, and the message body is composed entirely by the host (buildFleetMessage over the provider's own snapshot fields) with no caller text in it. Everything it can ever report — cumulative tokens, cost, idle time — is already in the sessions.snapshot the VIEW tier reads, so it discloses nothing new; what it removes is the caller's need to poll for it",
	"analytics.recent":           "the only params are a row limit and a time window, both coerced to numbers before they reach the store; nothing composes a path, an argv or a query the host runs",
	"analytics.summary":          "same as analytics.recent — numeric window only",
	"app.getCwd":                 "no params; returns the provider's own working directory",
	"brain.info":                 "no params; returns only the brain's own registration scope. It exists so `workspacer status` can ask whether a brain is on the bus at all — a question the previous probe (app.getCwd, which the DESKTOP registers) answered wrong whenever the desktop was running",
	"app.supervisorHome":         "no params. It DOES create ~/.workspacer and a README there, but both are fixed literals the provider composes — no part of the location comes from the caller",
	"claude.listModels":          "no params; the answer is the provider's own model catalog",
	"claude.profiles.list":       "no params; the mutating siblings (add/update/remove) carry their own decisions",
	"config.get":                 "no params. It hands back the whole config document, which is a disclosure decision rather than a confinement one: the keys a bus caller must not WRITE are the host-trusted list, and the keys it must not READ would be a different mechanism (there are none today — the config holds no credential; those live in remote-token, tokens.json and the plugin .settings.json files, none of which is in config.yaml)",
	"config.getPath":             "no params; returns the config file's location, which the caller can already derive from the platform rules",
	"config.reload":              "no params; re-reads the provider's own config file from disk",
	"layouts.list":               "no params; the entry names come from a readdir of the layouts store and are re-contained by the store resolver before anything is opened",
	"providers.checkAll":         "no params; it stats a fixed set of provider binary names against the process's own PATH, and the answer is a boolean per provider rather than a path the caller chose",
	"sessions.conversation":      "sessionId selects an existing session row; it is never joined into a path (the transcript location is derived by the provider) and never becomes argv",
	"sessions.detachTerminal":    "sessionId only, and the effect is to STOP streaming — the narrowing direction. The attach half carries its own decision",
	"sessions.list":              "no params; entry names come from a readdir of the sessions store and are re-contained by the store resolver",
	"sessions.recent":            "a numeric limit over rows the provider already holds",
	"sessions.snapshot":          "sessionId selects an existing session row; the snapshot is assembled by the provider from state it already holds",
	"sessions.snapshots":         "no params at all; it returns the whole snapshot set the provider already holds in memory, with nothing composed from a caller string",
	"sessions.terminalKeepalive": "sessionId only; it refreshes an idle timer and moves no bytes",
	"sessions.terminalResize":    "sessionId plus cols/rows, both coerced to integers before they reach the PTY ioctl; there is no string the host acts on",
	"replay.close":               "an opaque handle the provider minted; closing it releases the provider's own reader",
	"layout.get":                 "no params; returns the shared workspace document the hub already holds. A disclosure decision rather than a confinement one, like config.get: the document names agent cwds and pane URLs, and the one credential that ever rode in it (a plugin pane's busToken) is redacted on the way in and out",
	"fleet.quiescence":           "no params at all. It reports whether this machine's fleet is at rest, and a named blocker per reason when it is not. Every value in the answer is derived by the hub from state it already holds or already serves — session rows (the same ones agents.list and sessions.snapshots return to this tier), the job SCHEDULE (times and action kinds, never a spec: the argv stays behind the trusted-only jobs.* RPCs), the peer names federation.peers already discloses, and a description of each live bus connection that names no credential and no address. It composes nothing: the answer is a reading, and every action anyone takes on it happens outside this process. Admitted to the VIEW tier (authtoken viewMethods) for the same reason federation.peers is — the phone and the web renderer can already derive most of it by polling the snapshot feed they receive anyway, one call at a time",
	"federation.peers":           "no params; returns each configured peer hub's name, connected bit, and last-seen timestamp. Registered only when federation is configured. A disclosure decision: the peer NAMES are already stamped on every forwarded agent.* event the same callers receive, and the connected bit is the tombstone signal hub.peer.* broadcasts anyway. Admitted to the VIEW tier (authtoken viewMethods) so the /m PWA and web renderer can seed the federated fleet — the same tier already receives the stamped events it explains",
	"plugins.tools":              "no params; returns the consented facade-tool metadata (tool name/description/schema + the plugin bus method each forwards to) for enabled plugins. A disclosure decision, not a confinement one: the same method names are already visible to any caller that can invoke them, and the pin narrowing (Manager.ConsentedTools) means nothing is listed that the bus would refuse to let the plugin register. Deliberately NOT in any scoped tier — the MCP facade reads it over its trusted connection and applies its own per-token plugin grants",
	"push.key":                   "no params; returns the VAPID PUBLIC key, which every subscriber needs and which discloses nothing",
	"push.list":                  "no params; lists stored subscriptions. Operator-only by construction — it appears in neither scoped tier",
	"push.test":                  "no params; sends one canned notification to every registered subscription so a phone can answer \"is push reaching me at all\" without reading hub logs. Nothing about the message is caller-supplied — title and body are literals in RPCTest — so there is no text a caller can put on someone's lock screen, which is the shape that made a forged agent.snapshot worth closing. It is a SEND trigger available to the triage tier, bounded by the same recipient set every other push already goes to: subscriptions this hub stored, still-valid credential, endpoint already validated by validatePushEndpoint at subscribe time. The tier that may subscribe may already provoke real pushes by approving or answering, so this adds no reach it lacked — only a way to test it deliberately",
	// NOT here, deliberately: "notify.post" and "agents.kill". Both appear in the
	// renderer's plugin-consent list (pluginPermissions.ts) and are registered by
	// NO provider — not the brain's registry, not hubCapabilities.ts through either
	// door, not cmd/hub's RegisterLocal. An inert record on a name nobody serves is
	// worse than no record: MissingSpec reports it specced, so the day somebody
	// implements it the bus grants it unconfined and no guard fires.
	// TestInertMethodsAreActuallyRegistered keeps them out.
	// The consent list advertising them is a separate, real drift — the user is
	// asked to grant "Terminate agents" for a capability that does not exist — and
	// belongs to whoever owns that surface, not to this table.
}

// Classified reports whether SOMEBODY has decided what this method is: the bus
// confines its path (PathParam), somebody wrote down why it is safe unconfined
// (unscopedByDecision), or it carries nothing the host acts on (inertMethods).
// It is deliberately not derived from the method's NAME — that heuristic is
// [MissingSpec], and it is exactly what let 27 capabilities ship unexamined.
func Classified(method string) bool {
	if _, ok := PathParam[method]; ok {
		return true
	}
	if _, ok := unscopedByDecision[method]; ok {
		return true
	}
	_, ok := inertMethods[method]
	return ok
}

// MissingClassification is the fail-open condition [Classified] guards: a
// registered capability nobody has said anything about. Callers should treat it
// as a build failure, not a runtime denial — the point is that the decision gets
// made before the method ships, not that the bus guesses at call time.
func MissingClassification(method string) bool { return !Classified(method) }

// InertReason returns the recorded reason a method carries nothing the host acts
// on, for tests that report what IS classified when they report what isn't.
func InertReason(method string) (string, bool) {
	r, ok := inertMethods[method]
	return r, ok
}

// InertMethods lists every method on the inert record.
func InertMethods() []string {
	out := make([]string, 0, len(inertMethods))
	for m := range inertMethods {
		out = append(out, m)
	}
	sort.Strings(out)
	return out
}

// ParamKind is what a caller-supplied value BECOMES on the host. "Is it a
// path?" was the only question this package used to ask, and config.save is the
// proof that it is the wrong one: that capability takes no path at all, and two
// of its keys are argv[0] of every agent the host spawns. A caller string can
// become a filesystem location, an executable, an argv element, a byte stream
// into a live process, an environment, a URL, a port, a stored-file name, an id
// that resolves to any of those, or a pattern someone's regex engine compiles.
// Naming which one is what makes an excuse reviewable instead of a shrug.
type ParamKind string

const (
	// KindPath — a filesystem location opened, walked or written by the host.
	KindPath ParamKind = "path"
	// KindFilename — a bare name that a provider joins into a store directory
	// (sessions/, layouts/): path containment by another route.
	KindFilename ParamKind = "filename"
	// KindExecutable — argv[0]: something the host will exec.
	KindExecutable ParamKind = "executable"
	// KindArgv — an element of a command line the host builds (flags, refs).
	KindArgv ParamKind = "argv"
	// KindShell — bytes delivered into a live process/PTY, i.e. whatever the
	// program on the other end will interpret.
	KindShell ParamKind = "shell"
	// KindEnv — environment handed to a spawned process.
	KindEnv ParamKind = "env"
	// KindURL — a location the host fetches, opens, or downloads from.
	KindURL ParamKind = "url"
	// KindPort — a socket the host binds or dials.
	KindPort ParamKind = "port"
	// KindID — a handle that RESOLVES into one of the above (a library item id
	// carrying command/args/env, a profile id carrying CLAUDE_CONFIG_DIR).
	KindID ParamKind = "id"
	// KindRegex — a pattern compiled or handed to a matcher.
	KindRegex ParamKind = "regex"
	// KindPermission — a value that changes what the host will do WITHOUT
	// asking. Not a path, not argv, not bytes: `permissionMode: "yolo"` runs no
	// code of its own, it removes the approval that gates every tool call the
	// agent makes from then on. This kind exists because the previous list had
	// no slot for it, and a param with no slot is a param nobody writes a
	// decision for: agents.spawn CLAMPS exactly these values for a bus caller
	// and claude.setPermissionMode handed the same escalation back through a
	// second door, unclassified.
	KindPermission ParamKind = "permission"
	// KindInert — the name is in the vocabulary, but on THIS method the value
	// provably never becomes any of the above. Recording it is a decision, not
	// an omission: the Why has to say what it becomes instead.
	KindInert ParamKind = "inert"
)

var knownKinds = map[ParamKind]bool{
	KindPath: true, KindFilename: true, KindExecutable: true, KindArgv: true,
	KindShell: true, KindEnv: true, KindURL: true, KindPort: true,
	KindID: true, KindRegex: true, KindInert: true, KindPermission: true,
}

// KnownKind reports whether k is one of the classification kinds above.
func KnownKind(k ParamKind) bool { return knownKinds[k] }

// dangerousParams is the SHARED vocabulary of caller-param names that reach
// something dangerous, with what each name usually becomes. It used to live in
// this package's _test.go file, which meant exactly one loader (the desktop
// scan) could see it and the Go providers were scanned by nobody.
//
// A name in here forces a decision wherever a handler destructures it: either
// it is the method's PathParam field (the bus confines it), or the method's
// entry in unscopedParams says what it is and why it is safe. A name NOT in
// here is invisible to every scan — which is how `shell`, `configDir`,
// `filename` and `mcpItemIds` each shipped unclassified — so the bar for
// leaving something out is "the value cannot become code, a file, or a
// network destination", not "it doesn't look like a path".
//
// `sessionId` is deliberately absent: it is a handle to something the daemon
// already owns, carried by two dozen read/control methods, and adding it would
// drown the signal rather than sharpen it.
var dangerousParams = map[string]ParamKind{
	// Filesystem locations.
	"path": KindPath, "cwd": KindPath, "dir": KindPath, "directory": KindPath,
	"filePath": KindPath, "root": KindPath, "paths": KindPath,
	// claude.profiles.*'s persisted CLAUDE_CONFIG_DIR — the directory supplying
	// claude's settings.json, i.e. permissions.allow and hooks. `dir` was in this
	// list and `configDir` was not, and the scan matches names exactly.
	"configDir": KindPath, "workdir": KindPath,
	// A bare name a provider joins into a store directory. `filename` was the
	// blind spot that let sessions.load/save/delete ship as a FOURTH,
	// unclassified copy of path containment; `name` is the same value one step
	// earlier, before the slug that produces it.
	"filename": KindFilename, "fileName": KindFilename, "file": KindFilename,
	"name": KindFilename,
	// argv[0]. `shell` is terminals.create's, handed to Command::new with no
	// existence check, no PATH resolution and no containment.
	"shell": KindExecutable, "command": KindExecutable, "cmd": KindExecutable,
	"binary": KindExecutable, "binaries": KindExecutable, "bin": KindExecutable,
	"executable": KindExecutable, "interpreter": KindExecutable, "program": KindExecutable,
	// argv[1:]. `extraArgs` is how `--dangerously-skip-permissions` travels;
	// `hash`/`ref`/`rev` land in git argv, where a leading '-' is an option.
	"args": KindArgv, "argv": KindArgv, "extraArgs": KindArgv, "flags": KindArgv,
	"hash": KindArgv, "ref": KindArgv, "rev": KindArgv,
	// Bytes into a live process. Not a path, but the most direct process-control
	// class on the surface: bytes into a shell are argv of whatever is typed.
	//
	// `text`, `answers` and `option` are that same primitive under three other
	// names. claude.answer types each of them into the session's PTY verbatim
	// (`input(sessionId, text + "\r")` on BOTH providers — byte-for-byte what
	// sessions.terminalInput does with `data`), and the sessionId may be a
	// terminals.create shell, so a caller holding only claude.answer types into
	// /bin/bash. `effort` reaches a live agent as the message `/effort <level>`.
	// None of the four was in this list, so no scan on either provider could
	// ever demand a decision for them.
	// `note` is the same class as `text` one more name over: agents.reportProgress
	// delivers it into a running agent's conversation, where it is read as
	// instruction. It is in the vocabulary rather than excused as "just a status
	// line" because the NAME is the thing a scanner sees, and the next method to
	// grow a `note` may not flatten, cap and wrap it the way that one does.
	"data": KindShell, "bytesB64": KindShell, "stdin": KindShell,
	"script": KindShell, "keys": KindShell, "text": KindShell,
	"answers": KindShell, "option": KindShell, "effort": KindShell,
	"note": KindShell,
	// Approval policy of a process that is ALREADY running. agents.spawn clamps
	// `skipPermissions` and `permissionMode` off for a bus caller on both
	// providers; `mode` is the same value arriving after the fact, through
	// claude.setPermissionMode, and it was in neither the vocabulary nor any
	// decision table.
	"mode": KindPermission, "permissionMode": KindPermission,
	"skipPermissions": KindPermission,
	// …and the RESOLVER of those approvals, which was the biggest hole of the
	// three. `decision` is claude.approve's "yes"|"no"|"always": claudemon maps
	// it to {"decision":"approve"} on Claude Code's PreToolUse hook stdout, or
	// straight down a managed adapter's can_use_tool channel. `on` is
	// claude.gate's switch, which ARMS the parked-hook path a caller can then
	// resolve. KindPermission is the definition this package already gives —
	// "a value that changes what the host will do WITHOUT asking" — and
	// agents.sendMessage's own excuse rests on exactly these approvals ("what
	// bounds it is the agent's own tool approvals"), so leaving them
	// unclassified made that sentence unfalsifiable.
	"decision": KindPermission, "on": KindPermission,
	// Environment of a spawned process.
	"env": KindEnv, "envVars": KindEnv, "environment": KindEnv,
	// Network destinations the host opens or fetches.
	"url": KindURL, "uri": KindURL, "endpoint": KindURL, "href": KindURL,
	"webhook": KindURL, "port": KindPort,
	// Ids that RESOLVE into one of the above.
	"id": KindID, "itemId": KindID, "mcpItemIds": KindID, "profileId": KindID,
	// An id that resolves into WHO THE CALLER IS rather than what it is acting
	// on — agents.reportProgress derives its recipient from it. Listed so a
	// second method that ever accepts one has to say where it comes from; on the
	// path an agent uses, it comes from the credential, not the caller.
	"callerSessionId": KindID,
	// A patch WRAPPER: claude.profiles.update carries its configDir/extraArgs/
	// mcpItemIds inside `updates`, and a scanner that only saw the wrapper name
	// (the desktop's does — the object is passed whole) saw a param the
	// vocabulary did not know, so the decision recorded for it was consulted by
	// nobody. See TestEveryParamDecisionNamesAParamAScannerCanSee.
	"updates": KindArgv,
	// The same wrapper problem one namespace over: replay.seek's `ops` is a
	// []ReplayOp, and the path it writes to lives at ops[].input.file_path with
	// caller-supplied ops[].input.content beside it. Both scanners see the
	// wrapper and nothing else (the desktop's annotation is a NAMED type), so
	// the write leg of replay.* was the only one of the four with no entry
	// anywhere — while being the only one that writes caller bytes at a
	// caller-named path.
	"ops": KindPath,
	// Patterns someone compiles or hands to a matcher.
	"query": KindRegex, "pattern": KindRegex, "regex": KindRegex, "glob": KindRegex,
}

// dangerousParamsFold indexes the vocabulary by lowercased name, because ONE of
// the two scanners matches names the way its language does. encoding/json binds
// a caller key to a struct field case-insensitively, so a field tagged `Env`, or
// a field with no tag at all named `Env`, receives the caller's "env" — while an
// exact-spelling lookup against the map above reports "not in the vocabulary"
// and the param is classified by nobody. (JavaScript destructuring is
// case-SENSITIVE, so the desktop scan keeps using the exact lookup.)
var dangerousParamsFold = func() map[string]string {
	out := map[string]string{}
	for p := range dangerousParams {
		lower := strings.ToLower(p)
		// Deterministic winner if two spellings ever fold together, so the
		// canonical name a scanner reports cannot depend on map order.
		if prev, dup := out[lower]; !dup || p < prev {
			out[lower] = p
		}
	}
	return out
}()

// DangerousKind reports what a caller param of this NAME usually becomes, and
// whether the vocabulary knows it at all. Scanners use it to decide which
// destructured fields demand a classification.
func DangerousKind(param string) (ParamKind, bool) {
	k, ok := dangerousParams[param]
	return k, ok
}

// DangerousKindFold is DangerousKind for a scanner whose language matches
// caller keys case-insensitively (encoding/json). It returns the CANONICAL
// spelling as well, because the decision tables are keyed by that spelling: a
// handler binding `Env` has to be checked against the decision recorded for
// `env`, or the fold would find the danger and then look its excuse up under a
// name nothing records.
func DangerousKindFold(param string) (canonical string, kind ParamKind, ok bool) {
	// Exact first, then the fold — encoding/json's own precedence. It matters
	// because the vocabulary carries two spellings that fold together
	// (`filename` and `fileName`), and answering `fileName` for a handler that
	// literally binds `filename` would look the decision up under the spelling
	// nothing records.
	if kind, ok := dangerousParams[param]; ok {
		return param, kind, true
	}
	canonical, ok = dangerousParamsFold[strings.ToLower(param)]
	if !ok {
		return "", "", false
	}
	return canonical, dangerousParams[canonical], true
}

// FoldSpellings lists the vocabulary spellings a caller key of this name would
// bind to, exact first. More than one is possible (`filename`/`fileName`), and a
// decision may be recorded under any of them.
func FoldSpellings(param string) []string {
	var out []string
	lower := strings.ToLower(param)
	for name := range dangerousParams {
		if name != param && strings.ToLower(name) == lower {
			out = append(out, name)
		}
	}
	sort.Strings(out)
	if _, ok := dangerousParams[param]; ok {
		out = append([]string{param}, out...)
	}
	return out
}

// ClassifyParamFold is ClassifyParam for a scanner whose language binds caller
// keys case-insensitively. It consults every vocabulary spelling the key would
// bind to and returns the first classified answer, with the spelling that
// carried it, so a failure message names something a reader can grep for.
func ClassifyParamFold(method, param string) (ParamStatus, ParamDecision, string) {
	spellings := FoldSpellings(param)
	if len(spellings) == 0 {
		spellings = []string{param}
	}
	for _, s := range spellings {
		if status, d := ClassifyParam(method, s); status != ParamUnclassified {
			return status, d, s
		}
	}
	return ParamUnclassified, ParamDecision{}, spellings[0]
}

// DangerousParamNames lists the vocabulary, for tests that want to assert it
// hasn't silently shrunk. Sorted, so a fixture diff reads the same on every run.
func DangerousParamNames() []string {
	out := make([]string, 0, len(dangerousParams))
	for p := range dangerousParams {
		out = append(out, p)
	}
	sort.Strings(out)
	return out
}

// ── The denylist's blind side ──────────────────────────────────────────────
//
// dangerousParams is a DENYLIST, and a denylist of names is only as good as the
// namer's imagination: `entrypoint`, `exe`, `launcher` and `shellPath` are all
// argv[0] on terminals.create and all of them sailed through, because none of
// them is spelled `shell` or `command`.
//
// The airtight fix is to invert it — demand a decision for EVERY param either
// provider binds. We did not, and the reason is that it would demand written
// decisions for several hundred inert fields (`sessionId`, `limit`, `agentId`,
// `title`, every snapshot field the brain re-binds) on the first run. A guard
// that lands as a two-hundred-line chore is a guard someone deletes or
// blanket-excuses, and a blanket excuse is where this package started.
//
// So: keep the denylist for what a param IS, and add a SHAPE heuristic for what
// its name looks like. A param name whose tokens include an exec/argv/path/
// network stem, and which the vocabulary does not know, fails — with the
// instruction to add it to dangerousParams (if it is one of those things) or to
// knownInertParams with a reason (if the shape is a coincidence). That covers
// the synonym class — every one of the four above — at the cost of a handful of
// coincidences, which are enumerated below rather than guessed at.

// paramStems are the name fragments that mean "this value becomes code, a file,
// or a destination". Matched per TOKEN (camelCase / snake_case / dotted), so
// `shellPath`, `exec_path` and `launcher` all hit while `sessionId` does not.
var paramStems = map[string]bool{
	// filesystem
	"path": true, "paths": true, "dir": true, "dirs": true, "directory": true,
	"folder": true, "file": true, "filename": true, "cwd": true, "root": true,
	"workdir": true,
	// exec
	"cmd": true, "command": true, "exec": true, "executable": true, "exe": true,
	"shell": true, "bin": true, "binary": true, "binaries": true, "launch": true,
	"launcher": true, "entrypoint": true, "program": true, "script": true,
	"interpreter": true, "spawn": true, "run": true,
	// argv / environment
	"argv": true, "arg": true, "args": true, "argument": true, "arguments": true,
	"flag": true, "flags": true, "env": true, "environment": true,
	// network destinations
	"url": true, "uri": true, "href": true, "endpoint": true, "webhook": true,
	"port": true, "socket": true,
}

// knownInertParams are names whose SHAPE trips paramStems but whose value
// provably becomes none of those things on the surfaces that bind them. Each
// carries a reason for the same rationale the ParamDecision table does: an
// unexplained entry here is how the heuristic gets quietly emptied out. Every
// entry is a name a scanner actually found — the list is evidence, not
// anticipation.
var knownInertParams = map[string]string{}

// SuspiciousUnknownParam reports a param name that LOOKS like it becomes code,
// a file or a network destination but is not in the shared vocabulary, so no
// scanner would ever demand a decision for it. Fail closed on true: either the
// name belongs in dangerousParams, or in knownInertParams with the reason it is
// a coincidence.
//
// The known-check is fold-insensitive so the Go scanner (whose json binding is)
// gets the same answer as the desktop one.
func SuspiciousUnknownParam(param string) bool {
	if param == "" {
		return false
	}
	if _, _, known := DangerousKindFold(param); known {
		return false
	}
	if _, inert := knownInertParams[param]; inert {
		return false
	}
	for _, tok := range paramTokens(param) {
		if paramStems[tok] {
			return true
		}
	}
	return false
}

// RatchetError implements the coverage-floor rule BOTH provider scans are held
// to, in one place so the two cannot ratchet differently. It returns "" when
// observed == floor, and an explanation otherwise.
//
// A floor is a ratchet or it is nothing. `if flagged < 10` (with the true value
// 38) let a 66% collapse of the brain scan pass silently: delete most of the
// vocabulary, or break the AST walk for every handler shape but one, and the
// suite stays green. Requiring EQUALITY means a real drop fails, and so does an
// undeclared rise — the second half being what keeps the number honest, since a
// floor nobody updates drifts back into meaninglessness the first time the
// surface grows.
func RatchetError(scan string, observed, floor int) string {
	switch {
	case observed == floor:
		return ""
	case observed < floor:
		return "the " + scan + " scan classified " + strconv.Itoa(observed) + " dangerous params, but its recorded floor is " +
			strconv.Itoa(floor) + ". Coverage went DOWN: either the vocabulary shrank, the parse went blind to a handler shape, " +
			"or a capability stopped binding a param. Find out which before touching this number."
	default:
		return "the " + scan + " scan classified " + strconv.Itoa(observed) + " dangerous params and its recorded floor is " +
			strconv.Itoa(floor) + ". Coverage went UP — good — but the floor has to move with it (set it to " + strconv.Itoa(observed) +
			"), or the next collapse back down to " + strconv.Itoa(floor) + " passes."
	}
}

// InertParamReason returns the recorded reason a stem-shaped name is inert.
func InertParamReason(param string) (string, bool) {
	r, ok := knownInertParams[param]
	return r, ok
}

// ParamStems lists the shape heuristic's stems, sorted. Pinned by the same
// fixture as the vocabulary: emptying this map is the quiet way to switch the
// synonym check off, and it would leave every scan green.
func ParamStems() []string {
	out := make([]string, 0, len(paramStems))
	for s := range paramStems {
		out = append(out, s)
	}
	sort.Strings(out)
	return out
}

// InertParamNames lists the heuristic's escape hatch, for a test that holds each
// entry to a written reason.
func InertParamNames() []string {
	out := make([]string, 0, len(knownInertParams))
	for p := range knownInertParams {
		out = append(out, p)
	}
	sort.Strings(out)
	return out
}

// paramTokens splits a param name into lowercase words on camelCase humps and
// on the separators the two languages use (_, -, ., digits). `shellPath` →
// [shell path]; `mcp_server_url` → [mcp server url]; `argv0` → [argv].
func paramTokens(param string) []string {
	var tokens []string
	var cur strings.Builder
	flush := func() {
		if cur.Len() > 0 {
			tokens = append(tokens, strings.ToLower(cur.String()))
			cur.Reset()
		}
	}
	runes := []rune(param)
	for i, r := range runes {
		switch {
		case r == '_' || r == '-' || r == '.' || r == ' ' || unicode.IsDigit(r):
			flush()
		case unicode.IsUpper(r):
			// A hump starts a token, except inside a run of capitals (URLPath →
			// [url path]): break before the LAST capital of such a run.
			prevUpper := i > 0 && unicode.IsUpper(runes[i-1])
			nextLower := i+1 < len(runes) && unicode.IsLower(runes[i+1])
			if !prevUpper || nextLower {
				flush()
			}
			cur.WriteRune(unicode.ToLower(r))
		default:
			cur.WriteRune(r)
		}
	}
	flush()
	return tokens
}

// ParamDecision is one param's classification: what it becomes on this
// particular method, and the written reason it is safe unconfined. Why is
// mandatory — a decision without one is an oversight wearing a decision's
// clothes, which is precisely what the per-METHOD excuse used to be.
type ParamDecision struct {
	Kind ParamKind
	Why  string
}

// ParamStatus is the outcome of classifying one (method, param) pair. The zero
// value is ParamUnclassified, so a scanner that forgets to check fails closed.
type ParamStatus string

const (
	// ParamUnclassified — nobody has said what this param is. Fail closed.
	ParamUnclassified ParamStatus = "unclassified"
	// ParamScoped — it IS the method's PathParam field; the bus confines it to
	// the caller's granted roots.
	ParamScoped ParamStatus = "scoped"
	// ParamExcused — deliberately unconfined, with a kind and a written reason.
	ParamExcused ParamStatus = "excused"
)

// unscopedParams records, PER (METHOD, PARAM), what the caller's value becomes
// and why it is safe without bus confinement.
//
// The drift detector used to ask only `_, excused := unscopedByDecision[name]`,
// which is a per-METHOD excuse: once a capability was listed for ONE param, no
// other param on it could ever be flagged, no matter what the scanner learned
// to see. That is how terminals.create's `shell` hid behind its `cwd` excuse,
// how config.save's agents.binaries and claude.profiles.add's configDir stayed
// structurally invisible, and why teaching the scanner about agents.spawn's
// `mcpItemIds` changed nothing at all: the method was already excused, for a
// different field. A capability that GROWS a dangerous param must now fail the
// detector until that param is classified here, scoped in PathParam, or
// refused.
var unscopedParams = map[string]map[string]ParamDecision{
	"agents.spawn": {
		"cwd":        {KindPath, "the working directory of a process the caller is already authorized to start; holding agents.spawn is the gate, and confining it needs the spawn paths to learn root containment first (TestSpawnStaysDeliberatelyUnscoped)"},
		"mcpItemIds": {KindID, "each id resolves through libraryService -> toClaudeEntry -> buildSessionMcpConfig into a --mcp-config entry whose command/args/env come verbatim from a library item, pre-approved by --allowedTools mcp__<id>; the BUS path therefore forces it to nil (busMcpItemIds / spawn_bypass) and only a locally-initiated spawn honours a selection"},
		"profileId":  {KindID, "resolves to a stored profile whose configDir becomes CLAUDE_CONFIG_DIR and whose extraArgs become argv, so it is the other way to smuggle a bypass; both bus providers scrub the RESOLVED profile before spawning (scrubProfileBypass in hubCapabilities.ts, scrubBypassProfile in the brain — see spawn_bypass_test.go). One verified exception: the hub router (internal/bus sanitizeSpawnParams) strips this field unless the caller's token record grants that exact id (authtoken profilesAllowed), stamping hub-only `profileGranted` beside a survivor — and a GRANTED spawn keeps the LOCAL profile's configDir (remoteSpawnProfile in the brain) while the bypass-flag/mcpItemIds scrub still applies, because bus-written profiles have configDir scrubbed at write time so an honored configDir was always typed in locally"},
		// The two fields the SECURITY comment in both spawn handlers is actually
		// about. They were never in the vocabulary, so the clamp that is the
		// whole argument for this method's exemption was pinned by behavioural
		// tests only and by no classification at all — and the identical
		// escalation shipped unnoticed on claude.setPermissionMode.
		"skipPermissions": {KindPermission, "--dangerously-skip-permissions by another name; forced to false on both bus providers (assertNoPermissionBypass in hubCapabilities.ts, the same clamp in the brain's spawn) so a remote caller cannot start an agent that auto-approves everything. One verified exception: the hub router (internal/bus sanitizeSpawnParams) deletes any incoming `yoloGranted` and re-stamps it solely for a caller whose token record carries the full-access grant (authtoken yoloAllowed, minted by the local user via agents.fleetFullAccess) or the trusted host — and only a stamped spawn's request is honored by the providers"},
		"permissionMode":  {KindPermission, "the same escalation spelled as a mode: 'bypassPermissions' and 'yolo' are dropped to undefined on both bus providers, every other mode is passed through. A YOLO agent has to be started locally — or by a caller the hub stamped with the full-access grant (`yoloGranted`, same verified exception as skipPermissions)"},
		"effort":          {KindShell, "a reasoning-effort level handed to the daemon at spawn (codex model_reasoning_effort, claude's /effort); it selects among the provider's own levels and never becomes argv the caller composes"},
	},
	"terminals.create": {
		"cwd":   {KindPath, "a process working directory, as agents.spawn's — holding the capability is the gate"},
		"shell": {KindExecutable, "argv[0] of a host process, taken from a bus caller. There is no subtree to confine it to that the same caller cannot also fill in with fs.write, so it is an ALLOWLIST of the host's login shells instead: resolveTerminalShell in both providers (cmd/brain/shellallow.go, lib/shellAllowlist.ts)"},
	},
	"terminals.open": {
		"cwd":     {KindPath, "a process working directory for the visible terminal pane, as terminals.create's — holding the capability is the gate"},
		"command": {KindExecutable, "NOT argv[0]: it is a line RUN INSIDE the host's default login shell (the pane opens that shell; the caller names no shell), so it executes under that shell's own tool/PTY rules exactly as a line the user typed would — there is no argv[0] to allowlist and no subtree to confine, and desktop-only surfacing means the renderer is the thing that ever acts on it (emitToRenderer FACADE_OPEN_TERMINAL in hubCapabilities.ts)"},
	},
	// Hub-native. Neither params scan reaches cmd/hub — they read the two
	// PROVIDERS' handler sources — so these are written by hand, which is
	// precisely why the method-level guard over cmd/hub's registrations exists.
	"layout.set": {
		"data": {KindPermission, "the shared workspace document. Opaque to the hub except for four per-agent fields that become arguments to a LOCAL spawn when the desktop adopts it — skipPermissions, permissionMode, profileId, mcpItemIds — which layout.scrubAdoptedSpawnFields strips from a non-trusted write, matching agents.spawn's own clamps. The document's cwds and URLs are description: the hub never opens one"},
	},
	"files.upload": {
		// dataBase64 deliberately has NO row: the vocabulary has no data-at-rest
		// kind (fs.write's `contents` is likewise unlisted) — the bytes story is
		// the method-level unscopedByDecision reason and the compositionInert
		// claim in composition.go.
		"name": {KindFilename, "advisory only: the basename is discarded and the extension must be on the image/pdf allowlist; the written path is hub-composed (os.TempDir()/workspacer-uploads/m-<ts>-<rand>.<ext>)"},
	},
	"push.subscribe": {
		"endpoint": {KindURL, "a push-service URL the HOST posts to on the caller's behalf — a NETWORK SINK, not a string, and stored by one call for a different subsystem (push.Watch) to use later. Constrained by validatePushEndpoint to https at a non-private host; the payload is encrypted to the subscription's own keys, and RPCSubscribeAs records which credential asked so a revoked token stops being notified"},
	},
	"push.unsubscribe": {
		"endpoint": {KindID, "selects a stored subscription row to delete — narrowing, and never joined into a path"},
	},
	"push.revoke": {
		"id": {KindID, "selects a stored subscription row to delete. Operator-only by construction: push.revoke is in neither scoped tier"},
	},
	"sessions.transcript": {
		"cwd": {KindPath, "selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller"},
	},
	// providers.listModels' `cwd` moved to PathParam — it is CONFINED now, not
	// excused — so it must not also carry an unscoped decision. See PathParam.
	"claude.sessionsForDir": {
		"cwd": {KindPath, "encoded into a ~/.claude/projects slug by claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component; the caller's string is never opened as a path"},
	},
	"replay.open": {
		"cwd": {KindPath, "confined by the provider to the same workspace roots git.* uses (assertPathAllowed in hubCapabilities.ts), because it cuts a worktree from the repo at cwd"},
	},
	"replay.read": {
		"path": {KindPath, "a repo-relative coordinate inside a worktree the replay service itself created and keyed by sessionId; containment is structural (resolveInside)"},
	},
	"replay.diff": {
		"path": {KindPath, "same as replay.read — a coordinate inside a service-owned worktree, not a host path"},
	},
	"replay.seek": {
		"ops": {KindPath, "the wrapper the scanners can see: each op carries input.file_path and input.content, and the service re-anchors the path inside the worktree it owns — path.relative against the repo root, then containInWorktree, which resolves per component and writes the RESULT rather than the join. Written down here because the path is a level deeper than any scan reaches, so its confinement can only be pinned by naming the helper"},
	},
	"claude.setPermissionMode": {
		"mode": {KindPermission, "turns the host's approval prompts off (or back on) for an agent that is ALREADY running, so it is agents.spawn's clamped `permissionMode` arriving after the fact — through a method that does not ownership-check the session, i.e. one the local user started in ask mode. The bus handler therefore applies the SAME clamp as the spawn path (assertNoPermissionBypass): bypassPermissions/yolo are refused, everything that tightens or is neutral passes"},
	},
	"claude.setEffort": {
		"effort": {KindShell, "sent to a live claude session as the message `/effort <level>` (applyLiveEffort), so the value is prompt text for an already-running agent — the reach agents.sendMessage has, not the raw PTY write claude.answer has. Managed providers take the structural /model endpoint instead, where it selects among the provider's own levels"},
	},
	"claude.setModel": {
		"effort": {KindShell, "the same live-switch endpoint as claude.setEffort, reached with a model beside it; neither value is composed into argv by this process"},
	},
	"agents.reportProgress": {
		"note":            {KindShell, "prompt text for an already-running agent, like claude.setEffort's value and unlike claude.answer's — it is delivered with claudemonSessionClient.message (the queued /message endpoint every other [fleet] wake uses), never written to a PTY, and never composed into argv. The caller controls the SENTENCE and nothing around it: the host flattens it to one line, refuses it over 500 chars, and wraps it in a header and tail it composes itself (buildFleetMessage('progress')), which state that the sender is still running and that this is not a completion"},
		"callerSessionId": {KindID, "selects the CALLER, not a target: the host reads this session out of its own store and delivers to that row's parentSessionId, so the value can only ever pick a (session, its own parent) pair that already exists — it can name no recipient, and a session with no parent or a dead parent is refused rather than routed anywhere. On the path an agent actually uses it is not a caller value at all: the MCP facade overwrites it from the request token's `session:<id>` label, and the hub bus strips it from every untrusted caller (sanitizeReportProgressParams in internal/bus/rpc.go)"},
	},
	// claude.answer's three payload spellings all end at
	// claudemonSessionClient.input / r.cm.input — the same call
	// sessions.terminalInput makes with `data`. The decisions say so: a reason
	// that implied "this is only an answer" would be the excuse-wearing-a-
	// decision's-clothes shape this table exists to refuse.
	"claude.answer": {
		"text":    {KindShell, "typed into the session's PTY as `text + \"\\r\"` on both providers — byte-for-byte sessions.terminalInput's primitive, with no pending question required and no ownership check on the sessionId, so it reaches a terminals.create shell as readily as an agent. Holding the capability is the gate, exactly as for sessions.terminalInput; it buys nothing that granting THAT method does not, and grants nothing less"},
		"answers": {KindShell, "the multi-part spelling of `text`: each element is typed into the same PTY in turn, so excusing only `text` would leave the identical primitive unclassified under a second name (the mistake sessions.terminalInput's `bytesB64` records)"},
		"option":  {KindShell, "the numeric spelling — `option + \"\\r\"` into the same PTY. It is the narrowest of the three (a number), and it is listed because the decision has to cover every param that reaches the sink, not the one that looks worst"},
	},
	"agents.sendMessage": {
		"text": {KindShell, "a prompt delivered to an agent that is already running, through the daemon's POST /message rather than the PTY. What bounds it is the agent's own tool approvals — which is why claude.setPermissionMode's clamp is load-bearing for this method too"},
	},
	"git.status": {"cwd": gitCwd},
	"git.log":    {"cwd": gitCwd},
	"git.diff": {
		// git.diff's `cwd` is the SCOPED one (PathParam); only `path` needs a
		// decision here.
		"path": {KindPath, "an optional pathspec git interprets INSIDE the repo at cwd, and the provider confines it against the work-tree root git will actually resolve it in (workRoot via anchorGitPathspec), not the cwd the caller passed; the `untracked` leg is additionally held to the workspace roots"},
	},
	"git.numstat": {"cwd": gitCwd},
	"git.commitDiff": {
		"cwd":  gitCwd,
		"hash": {KindArgv, "lands in `git show` argv, where a leading '-' would be an option: gitService.assertCommitHash refuses anything that is not 4-40 hex digits before it gets there"},
		"path": {KindPath, "a pathspec after `--`, interpreted by git inside the already-confined repo"},
	},
	"git.commitNumstat": {
		"cwd":  gitCwd,
		"hash": {KindArgv, "same assertCommitHash gate as git.commitDiff — hex only, so it can never be an option-shaped argv element"},
	},
	// git.stage/git.unstage's `path` used to be excused as "a pathspec inside the
	// confined repo; git resolves it relative to the work-tree root the guard
	// returned" — a sentence that named the guard's cwd and then described a
	// DIFFERENT directory. `git add` runs from the derived work-tree root, and
	// nothing checked the pathspec at all, so `backend/prod-key.pem` (or no
	// pathspec, i.e. `git add -A` over the whole repository) indexed files
	// outside every allowed root, and a path-less `git.diff {staged}` handed
	// their full contents back. The staging leg now gets the same boundary the
	// untracked-diff leg got.
	"git.stage": {
		"cwd":  gitCwd,
		"path": {KindPath, "anchored on the work-tree root git will actually resolve it in and then held to the ordinary workspace roots (anchorGitPathspec in hubCapabilities.ts), because staging a file that is not in HEAD is what makes its full content readable through git.diff{staged}; with no path the call is bounded to the guarded cwd (cwdPathspec) instead of running `git add -A` from the root"},
	},
	"git.unstage": {
		"cwd":  gitCwd,
		"path": {KindPath, "same anchorGitPathspec treatment as git.stage, and the path-less form is likewise bounded to the guarded cwd by cwdPathspec — `git reset -q HEAD` from the derived root drops the index of a whole repository the caller was granted one directory of"},
	},
	"git.commit": {"cwd": gitCwd},
	"git.push":   {"cwd": gitCwd},
	"sessions.load": {
		"filename": {KindFilename, "a bare basename resolved and confined to <configDir>/sessions by both providers (sessionFilePath / resolveWithinSessionsDir); pinned by the corpus's sessionFilenames block"},
	},
	"sessions.save": {
		"filename": {KindFilename, "same as sessions.load: the provider derives it from the session name and re-checks it with the same resolver"},
		"name":     {KindFilename, "the value one step BEFORE the filename: both providers slug it (resolveSessionFilename in the brain, slugSession in sessionService.ts, pinned against each other by contracts/filename-slug-cases.json) and then run the result through the same sessions-dir containment as a caller-supplied filename"},
	},
	"sessions.delete": {
		"filename": {KindFilename, "same bare-basename rule as sessions.load, and it matters more here: the desktop copy of this resolver read and UNLINKED through a symlink until both were held to the corpus's sessionFilenames block (sessionFilePath / resolveWithinSessionsDir)"},
	},
	"sessions.terminalInput": {
		"data":     {KindShell, "raw bytes into an existing session's PTY: there is no path and no subtree to confine, so holding the capability is the gate, exactly as for terminals.create"},
		"bytesB64": {KindShell, "the base64 half of the same PTY byte stream — the brain accepts either encoding, so excusing only `data` would leave the identical primitive unclassified under a second name"},
	},
	"layouts.save": {
		"id":   {KindID, "a bare name slugged into <configDir>/layouts/<slug>.yaml and re-contained there by both providers (layoutFilePath / layoutService), never a caller-chosen directory"},
		"name": {KindFilename, "when `id` is absent the provider slugs `name` into the id, so it reaches the same layoutFilePath containment"},
	},
	"layouts.delete": {
		"id": {KindID, "same as layouts.save — re-slugged and re-contained to the layouts directory before anything is unlinked"},
	},
	// config.save takes no path and no param the scanners can see: its argument
	// is the whole config partial. Its decision keys are therefore config KEYS,
	// and a test holds this set equal to contracts/host-trusted-config-cases.json
	// — so a newly host-trusted key must be classified here, and a key
	// classified here that nothing strips is caught from the other side.
	"config.save": {
		"agents.binaries":        {KindExecutable, "the launcher path handed to claudemon's Command::new for every spawned agent, i.e. argv[0]; stripped from a bus write by dropHostTrusted as a dotted PATH so its sibling agents.* settings stay writable"},
		"claude.profiles":        {KindExecutable, "the profile list, each entry carrying configDir (CLAUDE_CONFIG_DIR) and extraArgs (--dangerously-skip-permissions); stripped from a bus write for the same reason claude.profiles.add scrubs"},
		"updates":                {KindURL, "updates.channel is concatenated into the electron-updater feed URL the desktop downloads and installs from, so one '../' relocates the updater to somebody else's repo; the whole section is stripped from a bus write"},
		"terminal.shell":         {KindShell, "argv[0] of the next terminal the LOCAL user opens: TerminalPane passes `shell || termCfg.shell` to IPC.TERMINAL_CREATE, which spawns argv:[resolvedShell]. The BUS door onto that primitive (terminals.create) has a shell allow-list; the local IPC door deliberately has none, so this is stripped from a bus write rather than allow-listed"},
		"terminal.shells":        {KindShell, "the same argv[0] as terminal.shell, reached through the NavBar \"+\" menu (shells[].path). Stripped as a dotted PATH so the sibling terminal.* settings a bus client legitimately edits stay writable"},
		"editor.terminalCommand": {KindShell, "not argv[0] but raw shell TEXT: ScrollContainer builds \"<cmd> <file>\" and TerminalPane types it into the user's own shell with a trailing CR, so ';' and '|' need no planted binary. Live when editor.engine is \"terminal\", which the same call can set"},
		"scripts":                {KindShell, "a map of agent cwd -> [{name,command}] the desktop renders as top-bar buttons and runs as a terminal's initialCommand, verbatim. The attacker picks the LABEL too and the cwd key comes free from agents.list; stripped as a whole SECTION because every key under it is a caller-chosen directory"},
	},
	"claude.profiles.add": {
		"configDir":  {KindPath, "persisted as CLAUDE_CONFIG_DIR — the directory supplying settings.json, permissions.allow and hooks. Not confinable (no subtree the same caller cannot fill in with fs.write), so it is SCRUBBED at write time on both bus providers (scrubBypassProfile)"},
		"extraArgs":  {KindArgv, "verbatim argv for a later claude spawn, i.e. where --dangerously-skip-permissions would live; scrubbed by the same scrubBypassProfile as configDir so a bus caller cannot plant one for the local user to pick"},
		"mcpItemIds": {KindID, "MCP server definitions pre-approved at spawn time; scrubbed to empty on a bus write, so nothing persisted here can carry an MCP command into a later LOCAL spawn"},
		"name":       {KindInert, "a display label. Profiles live in ONE claude-profiles.json keyed by a generated id (profilesPath / claudeProfiles.ts), so unlike sessions.save's `name` this one never becomes a filename"},
	},
	"claude.profiles.update": {
		"name":       {KindInert, "a display label on the profile row, exactly as in claude.profiles.add: profiles live in ONE claude-profiles.json keyed by a generated id (profilesPath), so this never becomes a filename"},
		"id":         {KindID, "selects an existing profile row in claude-profiles.json; it is not joined into any path (profiles are one file), and the update it selects is scrubbed exactly as an add is"},
		"updates":    {KindArgv, "the patch object carrying configDir/extraArgs/mcpItemIds — the scanners see only this wrapper, so it is classified as the vehicle: both providers run its fields through scrubBypassProfile before the write"},
		"configDir":  {KindPath, "same as claude.profiles.add, reached through `updates`"},
		"extraArgs":  {KindArgv, "same as claude.profiles.add, reached through `updates`"},
		"mcpItemIds": {KindID, "same as claude.profiles.add, reached through `updates`"},
	},
	"claude.profiles.remove": {
		"id": {KindID, "selects a row to delete from the single claude-profiles.json; never joined into a path, so there is no store directory to escape"},
	},
	// The approval-override pair. `decision` is the canonical KindPermission
	// value — this package defines that kind as "a value that changes what the
	// host will do WITHOUT asking", and "yes" on a pending PreToolUse is exactly
	// that.
	"claude.approve": {
		"decision": {KindPermission, "'yes'|'no'|'always' -> claudemon answers Claude Code's PreToolUse hook with {\"decision\":\"approve\"} on stdout, or sends allow=true down a managed adapter's can_use_tool channel; nothing downstream re-asks, so this value alone decides whether a queued tool call runs on the host"},
	},
	"claude.gate": {
		"on": {KindPermission, "arms or disarms the PreToolUse parking gate for a running session: with it ON every tool call stops and waits for claude.approve, and with it OFF the agent's own configured permissions apply. It changes what the host will do without asking, which is what KindPermission means"},
	},
	"notifications.post": {
		"url": {KindURL, "opened on click by the HOST, so a bus caller chooses a destination the desktop user's browser then visits; it goes through openExternalUrl, the same scheme allowlist the renderer's open-external path uses, rather than straight to the OS"},
	},
	// library.save's payload is an ITEM, and an item of kind `mcp` is a process
	// definition. The desktop provider hands the whole object to
	// libraryService.save without destructuring it, so the TS scan sees only
	// `cwd` — these four fields were visible to nobody until the brain's params
	// were scanned at all.
	"library.save": {
		"id":      {KindFilename, "slugged (slugLibrary) into <libraryDir>/<id>.md and re-confined by assertLibraryItemPath / guardLibraryFile before the write, so it names a file inside the item roots and cannot compose one outside them"},
		"command": {KindExecutable, "argv[0] of an MCP server, stored verbatim in the item's mcp block. It is inert on disk and becomes a process only when a SPAWN selects the item, which is why the bus spawn path refuses caller-supplied mcpItemIds (busMcpItemIds): the write side cannot be closed — the same bytes are reachable through an fs.write into a config store root — so the gate is on the spawner, not the writer"},
		"args":    {KindArgv, "the MCP server's argv[1:], stored beside `command` and gated the same way: unreachable without a spawn that selects the item, and a bus spawn refuses to"},
		"env":     {KindEnv, "the MCP server's environment, stored beside `command`. Same gate — an env is code execution by another route (PATH, LD_PRELOAD), so it is refused at spawn selection rather than trusted at write time"},
		"url":     {KindURL, "an SSE/HTTP MCP endpoint the agent would connect to instead of spawning a process; same selection gate as `command`, and the item file itself stays confined to the library item roots"},
	},
	"library.remove": {
		"id": {KindID, "names the item file to unlink under the library dir derived from the already-confined cwd; the unlink target is re-checked by guardLibraryFile('library.remove', libraryItemRoots(canonicalCwd))"},
	},
	"search.project": {
		"regex": {KindRegex, "a boolean MODE selector, not a value: false (the default) makes the provider pass ripgrep -F so `query` is a fixed string, true means `query` is a real pattern. It carries nothing of its own, and the pattern's own safety is argued in the `query` decision"},
		"query": {KindRegex, "passed after `--` in ripgrep's argv, so it can never become an option; with regex:true it is a real pattern, but rg's engine is linear-time (Rust regex, no backtracking) and the exec is timeout-bounded on both providers"},
	},
}

// gitCwd is the one decision the nine provider-confined git.* methods share.
// Written once so the nine entries cannot drift into nine subtly different
// claims about the same guard.
var gitCwd = ParamDecision{KindPath, "provider-confined to the workspace roots (guardGitCwd) before git runs in it; per-plugin scoping pending a catalog manifest update"}

// ClassifyParam answers the only question a drift detector should ask about a
// caller param: has somebody decided what this is? It returns ParamScoped when
// the bus itself confines the value, ParamExcused with the recorded kind and
// reason when it is deliberately unconfined, and ParamUnclassified — the zero
// value — otherwise.
//
// A method must be classified SOMEHOW first (a PathParam entry or an
// unscopedByDecision entry): a wholly unknown method picking up a familiar param
// name must not become classified by accident.
func ClassifyParam(method, param string) (ParamStatus, ParamDecision) {
	if field, scoped := PathParam[method]; scoped && field == param {
		return ParamScoped, ParamDecision{KindPath, "confined by the bus to the caller's granted fsRoots"}
	}
	_, excused := unscopedByDecision[method]
	_, scoped := PathParam[method]
	if !excused && !scoped {
		return ParamUnclassified, ParamDecision{}
	}
	if d, ok := unscopedParams[method][param]; ok {
		return ParamExcused, d
	}
	return ParamUnclassified, ParamDecision{}
}

// UnscopedParamCovered reports whether method's decision record covers this
// particular param. A method excused for one field is NOT excused for a second
// one it grew later.
func UnscopedParamCovered(method, param string) bool {
	status, _ := ClassifyParam(method, param)
	return status == ParamExcused
}

// ParamDecisions returns a copy of the per-param decisions recorded for method,
// for tests and diagnostics that want to show what IS classified when they
// report what isn't.
func ParamDecisions(method string) map[string]ParamDecision {
	out := map[string]ParamDecision{}
	for p, d := range unscopedParams[method] {
		out[p] = d
	}
	return out
}

// UnscopedMethods lists every method with a decision on the record, so a test
// can check that unscopedParams and unscopedByDecision describe the same set.
func UnscopedMethods() []string {
	out := make([]string, 0, len(unscopedByDecision))
	for m := range unscopedByDecision {
		out = append(out, m)
	}
	return out
}

// UnscopedReason returns the recorded reason for an excused method.
func UnscopedReason(method string) (string, bool) {
	r, ok := unscopedByDecision[method]
	return r, ok
}

// IsPathScoped reports whether method operates on a filesystem path and, if so,
// the params field that carries it. Methods absent from PathParam carry no path
// and need no filesystem confinement.
func IsPathScoped(method string) (field string, ok bool) {
	field, ok = PathParam[method]
	return field, ok
}

// pathVerbPrefixes are the capability-name namespaces whose methods, by
// convention, operate on a filesystem path: everything under fs.* reads/writes
// the host filesystem, search.* walks a directory tree, library.* reads and
// writes prompt/skill files under a project directory, and git.* runs commands
// in a checkout it is handed. A method under one of these prefixes MUST
// therefore be classified — a PathParam entry, or an explicit
// unscopedByDecision entry saying why not. If it is neither, IsPathScoped
// returns false and the bus's authorize() would wave it through with NO
// filesystem confinement. This is the drift the whole package exists to
// prevent, so we detect it by name rather than trusting whoever adds the next
// path-bearing method to also remember to scope it. The list starts at fs. and
// search. historically; every namespace added since had methods that reached
// the filesystem for years without anyone noticing, which is the argument for
// keeping it wide.
// providers.* joined the list when providers.listModels' `cwd` turned out not to
// be read but EXECUTED IN — claudemon runs the provider CLI with
// current_dir(cwd) and opencode loads `<cwd>/.opencode/plugin/*.js` from there.
// The namespace is on the list for the reason the sentence above gives: every
// prefix added since fs./search. had methods reaching the filesystem for years
// with nobody noticing.
// brief. joined for the same reason library. did: brief.append composes a file
// path under a caller-chosen directory. The namespace holds exactly one method
// today, and listing the NAMESPACE rather than the method is the point — a
// sibling added later (brief.read, brief.replace) is path-bearing by name and
// fails closed until somebody classifies it.
var pathVerbPrefixes = []string{"fs.", "search.", "library.", "git.", "providers.", "brief."}

// LooksPathBearing reports whether method's name sits under a known filesystem
// namespace (fs.*, search.*, library.*, git.*) and is therefore expected to
// carry a path that needs confinement. It is a naming convention, not proof:
// pair it with PathParam via [MissingSpec] to find methods that look
// path-bearing but were never classified.
func LooksPathBearing(method string) bool {
	for _, p := range pathVerbPrefixes {
		if strings.HasPrefix(method, p) {
			return true
		}
	}
	return false
}

// MissingSpec reports the exact fail-open condition this package guards against:
// a method whose name marks it path-bearing (fs.*, search.*, library.*, git.*)
// that nobody ever classified — neither a PathParam entry nor an
// unscopedByDecision entry — so it would be granted and called with no
// filesystem containment. Callers should fail closed on true: the bus refuses to
// grant such a method and authorize() denies it, and a test cross-checks every
// registered capability so the omission is caught at build time rather than as a
// silent privilege escape.
func MissingSpec(method string) bool {
	if _, ok := PathParam[method]; ok {
		return false
	}
	if _, ok := unscopedByDecision[method]; ok {
		return false
	}
	// inertMethods counts as a classification here too, and has to: adding
	// "providers." to pathVerbPrefixes made providers.checkAll — a method with no
	// params at all, already on the inert record with its reason — look
	// path-bearing by NAME. "Nobody classified it" is the condition this
	// function names; a written inert reason is somebody classifying it.
	//
	// The door that opens (file a real path method as inert and it is grantable
	// unconfined) is the same door unscopedByDecision already is, and it is
	// closed from the other side by TestInertPathBearingMethodsTakeNoParams:
	// an inert method under a path-verb prefix may carry no caller params at all.
	if _, ok := inertMethods[method]; ok {
		return false
	}
	return LooksPathBearing(method)
}

// Grant is one capability a plugin token may call, with optional filesystem
// scoping. FSRoots, when set, restricts a path-scoped call to targets within one
// of the (canonical, absolute) roots; it is empty for non-path methods. Defined
// here — not in the bus — so the plugin loader can build grants without importing
// the bus, and the bus can accept them without importing the loader.
type Grant struct {
	Method  string
	FSRoots []string
}

// EventGrants is a plugin token's pub/sub + provider surface — the event side of
// the same "declare it in the manifest to be allowed it" model that [Grant]
// gives capability calls:
//
//   - Emits: event types the plugin may publish on the bus.
//   - Consumes: event types it may receive (delivery of anything else is dropped).
//   - Provides: capability method names it may register as a provider of.
//
// Patterns use the bus topic syntax — exact, "prefix.*", or "*" — matched by
// internal/event.Matches. Empty means none: a plugin that declared nothing can
// neither publish, receive, nor provide, matching the fail-closed stance of
// capability calls. Trusted connections (the host) bypass all of this.
type EventGrants struct {
	Emits    []string
	Consumes []string
	Provides []string
}

// The event plane's topic registry lives in eventtopics.go — EventTopicSpec,
// EventTopicCapability, EventTopicHostOnly, EventTopicIsHostOwned. It is next to
// this file rather than in it because it answers a different verb (SUBSCRIBE and
// PUBLISH, not CALL) and because it needs three dispositions, not one table.
