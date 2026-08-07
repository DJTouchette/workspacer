package bus

import (
	"encoding/json"
	"errors"
	"os"
	"runtime"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// Filesystem path confinement for plugin capability grants.
//
// A plugin granted a path-scoped capability (fs.read, fs.write, search.project,
// …) may only touch paths inside the roots its grant declares. The trick that
// makes this safe is *canonicalize then contain*: resolve `..` and symlinks on
// both the target and the roots before the prefix check, so neither directory
// traversal nor a symlink pointing out of the root can escape. A purely textual
// prefix check would be fooled by either.
//
// TWINS. This predicate ships three times and the copies must agree case for
// case; contracts/path-containment-cases.json is what enforces that:
//
//	services/hub/cmd/brain/fsguard.go        (Go, answers fs.*/library.* by default)
//	apps/desktop/src/main/lib/pathConfinement.ts (TypeScript, the killswitch path)
//	this file                                (per-plugin grant confinement)
//
// Note the argument orders DIFFER on purpose and each file keeps its own local
// convention: here it is within(root, target); the brain spells the same
// predicate isWithin(canonicalTarget, root) — target first. Only the BEHAVIOUR is
// shared. Read the argument names, not the position, when porting a change.

// MAX_LINK_HOPS from the spec: the walk below is hand-rolled rather than
// delegated to the platform realpath, so a symlink cycle has to be terminated by
// a counter or it spins forever.
const maxLinkHops = 40

var (
	// errNotAbsolute is returned for an empty, whitespace-only or relative path.
	// Resolving one against the process working directory would silently make the
	// daemon's own cwd a reachable location, so it is refused outright.
	errNotAbsolute = errors.New("path is empty or not absolute")
	// errTooManyLinks terminates a symlink cycle (ELOOP, computed by us).
	errTooManyLinks = errors.New("too many symbolic links")
	// errSecretPath marks the SECRET refusal so the caller can pick the
	// non-echoing message for it (spec 7.5) without canonicalizing a second time
	// (spec 7.6). Every caller treats a non-nil error as a denial regardless.
	errSecretPath = errors.New("path is a credential or config-store file")
)

// --- platform primitives (spec 0.1-0.2) -------------------------------------

// canonicalSep is the separator used for joining and for the containment
// comparison. isSep matches the wider SPLITTING set: Windows accepts "/" too.
func canonicalSep() string {
	if runtime.GOOS == "windows" {
		return `\`
	}
	return "/"
}

func isSep(b byte) bool {
	if b == '/' {
		return true
	}
	return runtime.GOOS == "windows" && b == '\\'
}

func isDriveLetter(b byte) bool {
	return (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z')
}

// splitVolume peels the volume prefix off an absolute path (spec 0.2) and
// returns the remaining component region. ok is false when s is not absolute —
// which includes every path beginning with "~" (BINDING DECISION 1: nobody
// expands a tilde, at any layer; "~" is an ordinary component).
func splitVolume(s string) (volume, region string, ok bool) {
	if runtime.GOOS == "windows" {
		// UNC: \\server\share\...
		if len(s) >= 2 && isSep(s[0]) && isSep(s[1]) {
			i := 2
			for i < len(s) && !isSep(s[i]) {
				i++
			}
			server := s[2:i]
			if server == "" || i >= len(s) {
				return "", "", false
			}
			j := i + 1
			for j < len(s) && !isSep(s[j]) {
				j++
			}
			share := s[i+1 : j]
			if share == "" {
				return "", "", false
			}
			return `\\` + server + `\` + share + `\`, s[j:], true
		}
		// Drive: C:\...  (a drive-relative "C:foo" is NOT absolute)
		if len(s) >= 3 && isDriveLetter(s[0]) && s[1] == ':' && isSep(s[2]) {
			return string(s[0]) + `:\`, s[3:], true
		}
		return "", "", false
	}
	if len(s) > 0 && s[0] == '/' {
		return "/", s[1:], true
	}
	return "", "", false
}

// isAbsolutePath reports whether s carries a valid volume prefix (spec 0.3).
func isAbsolutePath(s string) bool {
	_, _, ok := splitVolume(s)
	return ok
}

// splitComponents splits a component region on runs of separator characters,
// discarding "" and "." — and ONLY those. ".." survives as a component and is
// handled by the walk, which is the whole point: collapsing it here textually is
// the defect this file exists to remove.
func splitComponents(region string) []string {
	out := make([]string, 0, 8)
	start := 0
	for i := 0; i <= len(region); i++ {
		if i == len(region) || isSep(region[i]) {
			c := region[start:i]
			if c != "" && c != "." {
				out = append(out, c)
			}
			start = i + 1
		}
	}
	return out
}

// appendComponent is spec 0.6 APPEND: join without normalizing anything.
func appendComponent(base, name string) string {
	if base != "" && isSep(base[len(base)-1]) {
		return base + name
	}
	return base + canonicalSep() + name
}

// parentOf is spec 0.7 PARENT: pure string arithmetic on a value the walk itself
// built, clamped at the volume prefix so ".." past the filesystem root is a
// no-op rather than an underflow into an empty prefix (which would then make
// every absolute path a suffix match).
func parentOf(base, volume string) string {
	if base == volume {
		return volume
	}
	i := strings.LastIndex(base, canonicalSep())
	if i < 0 {
		return volume
	}
	parent := base[:i]
	if parent == "" || len(parent) < len(volume) {
		return volume
	}
	return parent
}

// lastComponent is spec 6.2: everything after the final canonical separator, or
// "" when the path is exactly a volume prefix.
func lastComponent(p string) string {
	i := strings.LastIndex(p, canonicalSep())
	if i < 0 {
		return p
	}
	return p[i+1:]
}

// --- canonicalization (spec 2) ----------------------------------------------

// canonicalize resolves path one component at a time (spec 2), returning an
// absolute path with no "." or ".." component and every symlink followed.
//
// It deliberately uses NOTHING but os.Lstat and os.Readlink. filepath.Abs,
// filepath.Clean, filepath.Join-on-caller-input and filepath.EvalSymlinks are all
// forbidden here: each of them collapses "link/.." textually BEFORE any symlink
// is read, so the guard would check a path inside the root while the caller's
// original string opened a file outside it. The per-component walk is what makes
// a ".." apply to the link's TARGET.
//
// A component that does not exist is simply appended and the walk continues (a
// file fs.write is about to create, and a later ".." may pop back onto a path
// that does exist). Every other Lstat error — ENOTDIR, EACCES, ELOOP, anything
// unrecognised — fails closed; an unverifiable path is never "probably fine".
func canonicalize(path string) (string, error) {
	if strings.TrimSpace(path) == "" {
		return "", errNotAbsolute
	}
	// Everything from here on uses the ORIGINAL string: trimming was only the
	// emptiness test, and a filename may legitimately contain spaces.
	volume, region, ok := splitVolume(path)
	if !ok {
		return "", errNotAbsolute
	}

	queue := splitComponents(region)
	resolved := volume
	hops := 0

	for len(queue) > 0 {
		c := queue[0]
		queue = queue[1:]

		if c == ".." {
			// No filesystem access: `resolved` is by construction already fully
			// symlink-resolved, so its textual parent IS its real parent.
			resolved = parentOf(resolved, volume)
			continue
		}

		next := appendComponent(resolved, c)
		st, err := os.Lstat(next)
		if err != nil {
			if os.IsNotExist(err) {
				resolved = next
				continue // keep resolving; a later ".." can land on real ground again
			}
			return "", err // HARD: ENOTDIR / EACCES / ELOOP / unknown → fail closed
		}
		if st.Mode()&os.ModeSymlink != 0 {
			hops++
			if hops > maxLinkHops {
				return "", errTooManyLinks
			}
			link, err := os.Readlink(next)
			if err != nil {
				return "", err
			}
			if lv, lregion, abs := splitVolume(link); abs {
				resolved = lv
				queue = append(splitComponents(lregion), queue...)
			} else {
				// Relative link: interpreted against the directory that CONTAINS
				// the link, which is `resolved` — so it is left unchanged, and
				// `next` is deliberately NOT adopted.
				queue = append(splitComponents(link), queue...)
			}
			continue
		}
		resolved = next
	}
	return resolved, nil
}

// canonicalizeRoot is spec 3: the comparable form of an allowed root, or DISCARD.
// An empty, whitespace-only, relative or "~"-prefixed root is dropped rather than
// resolved — filepath.Abs("") returns the PROCESS working directory, which would
// silently make the daemon's own cwd an allowed root. A root that does not exist
// yet still canonicalizes (the config stores are created lazily and must be
// comparable before anything has been saved).
//
// DISCARD removes that one root and nothing else: it never aborts a check and
// never denies on its own.
func canonicalizeRoot(root string) (string, bool) {
	if strings.TrimSpace(root) == "" {
		return "", false
	}
	if !isAbsolutePath(root) {
		return "", false
	}
	c, err := canonicalize(root)
	if err != nil {
		return "", false
	}
	return c, true
}

// CanonicalizeRoot exposes spec 3 to the plugin manager, which does the same
// defence-in-depth check on a manifest-declared absolute scope at load time. It
// is exported so there is ONE implementation of this walk in the hub rather than
// a second copy drifting in internal/plugin.
func CanonicalizeRoot(root string) (string, bool) { return canonicalizeRoot(root) }

// within reports whether canonical target sits at or inside canonical root
// (spec 4). The separator guard is what stops a sibling whose name shares a
// prefix — `/srv/foo` must not be considered inside root `/srv/fo`.
//
// TWIN: services/hub/cmd/brain/fsguard.go spells this isWithin(canonicalTarget,
// root) — the SAME predicate with the arguments the other way round. Keep each
// file's local order; align only the behaviour.
func within(root, target string) bool {
	if target == root {
		return true
	}
	sep := canonicalSep()
	// A root that already ends in the separator (notably the filesystem root
	// "/", or a Windows drive/UNC root) contains everything below it directly;
	// concatenating another separator would produce "//" and match nothing.
	// That is not fail-closed, it is simply wrong containment — this branch is
	// the reference the other two copies adopted (BINDING DECISION 3), so do not
	// "simplify" it away.
	if strings.HasSuffix(root, sep) {
		return strings.HasPrefix(target, root)
	}
	return strings.HasPrefix(target, root+sep)
}

// --- secret gate (spec 6, BINDING DECISION 4) --------------------------------

// secretBasenames are credential files by name, denied wherever they resolve.
// A grant's roots are only as narrow as the directories a manifest declares:
// `workspacer plugin dev <dir>` drops a .bus-token inside an ordinary project,
// so a bus token is a bus token wherever it sits. Same two names in all three
// copies (contracts/path-containment-cases.json "secretBasenames").
var secretBasenames = map[string]bool{
	".bus-token":     true, // per-plugin bus credential
	".settings.json": true, // per-plugin settings, secrets in plaintext
}

// configStoreSubdirs are the only config-dir subtrees a caller legitimately
// touches through a filesystem capability: library items, layout templates and
// saved sessions. Order matters only for readability; membership must match the
// brain's configStoreRoots and the desktop's list exactly.
var configStoreSubdirs = []string{"library", "layouts", "sessions"}

// pathIsSecret is the second gate, applied to an ALREADY canonical target after
// the roots check — reads AND writes, because handing a token out is a privilege
// promotion and overwriting one is a denial of service on the whole bus.
//
// Narrowing a root is not sufficient on its own: a plugin manifest may declare
// an absolute scope covering the config dir (plugin/manager.go used to pass any
// absolute scope through on install-time consent alone), and reading
// remote-token there lets it reconnect as a TRUSTED bus connection — which drops
// per-plugin scoping and unlocks /plugins/install, i.e. arbitrary commands. So
// anything landing in the config dir outside library/ layouts/ sessions/ is
// refused regardless of which root admitted it, and the two credential basenames
// are refused everywhere, INCLUDING inside those three carve-outs.
//
// TWIN: brain fsguard.go pathIsSecret / desktop pathConfinement.ts isSecretPath.
func pathIsSecret(canonicalTarget string) bool {
	// Basename first and unconditionally, so a credential name planted inside a
	// store carve-out is still refused (spec 6.3).
	if secretBasenames[lastComponent(canonicalTarget)] {
		return true
	}
	// Read the environment at call time — a test points this at a sandbox.
	cfg, ok := canonicalizeRoot(authtoken.ConfigDir())
	if !ok {
		return true // unverifiable config dir → cannot prove we are outside it
	}
	if !within(cfg, canonicalTarget) {
		return false // nothing outside the config dir is secret by location
	}
	for _, store := range configStoreSubdirs {
		sr, ok := canonicalizeRoot(appendComponent(cfg, store))
		if !ok {
			continue
		}
		if within(sr, canonicalTarget) {
			return false
		}
	}
	// config.yaml, remote-token, tokens.json, remote-server.json, vapid.json,
	// workspacer.db, the Electron cookie/localStorage jars, plugins/** and the
	// dir itself.
	return true
}

// PathIsSecret exposes the gate above for the plugin manager's load-time check.
// The argument must already be canonical (see CanonicalizeRoot).
func PathIsSecret(canonicalTarget string) bool { return pathIsSecret(canonicalTarget) }

// --- the check itself --------------------------------------------------------

// pathWithinRoots reports whether target resolves to a location inside one of the
// (already-canonical) roots AND is not a credential/config-dir file. The target
// is canonicalized here exactly ONCE — roots are canonicalized at grant
// registration, and the secret gate reuses this same canonical form rather than
// resolving a second time (spec 7.6).
//
// Outcomes: (true, nil) allowed; (false, nil) outside every root;
// (false, errSecretPath) inside a root but refused by the secret gate;
// (false, err) unresolvable. The two refusals are kept apart because they carry
// DIFFERENT messages — the containment refusal may name the plugin's own
// requested path, the secret one must not (spec 7.5). Every caller denies on any
// non-nil error, so the error-means-deny posture is unchanged.
func pathWithinRoots(roots []string, target string) (bool, error) {
	ct, err := canonicalize(target)
	if err != nil {
		return false, err
	}
	for _, r := range roots {
		if within(r, ct) {
			if pathIsSecret(ct) {
				return false, errSecretPath
			}
			return true, nil
		}
	}
	return false, nil
}

// paramString pulls a string field out of a call's JSON params. ok is false when
// params don't parse or aren't a JSON object, the field is absent, or it isn't a
// string that holds something other than whitespace — all of which the caller
// treats as "can't verify the path" → deny, never as a pass-through.
//
// The value is returned UNTRIMMED: trimming is only the emptiness test here, and
// canonicalize owns what a leading space in a real filename means.
//
// A CASE-VARIANT DUPLICATE of the field is refused outright, and that is not
// pedantry — it was a complete bypass of every per-plugin grant. This lookup is
// byte-exact (`m[field]`), but the providers on the other end are Go structs and
// encoding/json falls back to a CASE-INSENSITIVE field match, so a later "Path"
// overwrites the "path" this function read. `{"path":"<pluginDir>/ok.txt",
// "Path":"<victimDir>/loot.txt"}` therefore authorized one string and opened
// another: the bus confined the benign path while the brain — the default
// answerer for fs.*/library.* — read and wrote the other one. There is exactly
// one path per call or there is no call.
func paramString(params json.RawMessage, field string) (value string, ok bool) {
	if len(params) == 0 {
		return "", false
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(params, &m); err != nil {
		return "", false
	}
	for k := range m {
		if k != field && strings.EqualFold(k, field) {
			return "", false // ambiguous: guard and provider would read different keys
		}
	}
	raw, present := m[field]
	if !present {
		return "", false
	}
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return "", false
	}
	// `s != ""` used to be the whole test, which accepted "   " — and three
	// spaces then absolutized to the daemon's working directory.
	if strings.TrimSpace(s) == "" {
		return "", false
	}
	return s, true
}
