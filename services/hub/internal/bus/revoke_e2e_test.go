package bus

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// trySend / tryReadUntil are the non-fatal twins of the shared client helpers.
// Revocation may close the socket, and a closed socket is a PASS here, so a
// write or read failure must not be a t.Fatal.
func (c *client) trySend(f Frame) bool {
	data, _ := json.Marshal(f)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	return c.ws.Write(ctx, websocket.MessageText, data) == nil
}

func (c *client) tryReadUntil(a, b string, d time.Duration) (Frame, bool) {
	ctx, cancel := context.WithTimeout(context.Background(), d)
	defer cancel()
	for {
		_, data, err := c.ws.Read(ctx)
		if err != nil {
			return Frame{}, false
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			return Frame{}, false
		}
		if f.Op == a || f.Op == b {
			return f, true
		}
	}
}

// dialRejected reports whether the server refuses a fresh /bus dial with token.
func dialRejected(t *testing.T, httpURL, token string) bool {
	t.Helper()
	wsURL := strings.Replace(httpURL, "http://", "ws://", 1) + "/bus?token=" + token
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		return true
	}
	c.CloseNow()
	return false
}

// Revoking a plugin/pane bus token has to affect the connection that already
// holds it.
//
// conn.caps is a snapshot taken once at handshake, and UnregisterPluginToken
// only deleted the entry that governs the NEXT handshake, so revocation was a
// no-op on any socket that was already open. Nothing in the tree noticed:
// internal/plugin/panetoken_test.go asserts the MANAGER calls the registrar, and
// the whole body of UnregisterPluginToken could be replaced with `_ = token`
// with every package green.
//
// The consequence is not theoretical. A pane token is the ONLY way a plugin ever
// gets ${agentCwd} filesystem roots — grantsFor gives the static token none,
// because "dynamic scopes like ${agentCwd} aren't bound here" — so a plugin that
// keeps one pane socket open kept fs.read/fs.write inside that agent's working
// tree after the pane closed, after the plugin was disabled, and after it was
// uninstalled. Manager.PaneToken calls exactly that state "an unrevocable grant
// leak".
func TestRevokedPluginTokenLosesItsReachOnALiveConnection(t *testing.T) {
	root := t.TempDir()
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}

	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("pane-tok", "test.plugin", []capspec.Grant{
		{Method: "fs.read", FSRoots: []string{canon}},
	}, capspec.EventGrants{})

	// Trusted provider answers fs.read for as long as anything reaches it.
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"fs.read"}})
	provider.readUntil("registered")
	// The provider counts calls it is REACHED by. The frame's ID is the hub's own
	// outbound id, not the caller's, so identity is the count, not the string.
	answered := make(chan struct{}, 4)
	go func() {
		for {
			f, ok := provider.tryReadUntil("call", "call", 5*time.Second)
			if !ok || f.ID == "" {
				return
			}
			answered <- struct{}{}
			if !provider.trySend(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)}) {
				return
			}
		}
	}()

	caller := dialClientToken(t, url, "pane-tok")
	target := json.RawMessage(`{"path":` + jstr(filepath.Join(root, "a.txt")) + `}`)

	// Floor: before revocation the same socket, the same call, is answered.
	caller.send(Frame{Op: "call", ID: "before", Method: "fs.read", Params: target})
	if r := caller.readUntil("result"); r.ID != "before" {
		t.Fatalf("pre-revocation call: got id %q, want before", r.ID)
	}
	select {
	case <-answered:
	case <-time.After(2 * time.Second):
		t.Fatal("provider never saw the pre-revocation call")
	}

	// The pane closes → Manager.revokePaneTokensFor → this.
	srv.UnregisterPluginToken("pane-tok")

	// The SAME socket must no longer be able to call. Either answer is
	// acceptable — a denial frame or a closed connection — but "result" is not.
	if !caller.trySend(Frame{Op: "call", ID: "after", Method: "fs.read", Params: target}) {
		return // the socket is already gone — revocation closed it, which is the point
	}
	f, ok := caller.tryReadUntil("result", "error", 2*time.Second)
	if ok && f.Op == "result" {
		t.Fatalf("REVOCATION IS A NO-OP: fs.read was answered on the live socket after UnregisterPluginToken (frame=%+v)", f)
	}
	if ok && f.Op == "error" && !strings.Contains(f.Error, "not authorized") {
		t.Fatalf("post-revocation error = %q, want a not-authorized denial", f.Error)
	}
	select {
	case <-answered:
		t.Fatal("the provider was reached by a revoked token's call")
	default:
	}
}

// The dial side of the same property, and the half that already worked: a
// revoked token cannot open a NEW connection either.
func TestRevokedPluginTokenCannotReconnect(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("pane-tok", "test.plugin", []capspec.Grant{
		{Method: "fs.read", FSRoots: []string{t.TempDir()}},
	}, capspec.EventGrants{})
	srv.UnregisterPluginToken("pane-tok")

	if _, ok := srv.lookupPluginToken("pane-tok"); ok {
		t.Fatal("UnregisterPluginToken left the token resolvable — a revoked plugin reconnects with its old grants")
	}
	if !dialRejected(t, url, "pane-tok") {
		t.Fatal("a revoked token was accepted on a fresh dial")
	}
}
