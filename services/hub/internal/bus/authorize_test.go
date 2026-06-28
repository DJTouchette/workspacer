package bus

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
)

// authorize is the per-call argument gate. These exercise it directly on a conn
// (no socket needed), since it's pure given the conn's grants.

func TestAuthorize_TrustedBypassesScope(t *testing.T) {
	cn := &conn{trusted: true}
	// A trusted host conn may read anywhere, with no grants at all.
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":"/etc/passwd"}`)); err != nil {
		t.Fatalf("trusted conn should bypass scope, got %v", err)
	}
}

func TestAuthorize_NonPathCapabilityPasses(t *testing.T) {
	cn := &conn{caps: map[string]capGrant{"agents.list": {}}}
	if err := cn.authorize("agents.list", json.RawMessage(`{}`)); err != nil {
		t.Fatalf("verb-only capability should pass authorize, got %v", err)
	}
}

func TestAuthorize_PathScoped(t *testing.T) {
	root := t.TempDir()
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}
	cn := &conn{caps: map[string]capGrant{
		"fs.read":  {fsRoots: []string{canon}},
		"fs.write": {fsRoots: []string{canon}},
	}}

	// Inside the granted root → allowed.
	inside := json.RawMessage(`{"path":` + jstr(filepath.Join(root, "a.txt")) + `}`)
	if err := cn.authorize("fs.read", inside); err != nil {
		t.Errorf("read inside root should be allowed, got %v", err)
	}

	// Outside the root → denied.
	outside := json.RawMessage(`{"path":"/etc/passwd"}`)
	if err := cn.authorize("fs.read", outside); err == nil {
		t.Errorf("read outside root should be denied")
	} else if !strings.Contains(err.Error(), "outside") {
		t.Errorf("error = %q, want it to mention being outside scope", err)
	}

	// Traversal that escapes the root → denied.
	esc := json.RawMessage(`{"path":` + jstr(filepath.Join(root, "..", "x")) + `}`)
	if err := cn.authorize("fs.write", esc); err == nil {
		t.Errorf("traversal write should be denied")
	}

	// Missing the path field → denied (can't verify → fail closed).
	if err := cn.authorize("fs.read", json.RawMessage(`{}`)); err == nil {
		t.Errorf("missing path should be denied")
	}
}

func TestAuthorize_PathScopedWithNoRootsDenied(t *testing.T) {
	// A path-scoped capability somehow granted with no roots must not become an
	// unrestricted grant — fail closed.
	cn := &conn{caps: map[string]capGrant{"fs.read": {}}}
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":"/tmp/x"}`)); err == nil {
		t.Errorf("path-scoped capability with no roots should be denied")
	}
}

func TestAuthorize_SearchProjectUsesCwdField(t *testing.T) {
	root := t.TempDir()
	canon, _ := canonicalize(root)
	cn := &conn{caps: map[string]capGrant{"search.project": {fsRoots: []string{canon}}}}

	ok := json.RawMessage(`{"cwd":` + jstr(root) + `,"query":"foo"}`)
	if err := cn.authorize("search.project", ok); err != nil {
		t.Errorf("search within root should be allowed, got %v", err)
	}
	bad := json.RawMessage(`{"cwd":"/var","query":"foo"}`)
	if err := cn.authorize("search.project", bad); err == nil {
		t.Errorf("search outside root should be denied")
	}
}

// jstr JSON-quotes a path so embedded separators/backslashes (Windows) stay valid.
func jstr(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}
