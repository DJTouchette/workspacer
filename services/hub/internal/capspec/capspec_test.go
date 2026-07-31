package capspec

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestIsPathScoped(t *testing.T) {
	cases := []struct {
		method    string
		wantField string
		wantOK    bool
	}{
		{"fs.read", "path", true},
		{"fs.write", "path", true},
		{"fs.listEntries", "path", true},
		{"fs.listDir", "path", true},
		{"fs.watch", "path", true},
		{"fs.unwatch", "path", true},
		{"search.project", "cwd", true},
		{"library.list", "cwd", true},
		{"library.save", "cwd", true},
		{"library.remove", "cwd", true},
		// git.diff is scoped on the repository it runs in, not on its optional
		// `path` pathspec — that one is repo-relative and git interprets it.
		{"git.diff", "cwd", true},
		// Not path-scoped — driving/observation/notifications.
		{"agents.list", "", false},
		{"agents.spawn", "", false},
		{"agents.sendMessage", "", false},
		{"notifications.post", "", false},
		{"config.get", "", false},
		{"", "", false},
	}
	for _, c := range cases {
		field, ok := IsPathScoped(c.method)
		if ok != c.wantOK || field != c.wantField {
			t.Errorf("IsPathScoped(%q) = (%q, %v), want (%q, %v)", c.method, field, ok, c.wantField, c.wantOK)
		}
	}
}

func TestLooksPathBearingAndMissingSpec(t *testing.T) {
	cases := []struct {
		method      string
		pathBearing bool
		missingSpec bool
	}{
		// Under a filesystem namespace and specced → path-bearing, not missing.
		{"fs.read", true, false},
		{"search.project", true, false},
		{"library.save", true, false},
		{"git.diff", true, false},
		// Under a filesystem namespace but classified nowhere → the drift we
		// guard: looks path-bearing and is missing its spec.
		{"fs.append", true, true},
		{"fs.copy", true, true},
		{"search.files", true, true},
		{"library.export", true, true},
		{"git.blame", true, true},
		// Under a namespace but deliberately left unconfined, with the reason on
		// the record: still path-bearing by name, but not missing a decision.
		{"git.status", true, false},
		{"git.push", true, false},
		// Outside the filesystem namespaces → neither path-bearing nor a concern,
		// even though some carry a cwd (spawning is a separate authz decision).
		{"agents.spawn", false, false},
		{"terminals.create", false, false},
		{"config.get", false, false},
		{"", false, false},
	}
	for _, c := range cases {
		if got := LooksPathBearing(c.method); got != c.pathBearing {
			t.Errorf("LooksPathBearing(%q) = %v, want %v", c.method, got, c.pathBearing)
		}
		if got := MissingSpec(c.method); got != c.missingSpec {
			t.Errorf("MissingSpec(%q) = %v, want %v", c.method, got, c.missingSpec)
		}
	}
}

// TestPathParamEntriesAreUnderKnownNamespaces keeps PathParam and the naming
// convention consistent: every specced path method must live under a prefix
// LooksPathBearing recognizes, or the guard in MissingSpec/authorize could never
// have flagged its unscoped sibling. If you add a path capability under a new
// namespace, add the prefix to pathVerbPrefixes too.
func TestPathParamEntriesAreUnderKnownNamespaces(t *testing.T) {
	for method := range PathParam {
		if !LooksPathBearing(method) {
			t.Errorf("PathParam has %q but LooksPathBearing(%q)=false — add its namespace to pathVerbPrefixes", method, method)
		}
	}
}

// TestEveryClassifiedMethodIsUnderAKnownNamespaceOrHasAReason keeps the two
// tables honest: a PathParam entry must live under a prefix LooksPathBearing
// recognizes (or MissingSpec could never have flagged its unscoped sibling), and
// every unscopedByDecision entry must actually carry a reason — an empty string
// there would be an oversight dressed up as a decision.
func TestEveryClassifiedMethodIsUnderAKnownNamespaceOrHasAReason(t *testing.T) {
	for method, why := range unscopedByDecision {
		if strings.TrimSpace(why) == "" {
			t.Errorf("unscopedByDecision[%q] has no reason — say why it is safe unconfined, or scope it", method)
		}
		if _, both := PathParam[method]; both {
			t.Errorf("%q is in both PathParam and unscopedByDecision — it can't be scoped and deliberately unscoped at once", method)
		}
	}
}

