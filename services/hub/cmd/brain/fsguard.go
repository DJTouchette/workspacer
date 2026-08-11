// Filesystem path confinement for the brain's fs.* / search.project handlers.
//
// This is a port of the confinement in the desktop's hubCapabilities.ts, and it
// has to exist HERE as well because of who actually answers the bus: the desktop
// registers those methods through `cat(...)`, which is a no-op whenever the
// catalog is delegated to this brain — the default. So the guard in the app was
// unreachable in normal operation and these handlers were serving arbitrary host
// paths to any bus client: a remote-share client on the tailnet, a plugin, or an
// agent through the MCP facade. Same rule, same failure mode, both providers.
//
// The rule: a caller-supplied path must canonicalize to a location at or inside
// a live agent cwd or one of the config-dir stores (library/, layouts/,
// sessions/), must not be a credential file by name, and must not land anywhere
// else in the config dir even when an agent cwd contains it. Canonicalize means
// absolute with `..` AND symlinks resolved, so neither traversal nor a symlink
// planted inside a project can reach out of it.
//
// This file implements the normative containment algorithm shared with
// apps/desktop/src/main/lib/pathConfinement.ts and services/hub/internal/bus/
// policy.go; contracts/path-containment-cases.json is the fixture all three are
// held to. Three decisions are load-bearing and each one has shipped as a bug:
//
//  1. NO TILDE EXPANSION, at any layer that handles a caller-supplied path. The
//     brain used to expandTilde() every guarded path while TypeScript did not, so
//     "~/notes.txt" was allowed by one provider and denied by the other. "~" is
//     now an ordinary character: a '~'-prefixed path is not absolute and is
//     refused, and a directory literally named "~" is an ordinary directory.
//  2. RESOLVE PER COMPONENT, and hand the RESULT to the filesystem. Every
//     whole-path helper (filepath.Abs, Clean, Join on caller input,
//     EvalSymlinks, Dir on caller input) collapses ".." textually BEFORE any
//     symlink is read, so with one directory symlink inside any allowed root the
//     guard validated ${ROOT}/token while the handler opened ${OUTSIDE}/token.
//     canonicalizePath walks component by component and assertPathAllowed
//     returns what it validated so there is exactly one string per request.
//  3. A ROOT THAT IS A VOLUME ROOT CONTAINS EVERYTHING BELOW IT. within("/",
//     "/etc/passwd") used to be true on the bus and false here — same inputs,
//     opposite verdicts. Refusing was not fail-closed, it was wrong containment.
//     The secret gate below is what still refuses the credentials under such a
//     root, and it is unaffected by that widening.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"
)

// maxLinkHops bounds the hand-rolled symlink walk below. The platform realpath
// has its own ELOOP limit; this walk does not delegate to it, so the counter is
// the only thing standing between a symlink cycle and a spin.
const maxLinkHops = 40

// onWindows is a compile-time constant so the volume-prefix rules below are
// decided once rather than per call. `filepath.Separator` is the CANONICAL
// separator (used for joining and for the containment comparison);
// `os.IsPathSeparator` is the separator SET (used only for splitting, and on
// Windows it accepts '/' as well as '\\').
const onWindows = filepath.Separator == '\\'

// errNotAbsolute and errEmptyPath are the two pre-filesystem refusals. They are
// never surfaced to a caller — assertPathAllowed collapses every reason into one
// message — but keeping them distinct makes the walk debuggable.
var (
	errEmptyPath   = errors.New("path is empty")
	errNotAbsolute = errors.New("path is not absolute")
	errTooManyHops = errors.New("too many levels of symbolic links")
)

// splitPath decomposes an ABSOLUTE path into its volume prefix and its
// components. On POSIX the volume is "/"; on Windows it is `C:\` or
// `\\server\share\`. A path that does not start with a valid volume prefix is
// not absolute and is rejected here — including a '~'-prefixed one, which is an
// ordinary (relative) string with an unusual first character and nothing else.
//
// "." and empty components are discarded; ".." is NOT, because step 2 of the
// algorithm has to pop it against an already-resolved prefix rather than
// textually against the caller's string.
func splitPath(p string) (volume string, comps []string, ok bool) {
	v := filepath.VolumeName(p) // "" on POSIX; "C:" or `\\server\share` on Windows
	rest := p[len(v):]
	if v == "" {
		// A Windows path with no drive/UNC prefix ("\foo") is drive-RELATIVE:
		// it resolves against whatever drive the process happens to be on.
		if onWindows {
			return "", nil, false
		}
		if len(rest) == 0 || !os.IsPathSeparator(rest[0]) {
			return "", nil, false
		}
	} else if len(rest) == 0 || !os.IsPathSeparator(rest[0]) {
		// "C:foo" is relative to the current directory ON drive C.
		return "", nil, false
	}
	return v + string(filepath.Separator), splitComponents(rest[1:]), true
}

