package bus

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// TestScopedFSCallEnforcedEndToEnd proves the path scope is enforced on the real
// routing path: a plugin granted fs.read confined to one directory has an
// in-scope call routed to the provider, while an out-of-scope call is rejected by
// the bus and never reaches the provider.
func TestScopedFSCallEnforcedEndToEnd(t *testing.T) {
	root := t.TempDir()
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}

	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "test.plugin", []capspec.Grant{
		{Method: "fs.read", FSRoots: []string{canon}},
	}, capspec.EventGrants{})

	// Trusted provider answers fs.read. It should only ever see the in-scope call.
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"fs.read"}})
	provider.readUntil("registered")
	go func() {
		f := provider.readUntil("call")
		if f.ID != "" {
			provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()

	caller := dialClientToken(t, url, "plug-tok")

	// In scope → routed, returns the provider's result.
	caller.send(Frame{Op: "call", ID: "in", Method: "fs.read",
		Params: json.RawMessage(`{"path":` + jstr(filepath.Join(root, "a.txt")) + `}`)})
	if r := caller.readUntil("result"); r.ID != "in" {
		t.Fatalf("in-scope call: got id %q, want in", r.ID)
	}

	// Out of scope → bus rejects it; the provider never sees it.
	caller.send(Frame{Op: "call", ID: "out", Method: "fs.read",
		Params: json.RawMessage(`{"path":"/etc/passwd"}`)})
	e := caller.readUntil("error")
	if e.ID != "out" {
		t.Fatalf("out-of-scope call: got id %q, want out", e.ID)
	}
	if !strings.Contains(e.Error, "outside") {
		t.Fatalf("error = %q, want it to mention being outside scope", e.Error)
	}
}

// TestRegisterRefusesUnspeccedPathCapability proves the capspec allowlist guard:
// a grant whose method is named like a filesystem capability (fs.*/search.*) but
// has no capspec.PathParam entry is refused at registration — it would otherwise
// be admitted by authorize() with zero path confinement. A properly specced
// sibling in the same grant list is still admitted.
func TestRegisterRefusesUnspeccedPathCapability(t *testing.T) {
	_, srv := rpcServerWith(t)
	srv.RegisterPluginToken("plug-tok", "test.plugin", []capspec.Grant{
		{Method: "fs.read", FSRoots: []string{t.TempDir()}}, // specced → granted
		{Method: "fs.append", FSRoots: []string{"/"}},        // no spec → refused
		{Method: "search.everything"},                        // no spec → refused
	}, capspec.EventGrants{})

	pi, ok := srv.lookupPluginToken("plug-tok")
	if !ok {
		t.Fatal("token was not registered at all")
	}
	if _, ok := pi.caps["fs.read"]; !ok {
		t.Error("fs.read (specced) should have been granted")
	}
	if _, ok := pi.caps["fs.append"]; ok {
		t.Error("fs.append has no capspec entry and must NOT be granted unconfined")
	}
	if _, ok := pi.caps["search.everything"]; ok {
		t.Error("search.everything has no capspec entry and must NOT be granted unconfined")
	}
}
