// Cross-provider drift guard for the catalog-delegation split.
//
// The desktop registers bus capabilities through one of two doors:
//
//	cat('m', …)                 — a no-op when the catalog is delegated to this
//	                              brain (the default). The brain must provide m.
//	registerCapability('m', …)  — main always provides m itself.
//
// Choosing the wrong door fails silently in the worst way: the method simply has
// no provider on the bus, every remote call errors at runtime, and nothing fails
// at build or test time. That has now happened twice — fs.readImage was shipped
// behind `cat` with no brain counterpart (thumbnails dead for every web/remote
// client), and the fs.* path confinement was written behind a door the default
// configuration never opens (arbitrary host reads/writes for any bus client).
//
// So: assert the two provider lists partition the surface, in both directions.
package main

import (
	"regexp"
	"strings"
	"testing"
)

var (
	catRe = regexp.MustCompile(`(?m)^\s*cat\(\s*'([a-zA-Z][\w.]*)'`)
	regRe = regexp.MustCompile(`(?m)^\s*registerCapability\(\s*'([a-zA-Z][\w.]*)'`)
)

func readDesktopCapabilities(t *testing.T) string {
	t.Helper()
	// A missing twin is a FAILURE, not a skip — see mustReadRepoFile. Skipping
	// on any read error is how a renamed hubCapabilities.ts turns this
	// delegation guard off while the package still prints `ok`.
	return string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "services", "hubCapabilities.ts"))
}

func names(re *regexp.Regexp, body string) []string {
	var out []string
	for _, m := range re.FindAllStringSubmatch(body, -1) {
		out = append(out, m[1])
	}
	return out
}

func brainScopeSet(scope string) map[string]bool {
	r := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	set := map[string]bool{}
	for _, m := range r.methodsForScope(scope) {
		set[m] = true
	}
	return set
}

func brainMethodSet() map[string]bool {
	// methods() is the full surface; catalog scope registers a subset. Delegation
	// hands the catalog scope to the brain, so that is the set that must cover
	// every `cat` method.
	return brainScopeSet("catalog")
}

// Every method the desktop delegates (registers via `cat`) must be provided by
// this brain in catalog scope — otherwise it has no provider at all by default.
func TestEveryDelegatedCapabilityHasABrainProvider(t *testing.T) {
	body := readDesktopCapabilities(t)
	delegated := names(catRe, body)
	if len(delegated) == 0 {
		t.Fatal("parsed no cat(...) capability names — the registration syntax changed; update catRe")
	}
	brain := brainMethodSet()
	for _, m := range delegated {
		if !brain[m] {
			t.Errorf("hubCapabilities.ts delegates %q via cat(...) but the brain's catalog scope does not provide it — "+
				"under the default configuration this method has NO provider on the bus. "+
				"Either implement it in the brain or register it with registerCapability.", m)
		}
	}
}