// splitComponents splits on any run of separator-set characters and drops "."
// and empty elements. It performs no other transformation: case is preserved
// byte for byte and "~" is an ordinary name.
func splitComponents(s string) []string {
	out := []string{}
	start := -1
	for i := 0; i < len(s); i++ {
		if os.IsPathSeparator(s[i]) {
			if start >= 0 {
				if c := s[start:i]; c != "." {
					out = append(out, c)
				}
				start = -1
			}
			continue
		}
		if start < 0 {
			start = i
		}
	}
	if start >= 0 {
		if c := s[start:]; c != "." {
			out = append(out, c)
		}
	}
	return out
}

// appendPath joins a name onto a prefix the algorithm itself built. It is not
// filepath.Join: it must not Clean, because Clean is what collapses `link/..`
// before the link is read.
func appendPath(base, name string) string {
	if len(base) > 0 && os.IsPathSeparator(base[len(base)-1]) {
		return base + name
	}
	return base + string(filepath.Separator) + name
}

// parentPath pops one component off a prefix the algorithm built, clamping at
// the volume root. Pure string arithmetic, never a filesystem access: `base` is
// by construction already fully symlink-resolved, so its textual parent IS its
// real parent — which is exactly why the per-component walk closes the
// symlink-plus-".." hole that a whole-path Clean opens.
func parentPath(base, volume string) string {
	if base == volume {
		return volume
	}
	i := strings.LastIndexByte(base, filepath.Separator)
	if i < 0 {
		return volume
	}
	p := base[:i]
	if p == "" || len(p) < len(volume) {
		return volume
	}
	return p
}

// canonicalizePath resolves target one component at a time: `..` pops the
// already-resolved prefix, a symlink is read and its value pushed back onto the
// queue, and a component that does not exist is appended and the walk CONTINUES
// (a later `..` can pop back onto a path that does exist, and must resume
// resolving there). The result is absolute, has no "." or ".." left, and every
// symlink on it has been followed.
//
// What it deliberately does NOT use: filepath.Abs, filepath.Clean,
// filepath.Join on caller input, filepath.EvalSymlinks, filepath.Dir on caller
// input. Every one of them collapses ".." textually before a symlink is read,
// so the guard would validate one path while the handler opened another.
//
// Fail-closed everywhere: an empty or non-absolute target is refused before any
// syscall, and the ONLY tolerated Lstat error is "does not exist". ENOTDIR,
// EACCES, ELOOP and anything unrecognised fail the whole call rather than
// becoming "not contained, keep looking" (which would make the guard an
// existence oracle) or "contained" (which would make it an escape).
// errUnnameable marks a component the Win32 path layer cannot name at all
// (one made only of dots and spaces, e.g. "..." or "   ", which it trims to
// nothing). A path we cannot name is never "probably fine".
var errUnnameable = errors.New("path component names nothing")

// winCanonComponent applies what the Win32 path layer does to EVERY component
// before it reaches the filesystem: trailing spaces and dots are stripped. It is
// the identity on POSIX, where those characters are ordinary.
//
// This is the check-path/opened-path invariant, and Windows breaks it silently.
// "x " opens x. ".. " opens .. — a PARENT TRAVERSAL, where POSIX gives a literal
// child name. Measured on the Windows runner before this existed: fs.listDir on
// "<root>/workspacer/layouts/.. " was ALLOWED as a child of the root and listed
// the CONFIG DIR, which is where remote-token lives — the credential that
// promotes a bus connection to trusted. The guard was not wrong by its own
// rules; its canonical string simply stopped naming the file the handler opens.
//
// "." and ".." have to be recognised BEFORE the trim, because trimming dots
// would erase them too.
func winCanonComponent(c string) (string, bool) {
	if runtime.GOOS != "windows" {
		return c, true
	}
	if t := strings.TrimRight(c, " ."); t != "" {
		return t, true
	}
	// Dots and spaces only. "." and ".." keep their meaning (a trailing space
	// does not change which directory ".." means); anything else — "...",
	// "   ", ".. ." — trims to nothing and names no file Win32 can open.
	switch s := strings.TrimRight(c, " "); s {
	case ".", "..":
		return s, true
	}
	return "", false
}

