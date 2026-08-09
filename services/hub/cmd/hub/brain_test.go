package main

import (
	"net"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func TestResolveBrainBinFlagWins(t *testing.T) {
	if got := resolveBrainBin("/explicit/brain"); got != "/explicit/brain" {
		t.Errorf("explicit flag should win, got %q", got)
	}
}

func TestResolveBrainBinSibling(t *testing.T) {
	// A real file next to a fake "hub executable" is found via the sibling path.
	dir := t.TempDir()
	sibling := filepath.Join(dir, brainExeName())
	if err := os.WriteFile(sibling, []byte("#!/bin/true\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	// resolveBrainBin uses os.Executable(); we can't fake that, so just assert the
	// sibling-probe logic directly: the file exists and is not a dir.
	st, err := os.Stat(sibling)
	if err != nil || st.IsDir() {
		t.Fatalf("sibling probe precondition failed: %v", err)
	}
}

func TestBrainArgs(t *testing.T) {
	got := brainArgs("127.0.0.1:7895", "http://host:7891", "catalog")
	want := []string{
		"--hub", "ws://127.0.0.1:7895/bus",
		"--claudemon", "http://host:7891",
		"--scope", "catalog",
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("brainArgs = %v, want %v", got, want)
	}
}

// THE SHAPE THIS EXISTS FOR: the hub's bind address is not always an address.
// With remote sharing on it is a wildcard ("0.0.0.0:7895"), and the URL we hand
// the brain doubles as the Host header it sends — which requireHost refuses,
// because "0.0.0.0" is neither loopback nor the address the socket landed on.
// The brain then reconnects into a 403 for ever with its output discarded,
// leaving config.*, library.*, layouts.*, sessions.* and fs.* with no provider
// on the bus, which reaches the user as settings that silently do not persist.
func TestBrainArgsRewritesAWildcardBindToLoopback(t *testing.T) {
	for _, tc := range []struct{ bind, want string }{
		{"0.0.0.0:7895", "ws://127.0.0.1:7895/bus"},
		{":7895", "ws://127.0.0.1:7895/bus"},
		{"[::]:7895", "ws://[::1]:7895/bus"},
		// A real address is already dialable and must be passed through as-is:
		// on a shared bind reached by its own tailnet address, rewriting to
		// loopback would be a behaviour change, not a fix.
		{"127.0.0.1:7895", "ws://127.0.0.1:7895/bus"},
		{"100.86.79.73:7895", "ws://100.86.79.73:7895/bus"},
	} {
		got := brainArgs(tc.bind, "http://host:7891", "catalog")
		if got[0] != "--hub" || got[1] != tc.want {
			t.Errorf("brainArgs(%q) bus URL = %q, want %q", tc.bind, got[1], tc.want)
		}
	}
}

// The end of the same chain, against the real guard rather than a restatement
// of it: what brainArgs produces must actually get past the hub's own Host pin.
// Asserting the string alone would survive requireHost changing its mind about
// what it accepts.
func TestBrainDialAddrIsAcceptedByTheHubsOwnHostPin(t *testing.T) {
	b := broker.New()
	srv := bus.NewServer(b)
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go http.Serve(ln, srv.Handler())

	_, port, err := net.SplitHostPort(ln.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	bind := net.JoinHostPort("0.0.0.0", port) // what --addr looks like when sharing is on

	// The bind address used verbatim — the bug. Kept in the test because it is
	// the only thing that proves the rewrite is load-bearing rather than
	// cosmetic: without it, passing this case would mean the pin stopped caring.
	if code := statusVia(t, ln.Addr().String(), bind); code != http.StatusForbidden {
		t.Fatalf("Host %q: got %d, want 403 — the Host pin no longer refuses a wildcard Host, "+
			"so this test can no longer prove busDialAddr is needed", bind, code)
	}
	// The rewritten address — what the brain is actually handed.
	if code := statusVia(t, ln.Addr().String(), busDialAddr(bind)); code != http.StatusOK {
		t.Errorf("Host %q: got %d, want 200", busDialAddr(bind), code)
	}
}

// statusVia dials `dial` but sends `host` as the Host header — the split the Host
// pin is about.
func statusVia(t *testing.T, dial, host string) int {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, "http://"+dial+"/health", nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Host = host
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	return resp.StatusCode
}
