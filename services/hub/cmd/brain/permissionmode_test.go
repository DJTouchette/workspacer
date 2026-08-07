package main

import (
	"regexp"
	"strings"
	"testing"
)

// The four spellings of "stop asking" that were live when this was written, plus
// the shape of the next one. All four must be refused for a bus caller; the
// point of an ALLOWLIST is that the fifth is refused before anyone has heard of
// it, which is what `made-up-mode` stands in for.
func TestBypassModesAreEscalationsAndOrdinaryModesAreNot(t *testing.T) {
	for _, mode := range []string{"bypassPermissions", "yolo", "dontAsk", "auto", "made-up-mode"} {
		if !isPermissionEscalation(mode) {
			t.Errorf("isPermissionEscalation(%q) = false — a bus caller could start (or switch into) an agent that auto-approves every tool call", mode)
		}
	}
	// The floor. An allowlist that refused everything would satisfy the loop
	// above and break every legitimate remote mode change; "" is the caller
	// asking for the provider default, not an escalation.
	for _, mode := range []string{"", "default", "ask", "acceptEdits", "plan", "manual"} {
		if isPermissionEscalation(mode) {
			t.Errorf("isPermissionEscalation(%q) = true — this mode only ever ADDS friction and the remote pill needs it", mode)
		}
	}
}

// tsPermissionModeSet is the desktop's copy of the same allowlist, read out of
// lib/permissionBypass.ts. The two providers answer the same bus surface and one
// of them (the desktop) owns the door the other does not register at all
// (claude.setPermissionMode), so the vocabularies have to be one vocabulary —
// exactly the argument for TestGitNoExecKeysMatchTheDesktopTwin above.
var tsPermissionModeEntry = regexp.MustCompile(`^'([A-Za-z]+)',$`)

func TestPermissionModeAllowlistMatchesTheDesktop(t *testing.T) {
	// A missing twin FAILS; only an absent checkout skips (mustReadRepoFile).
	src := mustReadRepoFile(t, "apps", "desktop", "src", "main", "lib", "permissionBypass.ts")
	text := string(src)
	start := strings.Index(text, "BUS_SETTABLE_PERMISSION_MODES: ReadonlySet<string> = new Set([")
	if start < 0 {
		t.Fatal("could not find BUS_SETTABLE_PERMISSION_MODES in permissionBypass.ts — the declaration was renamed or reshaped, and this parity test is now comparing nothing")
	}
	end := strings.Index(text[start:], "]);")
	if end < 0 {
		t.Fatal("BUS_SETTABLE_PERMISSION_MODES is not closed by `]);` — the parse cannot delimit the set")
	}
	ts := map[string]bool{}
	for _, line := range strings.Split(text[start:start+end], "\n") {
		if m := tsPermissionModeEntry.FindStringSubmatch(strings.TrimSpace(line)); m != nil {
			ts[m[1]] = true
		}
	}
	if len(ts) == 0 {
		t.Fatal("parsed zero modes out of permissionBypass.ts — this parity test has stopped comparing anything")
	}
	for mode := range busSettablePermissionModes {
		if !ts[mode] {
			t.Errorf("the brain lets a bus caller ask for %q and the desktop twin does not — one stack refuses a mode the other applies", mode)
		}
		delete(ts, mode)
	}
	for _, mode := range sortedKeys(ts) {
		t.Errorf("permissionBypass.ts lets a bus caller ask for %q and the brain does not", mode)
	}
}