// parentIsWalkable reports whether a MISSING component may be walked through,
// by asking the question POSIX answers with its errno and Windows does not.
//
// The walk continues on "does not exist" because a target that has yet to be
// created still has to canonicalize. POSIX distinguishes the two ways a
// component can be absent: a genuinely missing name is ENOENT, while a name
// UNDER a regular file is ENOTDIR, which fails closed. Windows collapses both
// into ERROR_PATH_NOT_FOUND, which Go maps to not-exist and Node reports as
// ENOENT — so on Windows all three copies of this predicate walked straight
// through a regular file and returned allow where POSIX returns deny. Measured:
// the `a path through a regular file` contract case is the one that failed on
// the Windows runner the first time this module ever compiled there.
//
// Asking directly makes both platforms answer the same question instead of
// inheriting their libc's opinion. A parent that does not exist is the ordinary
// missing-tail case and keeps walking; only an existing NON-directory stops it.
// On POSIX this is unreachable — Lstat already returned ENOTDIR — which is why
// the case that proves it can only go red on Windows.
func parentIsWalkable(parent string) bool {
	st, err := os.Lstat(parent)
	if err != nil {
		return true // missing tail: deeper components are missing too, not misrooted
	}
	return st.IsDir()
}

func canonicalizePath(target string) (string, error) {
	if strings.TrimSpace(target) == "" {
		return "", errEmptyPath
	}
	// Everything past the emptiness test runs on the ORIGINAL string: a
	// filename may legitimately begin or end with a space.
	volume, queue, ok := splitPath(target)
	if !ok {
		return "", errNotAbsolute
	}

	resolved := volume
	hops := 0
	for len(queue) > 0 {
		c := queue[0]
		queue = queue[1:]

		// Win32 strips trailing spaces/dots before the filesystem ever sees the
		// name, so the walk has to do it too or its answer describes a different
		// file than the handler opens. Identity on POSIX.
		c, ok := winCanonComponent(c)
		if !ok {
			return "", errUnnameable
		}
		if c == "." {
			continue // ". " normalizes to "." on Windows; splitComponents only drops the bare form
		}

		if c == ".." {
			resolved = parentPath(resolved, volume)
			continue
		}

		next := appendPath(resolved, c)
		st, err := os.Lstat(next)
		if err != nil {
			if os.IsNotExist(err) && parentIsWalkable(resolved) {
				resolved = next
				continue
			}
			return "", err
		}
		if st.Mode()&os.ModeSymlink == 0 {
			resolved = next
			continue
		}

		hops++
		if hops > maxLinkHops {
			return "", errTooManyHops
		}
		link, err := os.Readlink(next)
		if err != nil {
			return "", err
		}
		if lv, lc, abs := splitPath(link); abs {
			// An absolute link restarts from its own volume.
			resolved = lv
			queue = append(lc, queue...)
		} else {
			// A relative link is interpreted against the directory that
			// CONTAINS it — `resolved`, which is why it is not advanced here.
			queue = append(splitComponents(link), queue...)
		}
	}
	return resolved, nil
}

// canonicalRoot is the comparable form of an allowed root, or DISCARD (ok
// false). A root that does not exist yet still canonicalizes — the config-dir
// stores are created lazily and have to be comparable before anything has been
// saved.
//
// DISCARD removes that one root and nothing else: it never aborts the check and
// never denies on its own, or a single stale session snapshot with an empty cwd
// would disable every other root. An empty, whitespace-only or relative root is
// discarded rather than resolved, because filepath.Abs("") returns the PROCESS
// working directory — which would silently make the daemon's own cwd an allowed
// root. A '~'-prefixed root is relative by that rule and is discarded too;
// nothing expands it (BINDING DECISION 1).
func canonicalRoot(root string) (string, bool) {
	cr, err := canonicalizePath(root)
	if err != nil {
		return "", false
	}
	return cr, true
}

// asciiLower folds A-Z only. Deliberately not strings.ToLower: the three copies
// have to fold IDENTICALLY, and Go's Unicode folding and JavaScript's
// toLowerCase disagree (U+0130 'İ' is the one that already bit the filename
// slugs). Every name this is used on — the two credential basenames and the
// config dir's own path components — is ASCII in practice, so restricting the
// fold to ASCII costs nothing and removes a whole class of drift.
func asciiLower(s string) string {
	var b []byte
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c >= 'A' && c <= 'Z' {
			if b == nil {
				b = []byte(s)
			}
			b[i] = c + ('a' - 'A')
		}
	}
	if b == nil {
		return s
	}
	return string(b)
}

// containsPath is the containment comparison itself, over two ALREADY canonical
// paths. Byte-exact, no case folding on any platform.
//
// The trailing-separator arm is reached only when the root canonicalized to a
// volume prefix ("/" on POSIX, `C:\` or `\\server\share\` on Windows). Such a
// root contains everything below it; appending another separator would produce
// "//" and match nothing, which is not fail-closed, it is simply wrong
// containment. Otherwise the separator is mandatory: without it root "/srv/fo"
// contains "/srv/foo".
func containsPath(canonRoot, canonTarget string) bool {
	// An empty root grants NOTHING. Without this it is a WILDCARD: neither branch
	// below sees a trailing separator, so it falls through to
	// HasPrefix(canonTarget, "/") — true for every absolute path. canonicalRoot
	// discards a root it cannot resolve, but the last line of defence must not
	// itself be the widest possible grant.
	if canonRoot == "" {
		return false
	}
	if canonTarget == canonRoot {
		return true
	}
	if strings.HasSuffix(canonRoot, string(filepath.Separator)) {
		return strings.HasPrefix(canonTarget, canonRoot)
	}
	return strings.HasPrefix(canonTarget, canonRoot+string(filepath.Separator))
}