// pathishParams are param field names that carry a filesystem location. A
// capability destructuring one of these is reaching into the host filesystem
// (or handing a path to something that does), so it must be classified: scoped
// in PathParam, or written down in unscopedByDecision with the reason.
var pathishParams = map[string]bool{
	"path": true, "cwd": true, "dir": true, "filePath": true, "root": true, "paths": true,
}

// paramsDestructureRe pulls the `const { … } = (params` destructuring out of a
// capability handler body — the most common of the file's two ways of naming
// what a call carries. Renames (`path: p`) keep their source name on the left of
// the colon.
var paramsDestructureRe = regexp.MustCompile(`const\s*\{([^}]*)\}\s*=\s*\(params`)

// paramsAliasRe pulls the OTHER idiom: `const input = (params ?? {}) as { cwd?:
// string }`, where the params object is bound whole and its fields are reached
// through the alias. library.save is written this way — one of the three
// capabilities this detector exists because of — and search.project's `opts.cwd`
// likewise, so a detector that only understood destructuring was blind to
// exactly the drift it was added to catch.
var paramsAliasRe = regexp.MustCompile(`const\s+([A-Za-z_$][\w$]*)\s*=\s*\(params`)

// typeLiteralRe finds the `as { … }` annotation following an alias binding, whose
// field names are the second place a path-ish param shows up in that idiom (the
// first being member access on the alias). Non-nested by construction: these
// annotations are flat `{ cwd?: string }` shapes, and one that isn't simply
// contributes no fields — the member-access scan still sees the accesses.
var typeLiteralRe = regexp.MustCompile(`\bas\s*\{([^{}]*)\}`)

// pathishFieldsIn returns the path-ish params one capability's body names, by
// either idiom. Bodies are the slice from a registration site to the next one,
// so an alias and its uses can't be read across capability boundaries.
func pathishFieldsIn(body string) []string {
	var fields []string
	add := func(name string) {
		name = strings.TrimSpace(strings.SplitN(name, ":", 2)[0])
		name = strings.TrimSuffix(strings.TrimSpace(name), "?")
		if pathishParams[name] {
			fields = append(fields, name)
		}
	}
	for _, m := range paramsDestructureRe.FindAllStringSubmatch(body, -1) {
		for _, part := range strings.Split(m[1], ",") {
			add(part)
		}
	}
	for _, m := range paramsAliasRe.FindAllStringSubmatchIndex(body, -1) {
		alias := body[m[2]:m[3]]
		// The type literal, when the annotation is an inline shape rather than a
		// named type: `as { cwd?: string; kind?: … }`. Anchored at the binding so a
		// later, unrelated `as {…}` in the same body isn't attributed to it.
		rest := body[m[1]:]
		if lit := typeLiteralRe.FindStringSubmatchIndex(rest); lit != nil && lit[0] < 200 {
			for _, part := range strings.Split(rest[lit[2]:lit[3]], ";") {
				for _, f := range strings.Split(part, ",") {
					add(f)
				}
			}
		}
		// Member access on the alias, which is the only signal when the annotation
		// is a named type (`as SessionData`) or an imported parameter type.
		accessRe := regexp.MustCompile(`\b` + regexp.QuoteMeta(alias) + `\.([A-Za-z_$][\w$]*)`)
		for _, a := range accessRe.FindAllStringSubmatch(body, -1) {
			add(a[1])
		}
	}
	return fields
}

