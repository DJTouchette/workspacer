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
	"terminals.create":     "cwd is a process working directory and holding the capability at all is the gate (as agents.spawn); the OTHER caller string, `shell`, is argv[0] and is confined by an ALLOWLIST of the host's login shells rather than by fsRoots — resolveTerminalShell in both providers",
	"sessions.transcript":  "cwd only selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller",
	"providers.listModels": "cwd picks which project's provider config to read; the provider resolves the file itself",
	// The sentence used to stop at "never opened as a path", and it was false:
	// the encoder maps only '/', '\' and ':' to '-', so a cwd of ".." survived
	// verbatim, became a real path COMPONENT, and joined to ~/.claude — one
	// level out of the sandbox this exemption assumes. Both providers now run
	// the encoded name through claudeProjectDirName, which refuses "", "." and
	// "..", so the slug really is a single plain component and the reason below
	// is true rather than aspirational.
	"claude.sessionsForDir": "cwd is encoded into a ~/.claude/projects slug by the provider (claudeProjectDirName, which refuses '', '.' and '..' so the slug is always ONE plain component); the caller's string is never opened as a path",
	"replay.open":           "confined by the provider to the same workspace roots git.* uses (assertPathAllowed in hubCapabilities.ts), because it cuts a worktree from the repo at cwd",
	"replay.read":           "the path is a repo-relative coordinate inside a worktree the replay service itself created and keyed by sessionId; containment is structural (resolveInside), and fsRoots would be scoping the wrong namespace",
	"replay.diff":           "same as replay.read — a coordinate inside a service-owned worktree, not a host path",
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
	"config.save": "takes no path; the config file is the provider's own and the dangerous KEYS (agents.binaries, claude.profiles) are stripped from a bus write by dropHostTrusted — see contracts/host-trusted-config-cases.json",
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
	"sessions.terminalInput": "writes raw bytes into an existing session's PTY: there is no path and no subtree to confine, so holding the capability is the gate, exactly as for terminals.create. NOTE that this makes terminals.create's shell allowlist a boundary only against callers that do not ALSO hold this method — allowlisted /bin/bash plus typed bytes is full argv[0] freedom — and that the sessionId is not ownership-checked on either provider, so it reaches the local user's own agent PTY too",
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
	// KindInert — the name is in the vocabulary, but on THIS method the value
	// provably never becomes any of the above. Recording it is a decision, not
	// an omission: the Why has to say what it becomes instead.
	KindInert ParamKind = "inert"
)

var knownKinds = map[ParamKind]bool{
	KindPath: true, KindFilename: true, KindExecutable: true, KindArgv: true,
	KindShell: true, KindEnv: true, KindURL: true, KindPort: true,
	KindID: true, KindRegex: true, KindInert: true,
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
	"data": KindShell, "bytesB64": KindShell, "stdin": KindShell,
	"script": KindShell, "keys": KindShell,
	// Environment of a spawned process.
	"env": KindEnv, "envVars": KindEnv, "environment": KindEnv,
	// Network destinations the host opens or fetches.
	"url": KindURL, "uri": KindURL, "endpoint": KindURL, "href": KindURL,
	"webhook": KindURL, "port": KindPort,
	// Ids that RESOLVE into one of the above.
	"id": KindID, "itemId": KindID, "mcpItemIds": KindID, "profileId": KindID,
	// A patch WRAPPER: claude.profiles.update carries its configDir/extraArgs/
	// mcpItemIds inside `updates`, and a scanner that only saw the wrapper name
	// (the desktop's does — the object is passed whole) saw a param the
	// vocabulary did not know, so the decision recorded for it was consulted by
	// nobody. See TestEveryParamDecisionNamesAParamAScannerCanSee.
	"updates": KindArgv,
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
		"profileId":  {KindID, "resolves to a stored profile whose configDir becomes CLAUDE_CONFIG_DIR and whose extraArgs become argv, so it is the other way to smuggle a bypass; both bus providers scrub the RESOLVED profile before spawning (scrubProfileBypass in hubCapabilities.ts, scrubBypassProfile in the brain — see spawn_bypass_test.go)"},
	},
	"terminals.create": {
		"cwd":   {KindPath, "a process working directory, as agents.spawn's — holding the capability is the gate"},
		"shell": {KindExecutable, "argv[0] of a host process, taken from a bus caller. There is no subtree to confine it to that the same caller cannot also fill in with fs.write, so it is an ALLOWLIST of the host's login shells instead: resolveTerminalShell in both providers (cmd/brain/shellallow.go, lib/shellAllowlist.ts)"},
	},
	"sessions.transcript": {
		"cwd": {KindPath, "selects which historical session to resolve under ~/.claude/projects; the transcript path is derived by the provider, never taken from the caller"},
	},
	"providers.listModels": {
		"cwd": {KindPath, "picks which project's provider config to read; the provider resolves the file itself"},
	},
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
	"git.status": {"cwd": gitCwd},
	"git.log":    {"cwd": gitCwd},
	"git.diff": {
		// git.diff's `cwd` is the SCOPED one (PathParam); only `path` needs a
		// decision here.
		"path": {KindPath, "an optional pathspec git interprets INSIDE the repo at cwd, and the provider confines it against the work-tree root git will actually resolve it in (workRoot), not the cwd the caller passed"},
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
	"git.stage": {
		"cwd":  gitCwd,
		"path": {KindPath, "a pathspec inside the confined repo; git resolves it relative to the work-tree root the guard returned"},
	},
	"git.unstage": {
		"cwd":  gitCwd,
		"path": {KindPath, "a pathspec inside the confined repo; git resolves it relative to the work-tree root the guard returned"},
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
		"agents.binaries": {KindExecutable, "the launcher path handed to claudemon's Command::new for every spawned agent, i.e. argv[0]; stripped from a bus write by dropHostTrusted as a dotted PATH so its sibling agents.* settings stay writable"},
		"claude.profiles": {KindExecutable, "the profile list, each entry carrying configDir (CLAUDE_CONFIG_DIR) and extraArgs (--dangerously-skip-permissions); stripped from a bus write for the same reason claude.profiles.add scrubs"},
		"updates":         {KindURL, "updates.channel is concatenated into the electron-updater feed URL the desktop downloads and installs from, so one '../' relocates the updater to somebody else's repo; the whole section is stripped from a bus write"},
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
var pathVerbPrefixes = []string{"fs.", "search.", "library.", "git."}

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