// containsPathFolded is containsPath with ASCII case folded away. It is used
// ONLY by the secret gate, and only in the directions where folding DENIES more:
// deciding that a target is inside the config dir, never deciding it is inside a
// store carve-out.
//
// The roots comparison stays byte-exact (containsPath above) because there
// folding would WIDEN an allow-list. The secret gate is the mirror image: macOS
// (APFS, case-insensitive by default) and Windows (NTFS) both open
// <configDir>/remote-token when handed <configHome>/WORKSPACER/remote-token, and
// the per-component walk cannot see it — os.Lstat succeeds on the caller's
// spelling and the walk appends the caller's spelling, so the canonical form
// carries the attacker's case and a byte-exact gate answers for the STRING
// rather than for the file. Folding here is fail-closed on every platform: on a
// case-sensitive filesystem it can only refuse a path that names a different
// (and in practice non-existent) file.
func containsPathFolded(canonRoot, canonTarget string) bool {
	return containsPath(asciiLower(canonRoot), asciiLower(canonTarget))
}

// isWithin reports whether an ALREADY canonicalized target sits at or inside
// root. Callers that hold the canonical form use this directly so a path is
// resolved once per check rather than once per root.
func isWithin(canonicalTarget, root string) bool {
	cr, ok := canonicalRoot(root)
	if !ok {
		return false
	}
	return containsPath(cr, canonicalTarget)
}

// pathWithinRootsCanonical is the roots test over a path that has already been
// canonicalized exactly once by the caller.
//
// An empty roots list means NOTHING is allowed. It never means unrestricted —
// that is the state a failed session-store read leaves behind.
func pathWithinRootsCanonical(roots []string, canonicalTarget string) bool {
	for _, root := range roots {
		if isWithin(canonicalTarget, root) {
			return true
		}
	}
	return false
}

// pathWithinRoots reports whether target canonicalizes to a location at or
// inside one of roots. Anything unverifiable is denied.
func pathWithinRoots(roots []string, target string) bool {
	ct, err := canonicalizePath(target)
	if err != nil {
		return false
	}
	return pathWithinRootsCanonical(roots, ct)
}

// Live agent cwds change as agents spawn and stop, and in catalog scope reading
// them means an HTTP round trip to claudemon. A short TTL keeps an interactive
// file tree from paying that per entry while still picking up a new agent's cwd
// almost immediately. Deny decisions are never cached — only the root list is.
// A var, not a const, so TestStoppedAgentCwdLeavesTheAllowList can shrink it and
// assert the EXPIRY rather than only the shipped number. Both halves are
// necessary: without the behavioural test the direction of the expiry is free,
// and without the bound on the shipped value the TTL can be raised to an hour
// with the behavioural test still green.
var cwdCacheTTL = 2 * time.Second

// cwdCacheTTLCeiling is the contract on the shipped value: this cache is the only
// thing that ever REVOKES a root, so however it is tuned, a stopped agent's
// directory must leave the allow-list promptly.
const cwdCacheTTLCeiling = 5 * time.Second

var (
	cwdCacheMu   sync.Mutex
	cwdCacheAt   time.Time
	cwdCacheVals []string
)

// agentCwds returns the cwd of every LIVE session — the local store's in full
// scope, claudemon's /sessions in catalog scope (the default).
//
// The liveness filter is the load-bearing part, and it was missing. claudemon's
// /sessions is RESUMABLE HISTORY, not a fleet: api.rs list_sessions hides only
// `archived` rows (stopped AND idle past a seven-day window) and `empty_stopped`
// ones. Everything else it lists — an agent that ended this morning, a shell
// somebody opened with terminals.create — came back as an fs.read + fs.write
// root. So a closed project's whole tree stayed writable for up to a week after
// the agent died, and terminals.create (whose `cwd` capspec deliberately leaves
// unconfined, because holding the method IS the grant) doubled as a way to
// nominate any directory on the host as a workspace root.
//
// snapshotLive is the same vocabulary agents.list already filters on, minus the
// curation half — see its comment for why a stopped-but-curated row is visible
// yet must not be a root.
func (r *registry) agentCwds(ctx context.Context) []string {
	cwdCacheMu.Lock()
	if !cwdCacheAt.IsZero() && time.Since(cwdCacheAt) < cwdCacheTTL {
		cached := cwdCacheVals
		cwdCacheMu.Unlock()
		return cached
	}
	cwdCacheMu.Unlock()

	var raws []json.RawMessage
	if r.store != nil {
		raws = r.store.all()
	} else if r.cm != nil {
		if listed, err := r.cm.listSessions(ctx); err == nil {
			// /sessions is an array of session objects; a shape change here must
			// not silently widen the allow-list, so a decode failure yields none.
			_ = json.Unmarshal(listed, &raws)
		}
	}

	cwds := make([]string, 0, len(raws))
	for _, raw := range raws {
		var s struct {
			Cwd string `json:"cwd"`
		}
		if err := json.Unmarshal(raw, &s); err != nil || s.Cwd == "" {
			continue
		}
		if !snapshotLive(raw) {
			continue
		}
		cwds = append(cwds, s.Cwd)
	}

	cwdCacheMu.Lock()
	cwdCacheAt = time.Now()
	cwdCacheVals = cwds
	cwdCacheMu.Unlock()
	return cwds
}

