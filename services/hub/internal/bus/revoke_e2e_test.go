package bus

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
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

// The RACE, which is what makes revocation enforcing rather than nearly-always.
//
// The handshake resolves the presented token BEFORE websocket.Accept and only
// registers the connection for revocation afterwards. UnregisterPluginToken
// snapshots pluginConns under ptMu and closes exactly what it finds — so a dial
// whose lookup ran before the delete and whose track runs after it is in NEITHER
// set. It is never closed, never flagged, and conn.caps is a snapshot taken at
// accept, so it keeps its full ${agentCwd} grants for the life of the process.
//
// Measured on the unfixed tree with 8 concurrent dials per round: 3/3 runs
// leaked, in 2, 5 and 10 rounds, and a leaked socket went on answering fs.read
// for as long as it was asked to. A plugin sidecar holding its .bus-token and
// reconnecting in a loop wins this trivially, so the grant survives disable,
// reload and uninstall.
//
// The fix serializes the two under the same mutex: trackPluginConn re-checks
// that the token is still registered and the handshake revokes on the spot when
// it is not, so either the delete lands first (track returns false) or the track
// lands first (the delete finds the connection).
func TestRevocationCannotRaceAnInFlightHandshake(t *testing.T) {
	root := t.TempDir()
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "a.txt"), []byte("secret"), 0o600); err != nil {
		t.Fatal(err)
	}
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")

	// A trusted provider that answers every fs.read it is reached by. Reaching it
	// AT ALL after revocation is the leak.
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"fs.read"}})
	provider.readUntil("registered")
	served := make(chan struct{}, 256)
	go func() {
		for {
			f, ok := provider.tryReadUntil("call", "call", 30*time.Second)
			if !ok || f.ID == "" {
				return
			}
			served <- struct{}{}
			if !provider.trySend(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)}) {
				return
			}
		}
	}()

	target := json.RawMessage(`{"path":` + jstr(filepath.Join(root, "a.txt")) + `}`)
	wsURL := strings.Replace(url, "http://", "ws://", 1) + "/bus?token=pane-tok"

	// The DETERMINISTIC half. The window is lookup -> Accept -> track, and
	// whether a goroutine lands inside it is up to the scheduler, so the loop
	// below is a probability, not a proof. This is the proof: it plays the losing
	// interleaving by hand — the token is gone by the time the connection asks to
	// be tracked — and trackPluginConn has to say so. Both operations take ptMu,
	// so there is no third ordering to test.
	srv.RegisterPluginToken("pane-tok", "test.plugin", []capspec.Grant{
		{Method: "fs.read", FSRoots: []string{canon}},
	}, capspec.EventGrants{})
	late := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{canon}}}}
	if !srv.trackPluginConn("pane-tok", late) {
		t.Fatal("floor: tracking a LIVE token must succeed, or the check below proves nothing")
	}
	srv.untrackPluginConn("pane-tok", late)
	srv.UnregisterPluginToken("pane-tok")
	if srv.trackPluginConn("pane-tok", late) {
		t.Fatal("trackPluginConn accepted a connection for a token UnregisterPluginToken had already dropped — that connection is in neither set: never closed, never flagged, and conn.caps is a snapshot, so it keeps its ${agentCwd} grants for the life of the process")
	}

	for round := 0; round < 12; round++ {
		srv.RegisterPluginToken("pane-tok", "test.plugin", []capspec.Grant{
			{Method: "fs.read", FSRoots: []string{canon}},
		}, capspec.EventGrants{})

		const dials = 8
		var wg sync.WaitGroup
		conns := make(chan *websocket.Conn, dials)
		start := make(chan struct{})
		for i := 0; i < dials; i++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
				defer cancel()
				c, _, err := websocket.Dial(ctx, wsURL, nil)
				if err == nil {
					conns <- c
				}
			}()
		}
		close(start)
		// Revoke WHILE the dials are in flight. No sleep: the point is to land
		// inside the lookup→accept→track window, and the scheduler decides.
		srv.UnregisterPluginToken("pane-tok")
		wg.Wait()
		close(conns)

		// Drain anything the provider was reached by before revocation; only
		// post-revocation reachability counts.
		for {
			select {
			case <-served:
				continue
			default:
			}
			break
		}

		leaked := 0
		for c := range conns {
			cl := &client{ws: c}
			if !cl.trySend(Frame{Op: "call", ID: "after", Method: "fs.read", Params: target}) {
				c.CloseNow()
				continue // socket already gone: revocation closed it, which is the point
			}
			f, ok := cl.tryReadUntil("result", "error", 1500*time.Millisecond)
			if ok && f.Op == "result" {
				leaked++
			}
			c.CloseNow()
		}
		if leaked > 0 {
			t.Fatalf("REVOCATION RACE: after %d rounds, %d socket(s) were served fs.read on a token UnregisterPluginToken had already dropped — they were accepted after the lookup and tracked after the delete, so neither set contained them", round+1, leaked)
		}
		select {
		case <-served:
			t.Fatalf("round %d: the provider was reached by a revoked token's call", round+1)
		default:
		}
	}
}

// The `revoked` flag on its own. UnregisterPluginToken's own comment says both
// halves are needed — "revoked is what makes the NEXT call on an in-flight
// connection fail even before the close lands, and CloseNow is what stops the
// connection from sitting there consuming events" — and only the CloseNow half
// was pinned: deleting the `if cn.revoked.Load() { return false }` branch from
// mayCall left the whole tree green, 20 runs of the e2e test above included,
// because that test's `if !caller.trySend(...) { return }` takes the
// early-return branch every time (CloseNow is synchronous). This asserts the
// flag directly, with no socket in the way.
func TestARevokedConnectionMayCallNothing(t *testing.T) {
	root := t.TempDir()
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}
	cn := &conn{
		caps: map[string]capGrant{"fs.read": {fsRoots: []string{canon}}},
	}
	if !cn.mayCall("fs.read") {
		t.Fatal("floor: a live connection with the grant must be allowed to call it")
	}
	cn.revoked.Store(true)
	if cn.mayCall("fs.read") {
		t.Fatal("a REVOKED connection was still allowed to call fs.read — the flag is the half that denies a call already read off the wire, before the close lands")
	}
	// Revocation outranks TRUSTED too. A trusted conn never carries a plugin
	// token today, so this arm is fail-closed by construction rather than
	// reachable — but the ordering is the claim ("a revoked credential authorizes
	// nothing, whatever it used to authorize"), and moving the check below the
	// trusted short-circuit is the obvious refactor that would silently undo it.
	cn2 := &conn{trusted: true}
	if !cn2.mayCall("fs.read") {
		t.Fatal("floor: a trusted connection may call anything")
	}
	cn2.revoked.Store(true)
	if cn2.mayCall("fs.read") {
		t.Fatal("the revoked check is below the trusted short-circuit — revocation must come first")
	}
}
