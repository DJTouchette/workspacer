// Package capspec is the small, dependency-free vocabulary shared between the
// bus (which enforces capability grants) and the plugin loader (which validates
// manifests and translates them into grants). Keeping it here avoids a bus↔plugin
// import cycle and keeps the list of filesystem-scoped capabilities in exactly
// one place, so enforcement and validation can never drift apart.
package capspec

import "strings"

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
	"agents.spawn":         "starting an agent is a separate authorization decision — the cwd picks where a process runs, and confining it would need the spawn paths to learn root containment first (see cmd/brain's TestSpawnStaysDeliberatelyUnscoped)",
	"terminals.create":     "same as agents.spawn: cwd is a process working directory, and holding the capability at all is the gate",
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
