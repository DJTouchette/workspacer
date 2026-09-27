package main

import (
	"context"
	"flag"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestExternalClaudemonFlagAndPlan(t *testing.T) {
	fs := flag.NewFlagSet("serve", flag.ContinueOnError)
	flags := registerCommonServeFlags(fs)
	if err := fs.Parse([]string{"--external-claudemon"}); err != nil || !*flags.externalClaudemon {
		t.Fatalf("flag: %v", err)
	}
	plan := buildServePlan(serveOptions{ExternalClaudemon: true, Host: "127.0.0.1", APIPort: 12345, HookPort: 12346, HubPort: 12347, ClaudemonBin: "must-not-run", DBPath: "must-not-open", HubBin: "hub", BrainBin: "brain"})
	if plan.Init.Bin != "" || plan.Claudemon.Bin != "" {
		t.Fatalf("external engine has owned steps: %+v", plan)
	}
	if plan.ClaudemonHealth != "http://127.0.0.1:12345/health" {
		t.Fatal(plan.ClaudemonHealth)
	}
	for _, spec := range []childSpec{plan.Hub, plan.Brain} {
		if !strings.Contains(strings.Join(spec.Args, " "), "http://127.0.0.1:12345") {
			t.Fatal(spec)
		}
	}
}

func TestExternalClaudemonHealthContract(t *testing.T) {
	for _, tc := range []struct {
		name, body, marker string
		status             int
		good               bool
	}{
		{"daemon", "ok", "1", 200, true}, {"unrelated", "ok", "", 200, false}, {"wrong body", "healthy", "1", 200, false}, {"failed", "ok", "1", 503, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("X-Workspacer-Maintenance", tc.marker)
				w.WriteHeader(tc.status)
				io.WriteString(w, tc.body)
			}))
			defer srv.Close()
			err := waitForExternalClaudemon(context.Background(), srv.URL, 0)
			if (err == nil) != tc.good {
				t.Fatalf("health result %v, want good=%v", err, tc.good)
			}
		})
	}
	t.Run("redirect", func(t *testing.T) {
		target := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { t.Error("followed health redirect") }))
		defer target.Close()
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, target.URL, http.StatusFound) }))
		defer srv.Close()
		if err := waitForExternalClaudemon(context.Background(), srv.URL, 0); err == nil {
			t.Fatal("accepted redirect")
		}
	})
}

func TestExternalClaudemonSurvivesOwnedStackFailure(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("stand-in child uses /bin/sh")
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Workspacer-Maintenance", "1")
		io.WriteString(w, "ok")
	}))
	defer srv.Close()
	hook, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer hook.Close()
	dir := t.TempDir()
	logPath := filepath.Join(dir, "calls.log")
	opts := serveOptions{ExternalClaudemon: true, Host: "127.0.0.1", APIPort: srv.Listener.Addr().(*net.TCPAddr).Port, HookPort: hook.Addr().(*net.TCPAddr).Port, HubPort: freePorts(t, 1)[0], ClaudemonBin: fakeDaemon(t, dir, "claudemon", logPath), HubBin: fakeDaemon(t, dir, "hub", logPath)}
	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()
	if stk, err := bootStack(ctx, opts, io.Discard); err == nil {
		stk.shutdown(io.Discard)
		t.Fatal("fake hub unexpectedly healthy")
	}
	calls, _ := os.ReadFile(logPath)
	// Hub argv legitimately contains --claudemon; only a daemon executable
	// record at the beginning of a line indicates incorrect ownership.
	if !strings.HasPrefix(string(calls), "hub ") || strings.Contains(string(calls), "\nclaudemon ") {
		t.Fatalf("unexpected owned processes: %s", calls)
	}
	if err := waitForExternalClaudemon(context.Background(), srv.URL, 0); err != nil {
		t.Fatalf("external daemon stopped: %v", err)
	}
}

func TestExternalClaudemonStillRefusesOwnedPortCollision(t *testing.T) {
	occupied, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer occupied.Close()
	port := occupied.Addr().(*net.TCPAddr).Port
	opts := serveOptions{ExternalClaudemon: true, Host: "127.0.0.1", HubPort: port}
	if _, err := bootStack(context.Background(), opts, io.Discard); err == nil || !strings.Contains(err.Error(), "hub port") {
		t.Fatalf("hub collision: %v", err)
	}
	opts.HubPort = freePorts(t, 1)[0]
	opts.MCPBin = "mcp"
	opts.MCPPort = port
	if _, err := bootStack(context.Background(), opts, io.Discard); err == nil || !strings.Contains(err.Error(), "MCP facade port") {
		t.Fatalf("facade collision: %v", err)
	}
}

func TestExternalClaudemonReadinessFailureStartsNothing(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "ok") }))
	defer srv.Close()
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	opts := serveOptions{ExternalClaudemon: true, Host: "127.0.0.1", HubPort: freePorts(t, 1)[0], APIPort: srv.Listener.Addr().(*net.TCPAddr).Port, HubBin: "must-not-execute"}
	if _, err := bootStack(ctx, opts, io.Discard); err == nil || !strings.Contains(err.Error(), "external claudemon failed readiness") {
		t.Fatalf("preflight: %v", err)
	}
}