// configStoreRoots are the config-dir subtrees a client legitimately edits
// through fs.*: library items, layout templates and saved sessions.
//
// The config DIR itself used to be the root, which was far too wide: it also
// holds remote-token and tokens.json (bus credentials), remote-server.json,
// vapid.json, the Electron cookie/localStorage jars, and every installed
// plugin's .bus-token and plaintext .settings.json. A plugin whose manifest
// declared an absolute fs.read path landing in there could read remote-token and
// reconnect as a TRUSTED bus connection — which unlocks /plugins/install, i.e.
// arbitrary commands. Everything else in the config dir has a dedicated
// capability (config.*, claude.profiles.*) and never needed raw file access.
func configStoreRoots() []string {
	base := configDir()
	return []string{
		filepath.Join(base, "library"),
		filepath.Join(base, "layouts"),
		filepath.Join(base, "sessions"),
	}
}

// workspaceRoots is the allow-list for content-touching calls: live agent cwds
// plus the config-dir stores above.
func (r *registry) workspaceRoots(ctx context.Context) []string {
	cwds := r.agentCwds(ctx)
	stores := configStoreRoots()
	roots := make([]string, 0, len(cwds)+len(stores))
	roots = append(roots, cwds...)
	return append(roots, stores...)
}

// browseRoots is the wider allow-list for the directory picker (fs.listDir):
// the home tree, so a user can navigate to a project before an agent runs in it,
// but still not `/etc` or another user's home.
func (r *registry) browseRoots(ctx context.Context) []string {
	home, err := os.UserHomeDir()
	if err != nil {
		return r.workspaceRoots(ctx)
	}
	return append(r.workspaceRoots(ctx), home)
}

// secretBasenames are credential files by name, denied wherever they resolve.
// The roots above already keep the config dir's plugin tree out of reach, but a
// root is only as narrow as the cwds an agent runs in: spawn an agent in
// ~/.config/workspacer (or run `workspacer plugin dev` on a directory inside a
// project) and these come back into an allowed root. A bus token is a bus token
// wherever it sits, so deny it by name too.
var secretBasenames = map[string]bool{
	".bus-token":     true, // per-plugin bus credential
	".settings.json": true, // per-plugin settings, secrets in plaintext
}

// pathIsSecret is the second gate, applied to every guarded path after the roots
// check — reads AND writes, because handing a token out is a privilege promotion
// and overwriting one is a denial of service on the whole bus.
//
// Narrowing the config root is not enough on its own: an agent cwd is a root
// too, so spawning an agent in $HOME (or in ~/.config) re-admits the entire
// config dir through THAT root. So anything landing in the config dir outside
// library/ layouts/ sessions/ is refused here regardless of which root allowed
// it. Enumerating the credentials by name was the earlier shape and it was too
// narrow — it left config.yaml writable (updates.channel is concatenated into
// the electron-updater feed URL, so that is remote code execution laundered
// through the update dialog), and left workspacer.db and the legacy plaintext
// plugin-settings.json readable. This deny-the-whole-dir rule is the one
// hubCapabilities.ts isSecretPath implements; the two must stay word for word.
func pathIsSecret(target string) bool {
	ct, err := canonicalizePath(target)
	if err != nil {
		return true // unverifiable → deny, same posture as pathWithinRoots
	}
	return pathIsSecretCanonical(ct)
}

