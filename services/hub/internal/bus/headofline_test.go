package bus

import (
	"encoding/json"
	"strconv"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// providerConn returns the hub-side connection registered for method. In-package
// access is the point: holding its write mutex is how these tests simulate the
// one condition that matters here — a peer whose socket is not draining.
func providerConn(t *testing.T, rt *router, method string) *conn {
	t.Helper()
	rt.mu.Lock()
	defer rt.mu.Unlock()
	id, ok := rt.providers[method]
	if !ok {
		t.Fatalf("no provider registered for %q", method)
	}
	cn := rt.conns[id]
	if cn == nil {
		t.Fatalf("provider conn for %q is gone", method)
	}
	return cn
}

// pluginConn returns the hub-side connection belonging to a plugin id.
func pluginConn(t *testing.T, rt *router, pluginID string) *conn {
	t.Helper()
	rt.mu.Lock()
	defer rt.mu.Unlock()
	for _, cn := range rt.conns {
		if cn.pluginID == pluginID {
			return cn
		}
	}
	t.Fatalf("no connection for plugin %q", pluginID)
	return nil
}

// Forwarding a call writes to a DIFFERENT connection, so doing it inline on the
// caller's read loop lets one unresponsive provider head-of-line block
// everything else that caller sends — its next frame waits on a socket it has
// nothing to do with.
func TestSlowProviderDoesNotStallTheCallersOtherFrames(t *testing.T) {
	url, srv := rpcServerWith(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"slow.method"}})
	provider.readUntil("registered")

	// The provider's socket stops draining mid-write.
	stuck := providerConn(t, srv.router, "slow.method")
	stuck.writeMu.Lock()
	defer stuck.writeMu.Unlock()

	caller := dialClient(t, url)
	caller.send(Frame{Op: "call", ID: "blocked", Method: "slow.method"})

	// The call is parked against the wedged provider; everything else this
	// caller sends must still be answered.
	caller.send(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	if got := caller.readUntil("subscribed").Topics; len(got) != 1 {
		t.Fatalf("subscribed topics = %v — the caller's read loop was stuck forwarding to the wedged provider", got)
	}
}

// Getting the forward off the read loop must not cost the ordering the read
// loop used to give for free. One caller's successive calls have to reach the
// provider in the order it sent them: sessions.terminalInput is fired per
// keystroke/chunk by the web client without awaiting the result, so an inversion
// here is text arriving scrambled in a PTY. Under a goroutine-per-forward this
// failed reliably, not occasionally.
func TestOneCallersCallsReachTheProviderInSendOrder(t *testing.T) {
	url := rpcServer(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"sessions.terminalInput"}})
	provider.readUntil("registered")

	const n = 40
	caller := dialClient(t, url)
	for i := 0; i < n; i++ {
		caller.send(Frame{
			Op: "call", ID: "c" + strconv.Itoa(i), Method: "sessions.terminalInput",
			Params: json.RawMessage(`{"seq":` + strconv.Itoa(i) + `}`),
		})
	}

	got := make([]int, 0, n)
	for i := 0; i < n; i++ {
		f := provider.readUntil("call")
		var p struct {
			Seq int `json:"seq"`
		}
		if err := json.Unmarshal(f.Params, &p); err != nil {
			t.Fatalf("call %d params %s: %v", i, f.Params, err)
		}
		got = append(got, p.Seq)
	}
	for i, seq := range got {
		if seq != i {
			t.Fatalf("provider received %v — one caller's calls were reordered on the way to the provider", got)
		}
	}
}

// Ordering means queueing, and a queue against a peer that never drains has to
// end somewhere. Past the depth the call is refused with an error the caller can
// see, rather than buffered until the hub runs out of memory — and the caller's
// read loop keeps answering throughout.
func TestCallsBacklogAgainstAWedgedProviderIsRefusedNotBuffered(t *testing.T) {
	url, srv := rpcServerWith(t)
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"wedged.method"}})
	provider.readUntil("registered")

	stuck := providerConn(t, srv.router, "wedged.method")
	stuck.writeMu.Lock()
	defer stuck.writeMu.Unlock()

	caller := dialClient(t, url)
	for i := 0; i < forwardQueueDepth+8; i++ {
		caller.send(Frame{Op: "call", ID: "q" + strconv.Itoa(i), Method: "wedged.method"})
	}
	e := caller.readUntil("error")
	if !strings.Contains(e.Error, "too many calls queued") {
		t.Fatalf("error = %q, want the queue-full refusal", e.Error)
	}

	caller.send(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	if got := caller.readUntil("subscribed").Topics; len(got) != 1 {
		t.Fatalf("subscribed topics = %v — the caller's read loop stalled behind its own backlog", got)
	}
}

// The reply side of the same seam: a provider's read loop writes results to
// other connections, so a slow caller must not stall the results this provider
// owes everyone else.
func TestSlowCallerDoesNotStallAProvidersOtherReplies(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-slow", "slow.plugin", []capspec.Grant{{Method: "echo.method"}}, capspec.EventGrants{})

	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"echo.method"}})
	provider.readUntil("registered")

	slow := dialClientToken(t, url, "plug-slow")
	fast := dialClientToken(t, url, "host-secret")

	slow.send(Frame{Op: "call", ID: "slow-1", Method: "echo.method"})
	slowCall := provider.readUntil("call")

	// The slow caller stops draining before its reply is written.
	wedged := pluginConn(t, srv.router, "slow.plugin")
	wedged.writeMu.Lock()
	defer wedged.writeMu.Unlock()
	provider.send(Frame{Op: "result", ID: slowCall.ID, Result: json.RawMessage(`{"for":"slow"}`)})

	// A second caller's round trip must complete regardless — its result is not
	// queued behind a stranger's socket.
	fast.send(Frame{Op: "call", ID: "fast-1", Method: "echo.method"})
	fastCall := provider.readUntil("call")
	provider.send(Frame{Op: "result", ID: fastCall.ID, Result: json.RawMessage(`{"for":"fast"}`)})

	res := fast.readUntil("result")
	if res.ID != "fast-1" {
		t.Fatalf("result id = %q, want fast-1", res.ID)
	}
	if string(res.Result) != `{"for":"fast"}` {
		t.Fatalf("result = %s, want the fast caller's own reply", res.Result)
	}
}
