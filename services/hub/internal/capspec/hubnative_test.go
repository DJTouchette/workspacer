package capspec

import (
	"regexp"
	"sort"
	"testing"
)

// hubMainSrc is the third registry. Named once, next to desktopCapabilitiesSrc.
var hubMainSrc = []string{"services", "hub", "cmd", "hub", "main.go"}

// hubLocalCapRe matches cmd/hub's own capability registrations. BOTH doors:
// RegisterLocal for a handler that does not need the caller, RegisterLocalIdent
// for one that does — and layout.set moved from the first to the second the day
// it learned to scrub a non-trusted write, so a regex that matched only one form
// would have lost the very method that motivated this file.
//
// The same defect the CAP_LABELS guard had: its regex matched `registerCapability(`
// and not the delegation-aware alias `cat(`, so a third of the desktop surface
// sat outside a list whose doc comment claimed to cover "every capability the
// main process actually registers".
var hubLocalCapRe = regexp.MustCompile(`RegisterLocal(?:Ident)?\(\s*"([a-zA-Z][\w.]*)"`)

// hubNativeFloor is the size of cmd/hub's own capability surface today, and it
// is a RATCHET rather than a comment.
//
// Every guard in this family keeps re-learning the same lesson: a scan that
// finds nothing asserts nothing and reports PASS. Rename RegisterLocal, move the
// registrations into a helper, or split main.go, and a `for range matches` loop
// runs zero times with the whole module green — which is exactly the state
// layout.set shipped in, since NO scan looked at this file at all.
//
// Raise it when the surface grows. Lowering it is the change that has to be
// argued for.
//
// 22 as of routing.select, which is the twenty-second name cmd/hub registers.
const hubNativeFloor = 22

// TestHubNativeCapabilitiesAllClassified is the METHOD-level completeness check
// for the registry nobody was reading.
//
// There are THREE capability registries in this repo, not two:
//
//	apps/desktop/src/main/services/hubCapabilities.ts   registerCapability + cat
//	services/hub/cmd/brain/handlers.go                  methods + catalogMethods
//	services/hub/cmd/hub/main.go                        RegisterLocal(Ident)
//
// TestDesktopCapabilitiesAllClassified and TestBrainMethodsAllClassified cover
// the first two and, between them, the 73 methods a provider answers. The third
// was covered by nothing: its seven methods are in neither provider's list, so
// neither guard enumerated them; none carries an fs./search./library./git./
// providers. prefix, so MissingSpec was false for all seven; Classified was
// therefore false, which means RegisterPluginToken did NOT refuse them — a
// plugin manifest declaring `layout.set` was simply granted it — and no
// CAP_LABELS row warned a user consenting to that manifest either.
//
// layout.set is what that silence cost. The hub stores its `data` verbatim
// because "the hub does not interpret this document"; the desktop then adopts
// that document on its next launch and respawns every agent in it through the
// LOCAL IPC spawn door, which does no scrubbing. skipPermissions,
// permissionMode, profileId and mcpItemIds — the four things the bus's own
// agents.spawn refuses from a bus caller — arrived at spawnClaude verbatim, from
// a caller that may not spawn at all.
func TestHubNativeCapabilitiesAllClassified(t *testing.T) {
	data := mustReadRepoFile(t, hubMainSrc...)
	matches := hubLocalCapRe.FindAllStringSubmatch(string(data), -1)

	seen := map[string]bool{}
	for _, m := range matches {
		seen[m[1]] = true
	}
	if len(seen) < hubNativeFloor {
		t.Fatalf("parsed only %d hub-native capability names from cmd/hub/main.go (floor %d) — the registration syntax changed, or the registrations moved, and this guard is guarding nothing. That is the state layout.set shipped in: no scan read this file at all.", len(seen), hubNativeFloor)
	}

	names := make([]string, 0, len(seen))
	for n := range seen {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, name := range names {
		if MissingClassification(name) {
			t.Errorf("cmd/hub registers %q and capspec says nothing about it — not a PathParam entry, not an unscopedByDecision reason, not an inertMethods reason. With Classified false the bus does not refuse it to a plugin token and no CAP_LABELS row warns about it. Decide what the caller may supply and where it goes, then write it down.", name)
		}
	}

	// The registrations this file is ABOUT must be among what was parsed, or a
	// refactor that moved layout.* elsewhere would keep the floor satisfied with
	// seven push.* rows and quietly stop covering the method that motivated the
	// guard.
	for _, must := range []string{"layout.get", "layout.set"} {
		if !seen[must] {
			t.Errorf("cmd/hub/main.go no longer registers %q by that name — the guard's floor is met by other methods while the one it exists for went unscanned", must)
		}
	}
}

// TestHubNativeAndProviderRegistriesDoNotOverlap keeps the third registry a
// third registry. A method served by BOTH the hub itself and a provider has two
// implementations and one capspec entry, and the entry can only describe one of
// them — which is how a "the provider confines it" excuse ends up covering a
// door the provider never sees.
func TestHubNativeAndProviderRegistriesDoNotOverlap(t *testing.T) {
	hub := mustReadRepoFile(t, hubMainSrc...)
	desktop := mustReadRepoFile(t, desktopCapabilitiesSrc...)

	hubNames := map[string]bool{}
	for _, m := range hubLocalCapRe.FindAllStringSubmatch(string(hub), -1) {
		hubNames[m[1]] = true
	}
	if len(hubNames) < hubNativeFloor {
		t.Fatalf("parsed only %d hub-native names (floor %d) — see TestHubNativeCapabilitiesAllClassified", len(hubNames), hubNativeFloor)
	}
	desktopMatches := capNameRe.FindAllStringSubmatch(string(desktop), -1)
	if len(desktopMatches) < 60 {
		t.Fatalf("parsed only %d desktop capability names — the registration syntax changed", len(desktopMatches))
	}
	for _, m := range desktopMatches {
		if hubNames[m[1]] {
			t.Errorf("%q is registered by BOTH cmd/hub and hubCapabilities.ts. Two implementations answer one name depending on which provider connected, and capspec has one entry for it — whichever door the entry describes, the other one ships unexamined.", m[1])
		}
	}
}