// pathIsSecretCanonical is the same gate over a path already canonicalized once
// by the caller.
//
// Order is load-bearing: the BASENAME check runs first and unconditionally, so a
// credential name dropped inside a store carve-out (<configDir>/library/.bus-token,
// the one directory a remote caller can write to) is still refused. Returning
// "allowed" as soon as a carve-out matches would re-open that hole.
func pathIsSecretCanonical(canonicalTarget string) bool {
	// Folded: ".BUS-TOKEN" opens .bus-token on macOS and Windows, and the walk
	// hands this gate the caller's spelling (see containsPathFolded).
	if secretBasenames[asciiLower(canonicalBase(canonicalTarget))] {
		return true
	}
	if traversesGitDir(canonicalTarget) {
		return true
	}
	if pathIsGitGlobalConfig(canonicalTarget) {
		return true
	}
	// The same unconditional reach, for the same reason, aimed at the other
	// programs this host runs: a provider CLI's hooks/permissions/MCP files.
	if pathIsAgentInterpretedConfig(canonicalTarget) {
		return true
	}
	cfg, ok := canonicalRoot(configDir())
	if !ok {
		return true // unverifiable config dir → cannot prove the target is outside it
	}
	if !containsPathFolded(cfg, canonicalTarget) {
		return false // nothing outside the config dir is secret by location
	}
	for _, store := range configStoreRoots() {
		sr, ok := canonicalRoot(store)
		if !ok {
			continue
		}
		// A carve-out only ever NARROWS the gate, so everything about this arm
		// is fail-closed: byte-exact (folding here would exempt <configDir>/
		// LIBRARY), and the resolved carve-out must still be STRICTLY inside the
		// resolved config dir. Without that last test a symlink at
		// <configDir>/library aimed at its own parent — and <configDir>/library
		// is the one directory a remote caller can fs.write into — resolved the
		// carve-out to the config dir itself, which contains everything in it:
		// one symlink disarmed the entire gate and handed out remote-token.
		if sr == cfg || !containsPath(cfg, sr) {
			continue
		}
		if containsPath(sr, canonicalTarget) {
			return false
		}
	}
	return true
}

// gitMetadataDir is the one directory name that turns an ordinary write into
// command execution. See traversesGitDir.
const gitMetadataDir = ".git"

// traversesGitDir reports whether any component of an ALREADY canonical path is
// the repository metadata directory.
//
// A `.git` directory is not data, it is a program: `git` discovers the
// repository at whatever cwd it is handed and then executes what `.git/config`
// tells it to. The `-c` prefix in fsops.go / gitExec.ts neutralizes every
// exec-valued key that has a FIXED name, and `--no-ext-diff` covers
// diff.external, but nothing there can reach the NAMESPACED ones —
// filter.<drv>.clean (which `git add`, i.e. git.stage, runs),
// diff.<drv>.command/textconv, merge.<drv>.driver, trailer.<t>.command — because
// the driver name belongs to whoever wrote the file. So the write is refused
// here instead. `.git/hooks/*`, `.git/config.worktree` and
// `.git/info/attributes` are the same surface and the same rule covers them.
//
// This is HALF of the definition-site answer: git reads the same namespaced keys
// out of the per-user global config, which has no `.git` component at all. See
// pathIsGitGlobalConfig for the other half.
//
// Reads are refused too, on the same footing as the credential basenames: a
// `.git/config` routinely holds a remote URL with an embedded token and the name
// of a credential store. Nothing legitimate goes through these guards for a
// `.git` path — both providers' directory listings already drop the entry.
//
// Folded, because ".GIT" opens .git on APFS and NTFS, and the walk hands this
// gate the caller's spelling (see containsPathFolded). The FINAL component
// counts as well as the interior ones: a FILE named `.git` is the "gitfile"
// pointer form (`gitdir: ...`) and is equally a repository.
//
// TWIN: pathConfinement.ts traversesGitDir, internal/bus/policy.go traversesGitDir.
func traversesGitDir(canonicalTarget string) bool {
	for _, comp := range strings.Split(canonicalTarget, string(filepath.Separator)) {
		if asciiLower(comp) == gitMetadataDir {
			return true
		}
	}
	return false
}

// agentConfigDirs are provider CONFIG-HOME directory names. Everything at or
// under one is refused: the whole directory is that provider's configuration
// namespace and every one of them holds at least one file that is a command
// line. `.opencode` holds plugin/*.js, which opencode LOADS AND RUNS at startup
// with no manifest and no other file required — reachable from
// providers.listModels, which the consent list labels "List available models".
// `.codex` holds config.toml, whose mcp_servers entries are command+args+env.
//
// TWIN: AGENT_CONFIG_DIRS in pathConfinement.ts, agentConfigDirs in
// internal/bus/policy.go.
var agentConfigDirs = map[string]bool{".opencode": true, ".codex": true}

// agentConfigBasenames are provider config FILES, denied by name wherever they
// resolve. `.mcp.json` is Claude Code's project MCP-server file (each entry is a
// command + args the CLI launches) and `.claude.json` is its per-user twin;
// opencode.json/.jsonc carry opencode's `mcp` block, same shape.
//
// TWIN: AGENT_CONFIG_BASENAMES in pathConfinement.ts, agentConfigBasenames in
// internal/bus/policy.go.
var agentConfigBasenames = map[string]bool{
	".mcp.json":      true,
	".claude.json":   true,
	"opencode.json":  true,
	"opencode.jsonc": true,
}

