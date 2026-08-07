package capspec

import (
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"sort"
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

// contractFixtureRel is the cross-language path-containment corpus, relative to
// this package dir: services/hub/internal/capspec → repo root is four levels up.
const contractFixtureRel = "../../../../contracts/path-containment-cases.json"

// fixtureMethod is the fixture's `methods` block: the corpus's copy of "which
// capability is path-scoped, and which params field carries the path". Only the
// fields this package can hold an opinion about are decoded — `params` and the
// per-entry notes belong to the loaders that actually call the providers.
type fixtureMethod struct {
	Method    string   `json:"method"`
	Field     string   `json:"field"`
	RootSet   string   `json:"rootSet"`
	Providers []string `json:"providers"`
}

// fixtureRootSets and fixtureProviders are the closed vocabularies the methods
// block may use. `workspace` is the live-agent cwd set and `browse` the wider
// home tree the directory picker and the pre-spawn library listing need;
// providers name which door answers the call — the brain, the desktop's
// registerCapability provider, or the cat-door methods main only serves under
// WORKSPACER_NO_BRAIN=1. A typo in either would silently mean nothing to every
// loader that filters on them, which is how a corpus stops covering what it
// claims to cover.
var fixtureRootSets = map[string]bool{"workspace": true, "browse": true}

var fixtureProviders = map[string]bool{"brain": true, "main": true, "main-killswitch": true}

// TestPathContainmentFixtureCoversPathParam closes the loop on the fixture's
// method list, which is a FOURTH copy of the thing this package exists to keep
// in one place (after PathParam itself, the desktop registrations, and the
// brain's dispatch tables). Copies drift; the config_defaults.json codegen check
// is the same shape and exists for the same reason.
//
// Both directions are failures, and they are different failures. A PathParam
// entry with no fixture entry means a path-bearing capability shipped with no
// behavioural corpus — nothing anywhere asserts that its provider reaches the
// guard. A fixture entry with no PathParam entry is worse in the other
// direction: the corpus is exercising confinement the bus does not apply, so it
// reads as coverage while the method is grantable to a plugin unconfined.
func TestPathContainmentFixtureCoversPathParam(t *testing.T) {
	raw, err := os.ReadFile(contractFixtureRel)
	if err != nil {
		t.Fatalf("read %s: %v", contractFixtureRel, err)
	}
	var fx struct {
		Methods []fixtureMethod `json:"methods"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", contractFixtureRel, err)
	}
	if len(fx.Methods) == 0 {
		t.Fatalf("%s has no methods block — the corpus can no longer say which capabilities it covers", contractFixtureRel)
	}

	inFixture := map[string]fixtureMethod{}
	for _, m := range fx.Methods {
		if _, dup := inFixture[m.Method]; dup {
			t.Errorf("%s lists %q twice in methods", contractFixtureRel, m.Method)
		}
		inFixture[m.Method] = m
	}

	for _, method := range sortedFixtureMethods(inFixture) {
		m := inFixture[method]
		field, scoped := PathParam[method]
		if !scoped {
			t.Errorf("the fixture's methods block has %q, but capspec.PathParam does not — the bus grants that method with no path confinement, so the corpus is pinning a guard that never runs", method)
			continue
		}
		// Which params key the guard reads. Disagreeing on it is the quiet
		// version of no guard at all: the corpus injects its traversal into one
		// field while the bus confines another, and the case passes for the
		// wrong reason.
		if m.Field != field {
			t.Errorf("the fixture injects %q's path into %q, but capspec.PathParam says the bus reads %q", method, m.Field, field)
		}
		if !fixtureRootSets[m.RootSet] {
			t.Errorf("the fixture's rootSet for %q is %q; want one of workspace, browse", method, m.RootSet)
		}
		if len(m.Providers) == 0 {
			t.Errorf("the fixture entry for %q names no providers — nothing says which side has to enforce it", method)
		}
		for _, p := range m.Providers {
			if !fixtureProviders[p] {
				t.Errorf("the fixture says %q is provided by %q; want one of brain, main, main-killswitch", method, p)
			}
		}
	}

	for _, method := range sortedFixtureMethods(PathParam) {
		if _, ok := inFixture[method]; !ok {
			t.Errorf("capspec.PathParam has %q but the fixture's methods block does not — a path-scoped capability shipped with no containment corpus behind it", method)
		}
	}
}

// sortedFixtureMethods gives map iteration a stable order, so a drift failure
// reads the same on every run.
func sortedFixtureMethods[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// pathishParams are param field names that carry a filesystem location. A
// capability destructuring one of these is reaching into the host filesystem
// (or handing a path to something that does), so it must be classified: scoped
// in PathParam, or written down in unscopedByDecision with the reason.
var pathishParams = map[string]bool{
	"path": true, "cwd": true, "dir": true, "filePath": true, "root": true, "paths": true,
	// `filename` was the blind spot that let sessions.load / sessions.save /
	// sessions.delete ship as a FOURTH, unclassified copy of path containment:
	// their names carry no fs./library./git. prefix so LooksPathBearing is false,
	// and without this entry the params scan could not see them either. Their two
	// providers had drifted (a bare-basename rule in Go, a lexical
	// resolve+startsWith in TypeScript that read and unlinked through a symlink)
	// with both suites green.
	"filename": true,
	// Two more names that are process/filesystem identifiers rather than
	// "path"-shaped, and were invisible for the same reason `filename` was:
	//
	//   `shell`     — terminals.create's argv[0], handed straight to
	//                 Command::new / claudemonSessionClient.spawn with no
	//                 existence check, no PATH resolution and no containment. The
	//                 capability's unscopedByDecision reason named `cwd` and only
	//                 `cwd`, so the record was incomplete in exactly the way
	//                 sessions.*'s silence was.
	//   `configDir` — claude.profiles.add/update's persisted CLAUDE_CONFIG_DIR:
	//                 the directory supplying claude's settings.json, i.e.
	//                 permissions.allow and hooks. `dir` was in this list and
	//                 `configDir` was not, and the scan matches names exactly.
	"shell":     true,
	"configDir": true,
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

// guardCallRe matches a confinement call that names `method` as a string
// literal: either assertPathAllowed('fs.read', …) directly, or one of the
// small guardXxx wrappers that pass the capability name through to it
// (guardGitCwd('git.diff', …), guardLibraryCwd('library.save', …)). The callee
// is captured so the canary below can tell the two shapes apart. Single-quoted
// literals only, matching the file's convention for capability names.
func guardCallRe(method string) *regexp.Regexp {
	return regexp.MustCompile(`\b(assertPathAllowed|guard[A-Za-z]*)\(\s*'` + regexp.QuoteMeta(method) + `'`)
}

// TestUnscopedByDecisionProviderClaimsAreTrue holds unscopedByDecision to its
// own word.
//
// Nine git.* methods are excused from PathParam on one stated ground: "provider-
// confined to the workspace roots (guardGitCwd)". That sentence is the ONLY
// thing standing between a bus caller (web / remote / MCP / any trusted
// connection) and `git.commitDiff` on an arbitrary checkout, or `git.stage` on
// an arbitrary index — and nothing checked it. TestDesktopPathCapabilitiesAreGuarded
// iterates PathParam, which by construction excludes everything listed here, so
// six of the nine (git.log, git.numstat, git.commitDiff, git.commitNumstat,
// git.stage, git.unstage) could have their guardGitCwd() call deleted with the
// entire Go and desktop suites staying green.
//
// So: any entry whose reason NAMES a guard helper must actually call that helper
// with its own method name, in its own handler body. This is the same slice-to-
// the-next-registration parse the tests above use, for the same reason — a guard
// belonging to the capability registered above must not be read as this one's.
// The behavioural half lives in hubCapabilities.test.ts; this half is what makes
// a silent deletion impossible.
func TestUnscopedByDecisionProviderClaimsAreTrue(t *testing.T) {
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
	bodies := map[string]string{}
	for i, site := range sites {
		end := len(text)
		if i+1 < len(sites) {
			end = sites[i+1][0]
		}
		bodies[text[site[2]:site[3]]] = text[site[0]:end]
	}

	// The helper names a reason may claim. Adding one here is how a future
	// "confined by the provider" excuse becomes enforceable rather than prose.
	helpers := []string{"guardGitCwd", "assertPathAllowed"}
	checked := 0
	for _, method := range sortedFixtureMethods(unscopedByDecision) {
		reason := unscopedByDecision[method]
		var claimed string
		for _, h := range helpers {
			if strings.Contains(reason, h) {
				claimed = h
				break
			}
		}
		if claimed == "" {
			continue // the reason rests on something this file cannot read (see replay.*, agents.spawn)
		}
		body, ok := bodies[method]
		if !ok {
			t.Errorf("unscopedByDecision says %s is confined by %s in the provider, but hubCapabilities.ts registers no such capability — the excuse names a handler that does not exist", method, claimed)
			continue
		}
		if !guardCallRe(method).MatchString(body) {
			t.Errorf("unscopedByDecision excuses %s on the grounds that the provider confines it (%q), but its handler in %s never calls %s('%s', …) — every bus caller reaches it with an unconfined cwd", method, reason, src, claimed, method)
			continue
		}
		checked++
	}
	if checked == 0 {
		t.Fatalf("no unscopedByDecision entry claimed a provider-side guard this test could verify — either the reasons were reworded or the parse broke, and in both cases this guard is guarding nothing")
	}
	// The git block is the reason this test exists; losing it silently would put
	// the six unpinned methods straight back where they were.
	if checked < 9 {
		t.Errorf("expected at least the nine git.* provider-confinement claims to be verified, got %d", checked)
	}
}

// TestDesktopPathCapabilitiesAreGuarded is the second half of the question the
// scan above asks. TestDesktopCapabilitiesAllScoped only asks whether a
// registered method is CLASSIFIED — that capspec has an opinion about it.
// Classification is bookkeeping: it makes the bus confine a *plugin's* call to
// that plugin's granted roots. It says nothing about the provider's own door,
// which the desktop UI, the web client and the remote client all come through
// with a trusted connection and no grants at all. A capability that is in
// PathParam but whose handler never calls the guard is confined for plugins and
// wide open for everyone else.
//
// So: for every registration site whose name is in PathParam, the handler body
// must contain a guard call naming that method. The body is the slice from this
// registration site to the next one — the same slicing
// TestCapabilitiesWithAPathParamAreClassified uses, and for the same reason: a
// guard belonging to the capability registered above must not be read as this
// one's. Requiring the method name as a literal is what makes that precise; a
// bare assertPathAllowed(cap, …) inside a shared helper is attributed to
// nobody, and the wrappers pass the literal at the call site for exactly that
// reason.
//
// Sites registered through `cat` are scanned identically to `registerCapability`
// ones. Which door serves a method is a routing decision — cat-door methods
// normally go to the brain, which guards them in fsguard.go — but
// WORKSPACER_NO_BRAIN=1 opens main's copy instead, so main's handler needs the
// guard regardless of who answers on a normal boot.
//
// Skips (not fails) when the TS source isn't reachable, since it's cross-repo.
func TestDesktopPathCapabilitiesAreGuarded(t *testing.T) {
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

	guarded := 0
	sawDirect, sawWrapper := false, false
	for i, site := range sites {
		name := text[site[2]:site[3]]
		if _, scoped := PathParam[name]; !scoped {
			continue
		}
		end := len(text)
		if i+1 < len(sites) {
			end = sites[i+1][0]
		}
		m := guardCallRe(name).FindStringSubmatch(text[site[0]:end])
		if m == nil {
			t.Errorf("hubCapabilities.ts registers %q, which capspec.PathParam scopes, but its handler never calls assertPathAllowed('%s', …) or a guard wrapper naming it — every trusted caller (UI, web, remote) reaches the filesystem through it unconfined", name, name)
			continue
		}
		guarded++
		if m[1] == "assertPathAllowed" {
			sawDirect = true
		} else {
			sawWrapper = true
		}
	}
	if guarded == 0 {
		t.Fatalf("matched no path-scoped capability at all in %s — capspec.PathParam and the registrations have stopped overlapping, so this guard is guarding nothing", src)
	}
	// One canary per call shape, for the same reason the classification scan
	// names one capability per parsing idiom: the fs.* methods and
	// search.project call assertPathAllowed directly, while library.save/remove
	// and git.diff route through guardLibraryCwd/guardGitCwd. If either shape
	// stops matching, this test silently covers half the surface it claims to —
	// so losing one has to fail out loud rather than shrink the loop.
	if !sawDirect {
		t.Errorf("no path-scoped capability in %s was seen calling assertPathAllowed('<method>', …) directly — the direct-call shape has stopped matching, so every capability written that way is now unchecked by this test", src)
	}
	if !sawWrapper {
		t.Errorf("no path-scoped capability in %s was seen guarded through a guardXxx('<method>', …) wrapper — the wrapper shape has stopped matching (renamed helper?), so library.*/git.* are now unchecked by this test", src)
	}
}