// TestCapabilitiesWithAPathParamAreClassified is the drift detector the
// namespace-prefix check can't be: it reads the capability catalog the desktop
// actually registers and flags any method that takes a path-ish param yet
// appears in neither table. Three capabilities (library.list/save/remove) sat
// unconfined for exactly this reason — their names carry no fs. prefix, so
// nothing looked at their params. Skips (not fails) when the TS source isn't
// reachable, since it's cross-repo.
//
// The canary check at the end is not ceremony: the first cut of this test read
// only the destructuring idiom, so library.save and search.project — written the
// other way — were invisible to it, and it would have passed while missing half
// the file. Naming one capability per idiom is what makes a partial blind spot
// fail rather than a total parse failure only.
func TestCapabilitiesWithAPathParamAreClassified(t *testing.T) {
	src := filepath.Join("..", "..", "..", "..", "apps", "desktop", "src", "main", "services", "hubCapabilities.ts")
	data, err := os.ReadFile(src)
	if err != nil {
		t.Skipf("hubCapabilities.ts not reachable (%v); skipping cross-repo cross-check", err)
	}
	text := string(data)
	sites := capNameRe.FindAllStringSubmatchIndex(text, -1)
	if len(sites) == 0 {
		t.Fatalf("parsed no capability names from %s — the registration syntax changed; update capNameRe", src)
	}
	sawPathParam := map[string]bool{}
	for i, site := range sites {
		name := text[site[2]:site[3]]
		end := len(text)
		if i+1 < len(sites) {
			end = sites[i+1][0]
		}
		fields := pathishFieldsIn(text[site[0]:end])
		if len(fields) == 0 {
			continue
		}
		sawPathParam[name] = true
		_, scoped := PathParam[name]
		_, excused := unscopedByDecision[name]
		if !scoped && !excused {
			t.Errorf("hubCapabilities.ts registers %q taking %v, but capspec classifies it nowhere — add a PathParam entry (with the field carrying its path) or an unscopedByDecision entry saying why it needs no confinement", name, fields)
		}
	}
	// One canary per idiom: fs.read and library.remove destructure their params,
	// library.save and search.project bind the object whole and reach `.cwd`
	// through an alias (search.project's annotation is even a named type, so only
	// the member-access scan sees it). Losing any one of these means the parser has
	// gone blind to a whole style of handler, not that a capability was renamed.
	for _, canary := range []string{"fs.read", "library.remove", "library.save", "search.project"} {
		if !sawPathParam[canary] {
			t.Errorf("did not parse a path-ish param out of %q in %s — the params-parsing regexes have stopped seeing one of the two idioms hubCapabilities.ts writes, so this detector is now blind to every capability written that way", canary, src)
		}
	}
}

// capNameRe extracts the capability method names a provider registers, matching
// both cat('name', …) and registerCapability('name', …) forms in the desktop's
// hubCapabilities.ts. Single-quoted string literals only, which is the file's
// convention for capability names.
var capNameRe = regexp.MustCompile(`(?:registerCapability|cat)\(\s*'([a-zA-Z][\w.]*)'`)

// TestDesktopCapabilitiesAllScoped cross-checks the capability names the desktop
// provider actually registers (parsed from hubCapabilities.ts) against capspec:
// any fs.*/search.* capability it exposes must have a PathParam entry. This is
// the guard that catches "a new path-bearing capability was added to the app but
// not scoped" at build time. Skips (not fails) if the TS source isn't reachable
// from this package (e.g. a hub-only checkout), since it's cross-repo.
func TestDesktopCapabilitiesAllScoped(t *testing.T) {
	// internal/capspec → repo root is four levels up (services/hub/internal/capspec).
	src := filepath.Join("..", "..", "..", "..", "apps", "desktop", "src", "main", "services", "hubCapabilities.ts")
	data, err := os.ReadFile(src)
	if err != nil {
		t.Skipf("hubCapabilities.ts not reachable (%v); skipping cross-repo cross-check", err)
	}
	matches := capNameRe.FindAllStringSubmatch(string(data), -1)
	if len(matches) == 0 {
		t.Fatalf("parsed no capability names from %s — the registration syntax changed; update capNameRe", src)
	}
	seenPathCap := false
	for _, m := range matches {
		name := m[1]
		if LooksPathBearing(name) {
			seenPathCap = true
		}
		if MissingSpec(name) {
			t.Errorf("hubCapabilities.ts registers %q, which is filesystem-scoped by name but has no capspec.PathParam entry — it would be grantable to plugins with no path confinement", name)
		}
	}
	if !seenPathCap {
		t.Errorf("expected at least one fs.*/search.* capability in hubCapabilities.ts; parsed none — capNameRe likely stopped matching")
	}
}
