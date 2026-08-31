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
		"agents.list":                   {false, "both read claudemon's session list through the same visibility filter"},
		"agents.spawn":                  {false, "both POST claudemon /spawn"},
		"agents.sendMessage":            {false, "both POST claudemon /message"},
		"terminals.create":              {false, "both POST claudemon /terminals"},
		"claude.approve":                {false, "both POST claudemon's approval endpoint"},
		"claude.answer":                 {false, "both POST claudemon's answer endpoint"},
		"claude.signal":                 {false, "both POST claudemon's signal endpoint"},
		"claude.gate":                   {false, "both POST claudemon's gate endpoint"},
		"sessions.transcript":           {false, "both read claudemon's transcript"},
		"sessions.conversation":         {false, "both read claudemon's conversation"},
		"sessions.subagentConversation": {false, "both read claudemon's provider-owned child-thread conversation"},
		"sessions.snapshots":            {false, "both serve the same claudemon-backed snapshots"},
		"sessions.snapshot":             {false, "same, for one session"},
		"sessions.attachTerminal":       {false, "both bridge claudemon's PTY stream onto the bus"},
		"sessions.detachTerminal":       {false, "same bridge, other direction"},
		"sessions.terminalInput":        {false, "same bridge"},
		"sessions.terminalResize":       {false, "same bridge"},
		"sessions.terminalKeepalive":    {false, "same bridge"},
		"providers.listModels":          {false, "both shell out to the same provider binaries"},
		"providers.checkAll":            {false, "same"},
		"search.project":                {false, "both run the same ripgrep/walker over the same roots"},
		"app.getCwd":                    {false, "each answers its own process's cwd; both are the server's cwd"},
		"app.supervisorHome":            {false, "both compose the same fixed ~/.workspacer path"},

		// The READ-ONLY git.* port. These four now have two providers, and this
		// is the entry the guard demanded when the brain started registering
		// them — so record what first-registration-wins actually means here.
		//
		// In the deployment this port exists for (a node running claudemon +
		// `brain --hub <ws>`, with the always-on hub started `--brain-scope
		// off`) there is no second provider at all: the brain is the only
		// answerer and the alternative was "no provider for git.status".
		//
		// In the ADOPTED configuration (a desktop attaching to a `workspacer
		// serve` hub on the same machine) the brain registers first and wins.
		// That is `equivalent` rather than `degraded`: same git binary, same
		// wire shapes (ipcTypes.ts GitStatus/GitLogEntry/GitNumstatEntry), same
		// confinement — assertPathAllowed over the same workspace roots, with
		// the same anchorGitPathspec treatment of git.diff's `path`. What each
		// side calls a workspace root is derived from the same claudemon, so the
		// allow-lists agree.
		//
		// AND THE ASYMMETRY THAT IS NOT A COLLISION, because it is the thing a
		// reader will look for: the brain is read-only ON PURPOSE, so
		// git.commitDiff / git.commitNumstat / git.stage / git.unstage /
		// git.commit / git.push are not method names it claims at all. The
		// desktop keeps those, and an adopted Review pane therefore reads
		// through the brain and writes through main, with no method owned twice.
		// The router is per-METHOD, so this split is stable rather than a race.
		//
		// The one case worth knowing when debugging a REMOTE node: if a desktop
		// is attached to the same hub and happens to win the race, its guard
		// confines to ITS OWN workspace roots, so a node path fails with
		// "outside the allowed workspace" rather than "no provider". Different
		// message, same blank chip — and it fails safe either way.
		"git.status":  {false, "both shell out to the same git binary through the same assertPathAllowed(workspaceRoots) guard; identical wire shape"},
		"git.log":     {false, "same — the brain's parseGitLog is a port of gitService.ts's parseLog, same --pretty format and same 1..50 clamp"},
		"git.diff":    {false, "same, including anchorGitPathspec: `path` is anchored on the derived work-tree root and the untracked leg is additionally held to the workspace roots on BOTH providers"},
		"git.numstat": {false, "same, including the core.quotepath=false prefix so unicode paths match the ones git.status printed"},

		// LIVE CONTROL (livecontrol.go). The switch itself is equivalent on all
		// three — both providers POST the same claudemon endpoints, and the
		// brain applies the SAME escalation clamp on setPermissionMode
		// (isPermissionEscalation, pinned to lib/permissionBypass.ts) — but the
		// desktop ALSO writes optimistic notes into renderer/main-process state,
		// and the brain has no way to reach those stores. The daemon remains the
		// durable selection owner; adopted providers lose only local eagerness.
		"claude.setPermissionMode": {true, "both POST claudemon /sessions/:id/permission-mode under the same bypass clamp, but the brain cannot call claudeSessionStore.notePermissionMode — the desktop's livePermissionMode follows the daemon's telemetry instead of flipping on the confirmed reply"},
		"claude.setModel":          {true, "both POST the canonical pair plus legacy companion to claudemon /sessions/:id/model, whose owner result distinguishes queued from accepted and updates durable requested_selection without claiming provider execution; the desktop mirrors owner truth immediately, while the adopted brain waits for the owner snapshot broadcast and does not stamp queued work as live telemetry"},
		"claude.setEffort":         {true, "same mechanism on both (claude: the `/effort <level>` message; managed: the /model endpoint) — but noteEffort is the CLAUDE pill's only truth (effective effort appears in no hook, status line or init frame), so an adopted hub's effort pill does not move at all"},
		"claude.handoffBrief":      {false, "both POST claudemon /sessions/:id/handoff and return its markdown+path verbatim; the daemon composes the brief and chooses the filename on either side"},

		// THE AGENT-FACING FLEET VERBS (agentops.go, brief.go, visibleterm.go).
		// Every one of these is DEGRADED when the brain wins, and for one shared
		// reason worth stating once: each desktop implementation reaches into
		// claudeSessionStore, which is authoritative on the desktop and does not
		// exist here. The brain answers from its own projection of claudemon.
		"agents.reportProgress": {true, "same bounds (500 chars, 1/60s, 20 per session) and the same host-derived recipient, but the brain routes on the parent recorded by its OWN spawn metadata. A session the DESKTOP spawned has no entry there, so its worker's report is refused with 'not a tracked session' where main would have delivered it."},
		"agents.notifyWhen":     {true, "same one-shot predicate and the same 15s sweep, but evaluated over the brain's snapshot projection: the desktop's numbers come from its own accounting, the brain's from claudemon's status line. A watch armed on a session the brain has no row for is refused at arm time rather than firing later."},
		"agents.close":          {true, "same refusal for a WORKING session and the same daemon teardown, but 'forgotten' means removed from the BRAIN's store — the desktop's own row (and therefore its sidebar) is untouched, so an adopted desktop still shows the card it was asked to dismiss."},
		"agents.orphans":        {true, "the brain needs no tombstone store (claudemon keeps ended rows), but it can only see parents it recorded itself. A manager the DESKTOP spawned and that then died is not reported as an orphan candidate at all — the answer is narrower, not wrong."},
		"agents.reparent":       {true, "moves the parent link in the brain's spawn metadata, which is what the brain's own wakes route on. The desktop's claudeSessionStore is NOT updated, so in the adopted configuration a desktop-originated wake still goes to the retired manager."},
		"brief.append":          {false, "byte-for-byte the same additive insert (appendToBrief), the same O_EXCL lock and compare-and-swap, and the same assertPathAllowed(workspaceRoots) guard over `project` with the basename composed by the provider. The file on disk is the same file; nothing about which process writes it changes the result."},
		"brief.archive":         {false, "the same whole-line splice out of the same section into the same archive, under the same lock, the same compare-and-swap and the same archive-first ordering — and it is the one overlap held to its twin CASE BY CASE rather than by assertion: contracts/brief-board-cases.json runs both copies over the same briefs and compares both files' bytes."},
		"brief.check":           {true, "the READING is identical (same ported document model, same fixture), but the liveness source is not: main matches against claudeSessionStore, which holds federated peer rows and rows this desktop spawned, and the brain against its own projection of claudemon. A Now line naming a session only the DESKTOP knows about is reported stale under an adopted hub. It only ever REPORTS, so the cost is a false flag rather than a lost line — and where the brain cannot answer liveness at all it says so in `unavailableChecks` instead of narrowing silently."},
		"terminals.open":        {true, "main emits FACADE_OPEN_TERMINAL straight to ITS renderer; the brain has no renderer and publishes `facade.openTerminal` on the bus instead. A desktop that adopts a hub does not subscribe to that topic, so an agent's open_terminal reaches a WEB client and not the Electron window."},
		"fs.readImage":          {true, "same path guard and the same extension allowlist, but the brain has no image decoder: it inlines the original bytes under the twin's own MAX_INLINE_BYTES fallback rather than returning a downscaled thumbnail, and refuses an image over that cap where main would have resized it."},
		"sessions.recent":       {true, "same daemon list and the same merge, but the desktop's SQLite session-history join is unavailable headless: `model` and `costUSD` come back empty and `title` is never transcript-derived. The rows and their order are identical; three columns are blank."},
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