// claudeConfigChildren are the children of a `.claude` directory that are read
// as POLICY and ARGV rather than as instructions. The `.claude` subtree is NOT
// denied wholesale: library.save legitimately writes .claude/skills/<id>/SKILL.md,
// .claude/agents/<id>.md and .claude/commands/<id>.md through this same guard,
// and those are INSTRUCTIONS an agent still acts on through its own approvals.
// These three sit AHEAD of the approvals instead of behind them.
//
// TWIN: CLAUDE_CONFIG_CHILDREN in pathConfinement.ts and internal/bus/policy.go.
var claudeConfigChildren = map[string]bool{
	"settings.json":       true,
	"settings.local.json": true,
	"hooks":               true,
}

// claudeConfigDirName is the component claudeConfigChildren hang off.
const claudeConfigDirName = ".claude"

// pathIsAgentInterpretedConfig reports whether an ALREADY canonical path is
// agent-interpreted configuration: a file a provider CLI reads as hooks,
// permissions, or an MCP command line rather than as project data.
//
// This is traversesGitDir's argument aimed at the OTHER programs this host runs.
// `.git/config` is refused because git "discovers the repository at whatever cwd
// it is handed and then executes what `.git/config` tells it to";
// `<cwd>/.claude/settings.json` is that sentence with `claude` in it. Its
// hooks.SessionStart[].hooks[].command runs as the desktop user at session start
// — before any model call, with no approval prompt, no permission mode and no
// PreToolUse gate — and its permissions.allow / permissions.defaultMode rewrite
// the approval policy for every agent started in that project, the LOCAL user's
// included.
//
// Every guard treated these as ordinary project DATA: inside a root, not a
// credential basename, no `.git` component. The composition that made that fatal
// is two calls each of which is correctly confined —
//
//	fs.write     <agentCwd>/.claude/settings.json   (inside a live agent cwd)
//	agents.spawn { cwd: <agentCwd> }                (unconfined BY DECISION)
//
// — and needs nothing else. agents.spawn with no cwd normalizes to $HOME
// (normalizeCwd), which makes the home tree a root and puts
// ~/.claude/settings.json in reach: the same two calls then plant a hook that
// fires for EVERY claude session on the host, in any project, permanently.
//
// The codebase had made this argument twice already and stopped short of the
// write gate both times — claudeProfiles.ts drops a bus-written profile's
// configDir because "that directory supplies claude's settings.json —
// permissions.allow and hooks, i.e. commands claude runs unprompted", and
// plugin/manager.go drops a path scope resolving to "/" because it "granted the
// plugin fs.write on ~/.claude/settings.json (hooks are arbitrary commands)".
// Both closed one door onto the file; this closes the file.
//
// Reads are refused with writes, on the `.git/config` footing: a settings.json
// carries apiKeyHelper/awsAuthRefresh and env blocks, and a .mcp.json carries
// server credentials in `env`.
//
// TWIN: isAgentInterpretedConfigPath in pathConfinement.ts,
// pathIsAgentInterpretedConfig in internal/bus/policy.go.
func pathIsAgentInterpretedConfig(canonicalTarget string) bool {
	comps := strings.Split(canonicalTarget, string(filepath.Separator))
	for i := range comps {
		comps[i] = asciiLower(comps[i])
	}
	if len(comps) > 0 && agentConfigBasenames[comps[len(comps)-1]] {
		return true
	}
	for i, c := range comps {
		if agentConfigDirs[c] {
			return true
		}
		// The child is matched at i+1 ONLY: `.claude/skills/hooks/…` is a skill
		// named `hooks`, not the hook directory, and denying it would take a
		// legitimate library.save target out.
		if c == claudeConfigDirName && i+1 < len(comps) && claudeConfigChildren[comps[i+1]] {
			return true
		}
	}
	return false
}

// gitGlobalConfigBasename is git's per-user configuration file. It gets its own
// gate rather than joining secretBasenames because the reason is different: this
// is not a credential, it is a place to define a PROGRAM git will run.
const gitGlobalConfigBasename = ".gitconfig"

