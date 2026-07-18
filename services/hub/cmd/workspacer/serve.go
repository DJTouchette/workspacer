package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"
)

const readyTimeout = 20 * time.Second

// runServe is the product face of headless mode: resolve the sibling daemons,
// wire them together, supervise them, and tell the operator how to connect.
func runServe(args []string) int {
	fs := flag.NewFlagSet("workspacer serve", flag.ExitOnError)
	cf := registerCommonServeFlags(fs)
	pluginsDir := fs.String("plugins-dir", filepath.Join(configDir(), "plugins"),
		"plugins directory the hub loads + supervises (the same one the desktop app uses); empty = no plugins")
	webappDir := fs.String("webapp-dir", "", "built web app (dist/web) for the hub to serve at /app/ (full remote parity); empty = $WORKSPACER_WEBAPP_DIR, else a web/ dir shipped next to this binary, else disabled")
	jsonOut := fs.Bool("json", false, "print the ready banner as one JSON object on stdout (logs stay on stderr)")
	_ = fs.Parse(args)

	opts, ok := cf.resolveOptions()
	if !ok {
		return 1
	}
	opts.PluginsDir = *pluginsDir
	opts.WebappDir = resolveWebappDir(*webappDir, selfDir())
	if opts.PluginsDir != "" {
		// Best-effort: a missing plugins dir must not block serving.
		if err := os.MkdirAll(opts.PluginsDir, 0o755); err != nil {
			fmt.Fprintf(os.Stderr, "workspacer: warning: cannot create plugins dir %s: %v (continuing without plugins)\n", opts.PluginsDir, err)
			opts.PluginsDir = ""
		}
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	stk, err := bootStack(ctx, opts, os.Stderr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: %v\n", err)
		return 1
	}

	if *jsonOut {
		enc := json.NewEncoder(os.Stdout)
		_ = enc.Encode(stk.plan.Banner)
	} else {
		fmt.Print(renderBanner(stk.plan.Banner))
	}

	<-ctx.Done()
	stop() // restore default signal handling: a second Ctrl-C force-quits
	stk.shutdown(os.Stderr)
	return 0
}

// commonServeFlags are the flags shared by `serve` and `plugin dev`: the bind
// host, the two claudemon ports + hub port, the three binary overrides, and the
// bus token. Registered once (registerCommonServeFlags) so both commands keep
// byte-identical defaults and help text.
type commonServeFlags struct {
	host, token                    *string
	hubPort, apiPort, hookPort     *int
	claudemonBin, hubBin, brainBin *string
}

func registerCommonServeFlags(fs *flag.FlagSet) *commonServeFlags {
	return &commonServeFlags{
		host: fs.String("host", "127.0.0.1",
			"bind host for the hub (bus + /remote + /m). The default is loopback-only; binding wider (e.g. 0.0.0.0 or a tailnet IP) is your explicit opt-in to remote access — pair it with a private network like Tailscale, never the open internet"),
		hubPort:      fs.Int("hub-port", 7895, "hub port (event bus + web clients)"),
		apiPort:      fs.Int("claudemon-api-port", 7891, "claudemon API port (session state + control)"),
		hookPort:     fs.Int("claudemon-hook-port", 7890, "claudemon hook-ingestion port (Claude Code hooks post here)"),
		claudemonBin: fs.String("claudemon-bin", "", "path to the claudemon binary (default: sibling of this binary, then PATH)"),
		hubBin:       fs.String("hub-bin", "", "path to the hub binary (default: sibling of this binary, then PATH)"),
		brainBin:     fs.String("brain-bin", "", "path to the brain binary the hub supervises (default: sibling of this binary, then the hub auto-detects its own sibling / PATH)"),
		token: fs.String("token", os.Getenv("HUB_TOKEN"),
			"bus auth token / pairing credential (default: $HUB_TOKEN, else the persisted <config>/workspacer/remote-token, minted on first run)"),
	}
}

// resolveOptions turns parsed common flags into a serveOptions with the sibling
// binaries resolved, the advertise host chosen, and the token loaded/minted.
// PluginsDir and WebappDir are left to the caller (they differ per command). It
// prints the same operator-facing errors/warnings runServe used and returns
// ok=false when a required binary or the token can't be obtained.
func (f *commonServeFlags) resolveOptions() (serveOptions, bool) {
	sib := selfDir()
	opts := serveOptions{
		Host:          *f.host,
		HubPort:       *f.hubPort,
		APIPort:       *f.apiPort,
		HookPort:      *f.hookPort,
		Token:         *f.token,
		ClaudemonBin:  resolveBin("claudemon", *f.claudemonBin, sib),
		HubBin:        resolveBin("hub", *f.hubBin, sib),
		BrainBin:      resolveBin("brain", *f.brainBin, sib),
		AdvertiseHost: advertiseHost(*f.host, localIPv4s()),
	}
	if opts.ClaudemonBin == "" {
		fmt.Fprintln(os.Stderr, "workspacer: claudemon binary not found next to this binary or on PATH (build it with `make build-claudemon`, or pass --claudemon-bin)")
		return opts, false
	}
	if opts.HubBin == "" {
		fmt.Fprintln(os.Stderr, "workspacer: hub binary not found next to this binary or on PATH (build it with `make build-cli`, or pass --hub-bin)")
		return opts, false
	}
	// The brain is resolved here (relative to *this* binary) and passed down so
	// a hub found on PATH still supervises the brain that shipped with us; when
	// neither of us finds one, the hub logs it and serves the bus brain-less.
	if opts.BrainBin == "" {
		fmt.Fprintln(os.Stderr, "workspacer: warning: no brain binary found — the bus will have no headless capability provider (build it with `make build-cli`, or pass --brain-bin)")
	}
	// The token is what makes a headless server pairable *and* safe: unlike the
	// desktop-spawned hub there is no trusted local UI, so we never run open.
	if opts.Token == "" {
		tok, err := loadOrCreateToken(configDir())
		if err != nil {
			fmt.Fprintf(os.Stderr, "workspacer: cannot create auth token: %v\n", err)
			return opts, false
		}
		opts.Token = tok
	}
	return opts, true
}

// stack is a booted claudemon + hub pair plus the plan that wired them, returned
// by bootStack. Shared by `serve` and `plugin dev`.
type stack struct {
	plan      servePlan
	claudemon *child
	hub       *child
}

// shutdown stops the hub first, then claudemon: SIGTERM lets the hub tear down
// the brain and plugin sidecars while claudemon (which the brain talks to) is
// still alive.
func (s *stack) shutdown(logw io.Writer) {
	fmt.Fprintln(logw, "[workspacer] shutting down…")
	s.hub.Stop()
	s.claudemon.Stop()
}

// bootStack probes the ports are free, starts + supervises claudemon and the
// hub per the plan built from opts, and waits for both to report healthy. On any
// failure it tears down whatever it started and returns the error. ctx
// cancellation stops the children. Extracted from runServe so `plugin dev` boots
// the identical stack (only the plugins dir + what happens after ready differ).
func bootStack(ctx context.Context, opts serveOptions, logw io.Writer) (*stack, error) {
	// Refuse to double-start rather than killing whatever holds the ports (the
	// desktop kills stale listeners because it *owns* its daemons; a CLI can't
	// know the listener isn't another deliberate workspacer server).
	ports := []struct {
		host string
		port int
		what string
	}{
		{"127.0.0.1", opts.HookPort, "claudemon hook port"},
		{"127.0.0.1", opts.APIPort, "claudemon API port"},
		{opts.Host, opts.HubPort, "hub port"},
	}
	for _, p := range ports {
		if err := probeListen(p.host, p.port); err != nil {
			return nil, fmt.Errorf("%s %d is already in use (%v) — is a workspacer server or the desktop app already running? Try `workspacer status`", p.what, p.port, err)
		}
	}

	plan := buildServePlan(opts)

	fmt.Fprintln(logw, "[workspacer] starting claudemon + hub (brain-scope full)…")
	claudemon := startChild(ctx, plan.Claudemon, logw, newRestartBackoff())
	hub := startChild(ctx, plan.Hub, logw, newRestartBackoff())
	s := &stack{plan: plan, claudemon: claudemon, hub: hub}

	if err := waitForHealth(ctx, plan.ClaudemonHealth, readyTimeout); err != nil {
		s.shutdown(logw)
		return nil, fmt.Errorf("claudemon failed to become healthy: %w", err)
	}
	if err := waitForHealth(ctx, plan.HubHealth, readyTimeout); err != nil {
		s.shutdown(logw)
		return nil, fmt.Errorf("hub failed to become healthy: %w", err)
	}
	return s, nil
}

// probeListen checks a port is free by briefly binding it. There is a small
// window between the probe and the child binding for real, but a clear
// "already running" error beats the alternative (both daemons crash-looping
// on EADDRINUSE with confusing logs).
func probeListen(host string, port int) error {
	l, err := net.Listen("tcp", net.JoinHostPort(host, fmt.Sprintf("%d", port)))
	if err != nil {
		return err
	}
	return l.Close()
}

// waitForHealth polls url until it answers 200, like the desktop's
// waitForHealth — spawn success alone doesn't mean the daemon is serving.
func waitForHealth(ctx context.Context, url string, timeout time.Duration) error {
	client := &http.Client{Timeout: 2 * time.Second}
	deadline := time.Now().Add(timeout)
	for {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
		if err != nil {
			return err
		}
		resp, err := client.Do(req)
		if err == nil {
			resp.Body.Close()
			if resp.StatusCode == http.StatusOK {
				return nil
			}
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("no healthy answer from %s within %s", url, timeout)
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
}

// resolveWebappDir: explicit flag wins, then the hub's own $WORKSPACER_WEBAPP_DIR
// fallback (by leaving it empty), then the web build shipped next to the binary
// — so the packaged app's bundled CLI and the server tarball serve /app with no
// flags at all.
func resolveWebappDir(flag, sib string) string {
	if flag != "" || os.Getenv("WORKSPACER_WEBAPP_DIR") != "" {
		return flag
	}
	return defaultWebappDir(sib)
}