// The mirror image: a method main keeps for itself must NOT also be claimed by
// the brain. The router is single-owner per method, so two providers race and
// whichever registers first silently wins — the loser's registration is
// withheld and its implementation is never invoked again.
//
// Checked against BOTH brain scopes. Catalog scope is what the desktop spawns;
// FULL scope is what runs when the desktop ADOPTS a `workspacer serve` hub
// (hubDaemon adopt-don't-kill), which is a supported configuration the
// catalog-only check could not see at all — so a full-scope collision was
// unreportable, and the escape hatch below protected nothing because catalog
// scope never contained any of it.
//
// Each entry in declaredOverlap is a DEGRADATION the user gets in the adopted
// configuration, with the reason on the record. Adding one is a decision, not a
// silencer: whatever is listed here must also be disclosed where the user can
// see it (see hubCapabilities.ts's adopted-hub note).
func TestMainOwnedCapabilitiesDoNotCollideWithTheBrain(t *testing.T) {
	body := readDesktopCapabilities(t)
	// method → what the user gets when the BRAIN wins the race (it registers
	// first, because it is already connected when the desktop starts).
	//
	// `equivalent` means the brain's implementation proxies the same claudemon
	// the desktop would have, so nothing observable changes. `degraded` means
	// the brain's version is a stand-in and the user loses something — those are
	// the ones that must be disclosed where a user can see them.
	type overlap struct {
		degraded bool
		why      string
	}
	declaredOverlap := map[string]overlap{
		// Degraded: the brain cannot do what main does.
		"notifications.post": {true, "the brain only LOGS a notification (handlers.go notify); main raises a real OS toast. Adopted → plugin/agent notifications stop reaching the OS."},
		"analytics.summary":  {true, `the brain answers an all-zero stub carrying unavailable:"headless"; main has the real SQLite session-history store. Adopted → every analytics caller (plugins can hold this grant) gets zeros.`},
		"analytics.recent":   {true, "same stub as analytics.summary — an empty row list rather than the desktop's session history."},

		// Equivalent: both sides proxy the SAME claudemon over its HTTP API, so
		// whichever owns the method answers identically.
		"agents.list":                {false, "both read claudemon's session list through the same visibility filter"},
		"agents.spawn":               {false, "both POST claudemon /spawn"},
		"agents.sendMessage":         {false, "both POST claudemon /message"},
		"terminals.create":           {false, "both POST claudemon /terminals"},
		"claude.approve":             {false, "both POST claudemon's approval endpoint"},
		"claude.answer":              {false, "both POST claudemon's answer endpoint"},
		"claude.signal":              {false, "both POST claudemon's signal endpoint"},
		"claude.gate":                {false, "both POST claudemon's gate endpoint"},
		"sessions.transcript":        {false, "both read claudemon's transcript"},
		"sessions.conversation":      {false, "both read claudemon's conversation"},
		"sessions.snapshots":         {false, "both serve the same claudemon-backed snapshots"},
		"sessions.snapshot":          {false, "same, for one session"},
		"sessions.attachTerminal":    {false, "both bridge claudemon's PTY stream onto the bus"},
		"sessions.detachTerminal":    {false, "same bridge, other direction"},
		"sessions.terminalInput":     {false, "same bridge"},
		"sessions.terminalResize":    {false, "same bridge"},
		"sessions.terminalKeepalive": {false, "same bridge"},
		"providers.listModels":       {false, "both shell out to the same provider binaries"},
		"providers.checkAll":         {false, "same"},
		"search.project":             {false, "both run the same ripgrep/walker over the same roots"},
		"app.getCwd":                 {false, "each answers its own process's cwd; both are the server's cwd"},
		"app.supervisorHome":         {false, "both compose the same fixed ~/.workspacer path"},
	}
	for _, scope := range []string{"catalog", "full"} {
		brain := brainScopeSet(scope)
		for _, m := range names(regRe, body) {
			if !brain[m] {
				continue
			}
			if _, declared := declaredOverlap[m]; declared {
				continue
			}
			t.Errorf("hubCapabilities.ts registers %q unconditionally while the brain's %s scope also provides it — "+
				"two providers for one method; the router keeps whichever registered first and the loser is never invoked. "+
				"Either stop registering it on one side, or add it to declaredOverlap with the degradation the user gets.", m, scope)
		}
	}
	// An entry that no longer overlaps is a stale silencer: it would keep a real
	// future collision quiet. (This is what made the old
	// allowedOverlap["notifications.post"] inert — catalog scope never had it.)
	full, catalog := brainScopeSet("full"), brainScopeSet("catalog")
	registered := map[string]bool{}
	for _, m := range names(regRe, body) {
		registered[m] = true
	}
	for m := range declaredOverlap {
		if !registered[m] {
			t.Errorf("declaredOverlap names %q, which hubCapabilities.ts no longer registers — drop it", m)
		}
		if !full[m] && !catalog[m] {
			t.Errorf("declaredOverlap names %q, which no brain scope provides — the entry protects nothing and would hide a real collision later", m)
		}
	}
	// A DEGRADED overlap is something the user loses in a supported
	// configuration. Requiring it by name in the desktop's adopted-hub note is
	// what stops the next one from being classified in a test file and nowhere
	// a human reads. (The old note asserted the opposite for analytics.* — that
	// it "registers fine" — which was factually wrong.)
	for m, o := range declaredOverlap {
		if o.degraded && !strings.Contains(body, "ADOPTED-DEGRADED: "+m) {
			t.Errorf("%q is a DEGRADED overlap (%s) but hubCapabilities.ts's adopted-hub note has no `ADOPTED-DEGRADED: %s` line — the loss would be recorded only in this test file", m, o.why, m)
		}
	}
}

// `workspacer status` asks one method whether the brain is on the bus. That
// method must be provided by the brain in EVERY scope and by nobody else —
// otherwise the brain line is green while the whole catalog plane is dead,
// which is exactly what app.getCwd did (desktop-registered, in no brain scope).
func TestBrainProbeMethodIsProvidedInEveryScopeAndOwnedOnlyByTheBrain(t *testing.T) {
	for _, scope := range []string{"catalog", "full"} {
		if !brainScopeSet(scope)[brainProbeMethod] {
			t.Errorf("%s scope does not register %q — `workspacer status` would report the brain down while it is running", scope, brainProbeMethod)
		}
	}
	body := readDesktopCapabilities(t)
	for _, m := range append(names(regRe, body), names(catRe, body)...) {
		if m == brainProbeMethod {
			t.Errorf("the desktop also registers %q — the status probe would answer for a brain that is not there", brainProbeMethod)
		}
	}
	// And it must actually answer.
	r := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	r.scope = "catalog"
	out, err := r.handle(t.Context(), brainProbeMethod, nil)
	if err != nil {
		t.Fatalf("%s is registered but the dispatcher does not handle it: %v", brainProbeMethod, err)
	}
	if !strings.Contains(string(out), `"catalog"`) {
		t.Fatalf("%s answered %s, which does not name the running scope", brainProbeMethod, out)
	}
}

// fs.readImage is the concrete instance that shipped broken, so pin it by name:
// main must own it (the brain has no thumbnail implementation).
func TestFsReadImageIsOwnedByMainNotDelegated(t *testing.T) {
	body := readDesktopCapabilities(t)
	for _, m := range names(catRe, body) {
		if m == "fs.readImage" {
			t.Error("fs.readImage is delegated via cat(...) but no brain provider implements it — " +
				"thumbnails would fail for every web and remote client")
		}
	}
	if !strings.Contains(body, "registerCapability('fs.readImage'") {
		t.Error("fs.readImage should be registered with registerCapability so main provides it under delegation")
	}
}