// gitXdgConfigDir is git's other per-user configuration directory —
// $XDG_CONFIG_HOME/git, or $HOME/.config/git — holding `config`, `attributes`
// and `ignore`. Returns the RESOLVED directory, or ok=false when neither the
// environment nor the home directory yields one.
//
// The WHOLE directory is canonicalized, `git` component included — not the base
// alone with `git` appended afterwards. `~/.config/git -> ~/dotfiles/git` is the
// ordinary stow/chezmoi/yadm arrangement, and the target this is compared
// against has already been resolved per component, so appending an UNRESOLVED
// component compares a resolved path against an unresolved directory and can
// only ever miss. That miss took git's per-user config out of the gate entirely
// on every dotfiles host: basename `config`, no `.git` component, nowhere near
// the workspacer config dir, so nothing else caught it and fs.write could define
// filter.<drv>.clean. The workspacer store carve-outs already resolve the whole
// path this way ("a store carve-out is canonicalized too"); git's did not.
//
// A missing directory still canonicalizes (the walk keeps going on ENOENT), so
// this stays comparable on a host that has never created either component.
func gitXdgConfigDir() (string, bool) {
	base := os.Getenv("XDG_CONFIG_HOME")
	if !filepath.IsAbs(base) {
		home, err := os.UserHomeDir()
		if err != nil || home == "" {
			return "", false
		}
		base = filepath.Join(home, ".config")
	}
	return canonicalRoot(filepath.Join(base, "git"))
}

// pathIsGitGlobalConfig reports whether an ALREADY canonical path is one of
// git's per-user configuration files — the OTHER place a namespaced exec driver
// can be defined.
//
// gitExec.ts's third mechanism used to claim that filter.<drv>.clean and its
// siblings "have to be DEFINED in a config file inside the repository's `.git`
// directory", which traversesGitDir refuses. That was false. git reads those
// keys from ~/.gitconfig and $XDG_CONFIG_HOME/git/config too, and $HOME is an
// ordinary workspace root the moment any agent's cwd is $HOME — which is what a
// bare agents.spawn({}) produces, since normalizeCwd("") returns the home
// directory. Neither file is a secretBasename, neither is under the config dir,
// and neither carries a `.git` component, so fs.write allowed both; the `*
// filter=drv` half of the chain is an ordinary .gitattributes that nothing
// refuses (nor should it). The definition site was the only thing left to close.
//
// It cannot be closed from the exec side: `-c` can only SET keys, and
// neutralizing GIT_CONFIG_GLOBAL wholesale would drop core.excludesFile, which
// check-ignore, status and add all depend on.
//
// THREE clauses, because the file moves:
//  1. the BASENAME anywhere, folded like the credential basenames;
//  2. whatever <home>/.gitconfig RESOLVES to — a global config symlinked into a
//     dotfiles repo is an ordinary arrangement, and that repo can be an agent
//     cwd;
//  3. anything at or inside the resolved $XDG_CONFIG_HOME/git.
//
// Reads go with writes, as for .git/config: a global config routinely holds
// credential-helper settings and url.<base>.insteadOf rewrites carrying tokens.
//
// TWIN: pathConfinement.ts isGitGlobalConfigPath, internal/bus/policy.go
// pathIsGitGlobalConfig.
func pathIsGitGlobalConfig(canonicalTarget string) bool {
	if asciiLower(canonicalBase(canonicalTarget)) == gitGlobalConfigBasename {
		return true
	}
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		if resolved, err := canonicalizePath(filepath.Join(home, gitGlobalConfigBasename)); err == nil {
			if resolved == canonicalTarget {
				return true
			}
		}
	}
	if xdgGit, ok := gitXdgConfigDir(); ok && containsPathFolded(xdgGit, canonicalTarget) {
		return true
	}
	return false
}

// canonicalBase is the substring after the final canonical separator — "" when
// the path is exactly a volume prefix. filepath.Base is not used: it Cleans, and
// it answers "/" rather than "" for the root.
func canonicalBase(p string) string {
	i := strings.LastIndexByte(p, filepath.Separator)
	if i < 0 {
		return p
	}
	return p[i+1:]
}

// assertPathAllowed rejects a call whose path escapes the allowed roots or lands
// on a credential file, and RETURNS the canonical path it validated. Every call
// site must hand that value to the filesystem operation: check-path and
// opened-path cannot differ if there is only one string (BINDING DECISION 2).
// Canonicalization runs exactly once per call, not once per gate.
//
// The message deliberately does not echo the target, the resolved path or the
// matched root, and is the same for all three refusals — it goes to a remote
// caller, and confirming where a denied path landed (or that it hit something
// worth protecting) is a probe primitive.
func assertPathAllowed(capability, target string, roots []string) (string, error) {
	refuse := func() (string, error) {
		return "", fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config stores)", capability)
	}
	ct, err := canonicalizePath(target)
	if err != nil {
		return refuse()
	}
	if !pathWithinRootsCanonical(roots, ct) {
		return refuse()
	}
	if pathIsSecretCanonical(ct) {
		return refuse()
	}
	return ct, nil
}

// resetCwdCacheForTest drops the memoized root list. Tests change what counts as
// a live cwd between cases, and would otherwise see a stale allow-list.
func resetCwdCacheForTest() {
	cwdCacheMu.Lock()
	cwdCacheAt = time.Time{}
	cwdCacheVals = nil
	cwdCacheMu.Unlock()
}
