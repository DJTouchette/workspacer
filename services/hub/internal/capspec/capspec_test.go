package capspec

import (
	"os"
	"path/filepath"
	"regexp"
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
		// Under a filesystem namespace but NOT in PathParam → the drift we guard:
		// looks path-bearing and is missing its spec.
		{"fs.append", true, true},
		{"fs.copy", true, true},
		{"search.files", true, true},
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
