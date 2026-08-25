package nodes

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func write(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "nodes.json")
	if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return p
}

// The ordinary desktop install has no nodes.json, and that is not an error —
// it is how this whole subsystem stays dormant on a machine with no remote
// node.
func TestAMissingRegistryIsNoNodes(t *testing.T) {
	got, err := LoadFile(filepath.Join(t.TempDir(), "absent.json"))
	if err != nil || len(got) != 0 {
		t.Fatalf("LoadFile(missing) = %v, %v; want no nodes and no error", got, err)
	}
}

// A corrupt registry fails LOUDLY, following peers.json's reasoning: a typo
// that silently disables the registry reads to the user as "my remote machine
// vanished", which is the most expensive way to learn about a missing comma.
func TestACorruptRegistryIsAnError(t *testing.T) {
	if _, err := LoadFile(write(t, `[{"id": "den",]`)); err == nil {
		t.Fatal("a corrupt nodes.json loaded without complaint")
	}
}

func TestLoadFileParsesAFullEntry(t *testing.T) {
	p := write(t, `[{"id":"den","label":"Fly node (den)","fly":{"app":"wks-node","machineId":"17811944b12345","token":"tok"}}]`)
	got, err := LoadFile(p)
	if err != nil {
		t.Fatalf("LoadFile: %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("got %d nodes, want 1", len(got))
	}
	n := got[0]
	if n.ID != "den" || n.Label != "Fly node (den)" || !n.Wakeable() {
		t.Fatalf("parsed %+v", n)
	}
	tok, err := ResolveToken(n.Fly)
	if err != nil || tok != "tok" {
		t.Fatalf("ResolveToken = %q, %v", tok, err)
	}
}

func TestLoadFileRefusesBadEntries(t *testing.T) {
	cases := map[string]string{
		"no id":          `[{"label":"x"}]`,
		"bad id":         `[{"id":"den/../etc"}]`,
		"duplicate id":   `[{"id":"den"},{"id":"den"}]`,
		"app without id": `[{"id":"den","fly":{"app":"wks-node"}}]`,
		"id without app": `[{"id":"den","fly":{"machineId":"m1"}}]`,
	}
	for name, body := range cases {
		t.Run(name, func(t *testing.T) {
			if _, err := LoadFile(write(t, body)); err == nil {
				t.Fatalf("%s loaded without complaint", name)
			}
		})
	}
}

// A node with no fly block at all is legal — it is a machine somebody else
// powers on, which the hub can observe but not wake.
func TestANodeWithNoCloudBlockLoads(t *testing.T) {
	got, err := LoadFile(write(t, `[{"id":"laptop","label":"The laptop"}]`))
	if err != nil {
		t.Fatalf("LoadFile: %v", err)
	}
	if got[0].Wakeable() {
		t.Error("a node with no fly block reported itself wakeable")
	}
}

func TestResolveTokenPrefersInlineThenFileThenEnv(t *testing.T) {
	dir := t.TempDir()
	tf := filepath.Join(dir, "tok")
	if err := os.WriteFile(tf, []byte("  from-file\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv(FlyTokenEnv, "from-env")

	if got, _ := ResolveToken(&Fly{Token: "inline", TokenFile: tf}); got != "inline" {
		t.Errorf("inline token = %q", got)
	}
	if got, _ := ResolveToken(&Fly{TokenFile: tf}); got != "from-file" {
		t.Errorf("file token = %q (leading/trailing whitespace must be trimmed)", got)
	}
	if got, _ := ResolveToken(&Fly{}); got != "from-env" {
		t.Errorf("env token = %q", got)
	}
	if _, err := ResolveToken(&Fly{TokenFile: filepath.Join(dir, "nope")}); err == nil {
		t.Error("a missing tokenFile resolved without complaint — that would silently disable waking")
	}
}

// The registry holds a credential that spends money. A caller must be able to
// notice it is world-readable.
func TestFileLooksExposedNoticesLoosePermissions(t *testing.T) {
	p := write(t, `[]`)
	if FileLooksExposed(p) {
		t.Fatal("a 0600 file was reported exposed")
	}
	if err := os.Chmod(p, 0o644); err != nil {
		t.Fatal(err)
	}
	if !FileLooksExposed(p) {
		t.Fatal("a 0644 file holding a Fly token was not reported exposed")
	}
}

// DefaultPath sits beside peers.json and tokens.json, and is nodes.json.
func TestDefaultPathIsNodesJSONInTheConfigDir(t *testing.T) {
	if base := filepath.Base(DefaultPath()); base != "nodes.json" {
		t.Errorf("DefaultPath basename = %q, want nodes.json", base)
	}
	if !strings.Contains(DefaultPath(), "workspacer") {
		t.Errorf("DefaultPath = %q, want it under the workspacer config dir", DefaultPath())
	}
}
