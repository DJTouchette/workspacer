package plugin

import (
	"encoding/json"
	"path/filepath"
	"testing"
)

func TestCapabilityUnmarshal(t *testing.T) {
	// Bare string → verb only, no paths.
	var c Capability
	if err := json.Unmarshal([]byte(`"agents.list"`), &c); err != nil {
		t.Fatal(err)
	}
	if c.Method != "agents.list" || c.Paths != nil {
		t.Fatalf("string form = %+v, want {agents.list <nil>}", c)
	}

	// Object form → method + paths.
	var d Capability
	if err := json.Unmarshal([]byte(`{"method":"fs.read","paths":["${pluginDir}","/abs"]}`), &d); err != nil {
		t.Fatal(err)
	}
	if d.Method != "fs.read" || len(d.Paths) != 2 || d.Paths[0] != "${pluginDir}" || d.Paths[1] != "/abs" {
		t.Fatalf("object form = %+v", d)
	}

	// A whole manifest with mixed capability forms round-trips.
	var caps []Capability
	if err := json.Unmarshal([]byte(`["agents.list",{"method":"fs.write","paths":["${pluginDir}"]}]`), &caps); err != nil {
		t.Fatal(err)
	}
	if len(caps) != 2 || caps[0].Method != "agents.list" || caps[1].Method != "fs.write" {
		t.Fatalf("mixed list = %+v", caps)
	}
}

func TestValidate_RejectsUnscopedFilesystemCapability(t *testing.T) {
	m := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "fs.read"}}}
	if err := m.Validate(); err == nil {
		t.Fatal("expected unscoped fs.read to be rejected")
	}

	// With paths declared it's fine.
	ok := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "fs.read", Paths: []string{"${pluginDir}"}}}}
	if err := ok.Validate(); err != nil {
		t.Fatalf("scoped fs.read should validate, got %v", err)
	}

	// A non-path capability needs no paths.
	verb := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: "agents.list"}}}
	if err := verb.Validate(); err != nil {
		t.Fatalf("verb-only capability should validate, got %v", err)
	}

	// Empty method is rejected.
	empty := Manifest{ID: "x", APIVersion: APIVersion, Capabilities: []Capability{{Method: ""}}}
	if err := empty.Validate(); err == nil {
		t.Fatal("expected empty capability method to be rejected")
	}
}

func TestExpandScope(t *testing.T) {
	dir := filepath.FromSlash("/plugins/acme")
	cwd := filepath.FromSlash("/work/project")
	bindings := map[string]string{"pluginDir": dir, "agentCwd": cwd}
	cases := []struct {
		in   string
		want string
	}{
		{"${pluginDir}", dir},
		{"${pluginDir}/data", filepath.Join(dir, "data")},
		{"${agentCwd}", cwd},                           // bound dynamic scope resolves
		{"${agentCwd}/src", filepath.Join(cwd, "src")}, // …with a subpath
		{filepath.FromSlash("/abs/path"), filepath.FromSlash("/abs/path")},
		{"relative/path", ""}, // relative → dropped
		{"${unknown}/x", ""},  // no such binding → dropped
		{"${malformed", ""},   // no closing brace → dropped
	}
	for _, c := range cases {
		if got := expandScope(c.in, bindings); got != c.want {
			t.Errorf("expandScope(%q) = %q, want %q", c.in, got, c.want)
		}
	}
	// With no agentCwd binding (the static load-time case), ${agentCwd} grants nothing.
	if got := expandScope("${agentCwd}", map[string]string{"pluginDir": dir}); got != "" {
		t.Errorf("unbound ${agentCwd} = %q, want \"\"", got)
	}
}

func TestGrantsFor(t *testing.T) {
	dir := filepath.FromSlash("/plugins/acme")
	mf := Manifest{
		Dir: dir,
		Capabilities: []Capability{
			{Method: "agents.list"},
			{Method: "fs.read", Paths: []string{"${pluginDir}", "relative-dropped"}},
			{Method: ""}, // skipped
		},
	}
	grants := grantsFor(mf)
	if len(grants) != 2 {
		t.Fatalf("got %d grants, want 2 (empty-method skipped): %+v", len(grants), grants)
	}
	// agents.list → verb only, no roots.
	if grants[0].Method != "agents.list" || len(grants[0].FSRoots) != 0 {
		t.Errorf("grant[0] = %+v, want agents.list with no roots", grants[0])
	}
	// fs.read → only the resolvable root survives (relative dropped).
	if grants[1].Method != "fs.read" || len(grants[1].FSRoots) != 1 || grants[1].FSRoots[0] != dir {
		t.Errorf("grant[1] = %+v, want fs.read scoped to %q", grants[1], dir)
	}
}
