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
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

const readyTimeout = 20 * time.Second

// claudemon's default ports, named once so the flag defaults and the
// shared-database guard cannot drift apart. Changing either of these is the
// gesture that means "a SECOND claudemon on this machine" — see resolveDBPath.
const (
	defaultClaudemonAPIPort  = 7891
	defaultClaudemonHookPort = 7890
)

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
	trustedHost                    *string
	hubPort, apiPort, hookPort     *int
	claudemonBin, hubBin, brainBin *string
	dbPath                         *string
	noClaudemonInit                *bool
	allowNewToken                  *bool
}

func registerCommonServeFlags(fs *flag.FlagSet) *commonServeFlags {
	return &commonServeFlags{
		host: fs.String("host", "127.0.0.1",
			"bind host for the hub (bus + /remote + /m). The default is loopback-only; binding wider (e.g. 0.0.0.0 or a tailnet IP) is your explicit opt-in to remote access — pair it with a private network like Tailscale, never the open internet"),
		hubPort:      fs.Int("hub-port", 7895, "hub port (event bus + web clients)"),
		apiPort:      fs.Int("claudemon-api-port", defaultClaudemonAPIPort, "claudemon API port (session state + control)"),
		hookPort:     fs.Int("claudemon-hook-port", defaultClaudemonHookPort, "claudemon hook-ingestion port (Claude Code hooks post here)"),
		claudemonBin: fs.String("claudemon-bin", "", "path to the claudemon binary (default: sibling of this binary, then PATH)"),
		hubBin:       fs.String("hub-bin", "", "path to the hub binary (default: sibling of this binary, then PATH)"),
		brainBin:     fs.String("brain-bin", "", "path to the brain binary the hub supervises (default: sibling of this binary, then the hub auto-detects its own sibling / PATH)"),
		trustedHost: fs.String("trusted-host", os.Getenv("HUB_TRUSTED_HOSTS"),
			"comma-separated hostname(s) a reverse proxy in front of the hub presents (e.g. the `tailscale serve` MagicDNS name). A TLS front-end terminates elsewhere and forwards to our loopback socket, which is the DNS-rebinding shape the hub's Host/Origin pins refuse, so it must be named or every route behind it answers 403"),
		dbPath: fs.String("claudemon-db-path", "",
			"path to claudemon's SQLite session store (default: $XDG_DATA_HOME/claudemon/state.db, else ~/.claudemon/state.db — the same file the desktop app uses, deliberately). REQUIRED when you change claudemon's ports: that means a second daemon, and two daemons on one state.db share every session and event row"),
		noClaudemonInit: fs.Bool("no-claudemon-init", false,
			"skip the claudemon-init pre-flight that registers Claude Code's hooks + statusLine in ~/.claude/settings.json. Only for an operator who writes that file themselves (or ships it read-only): without those hooks a PTY session never leaves mode=unknown, and a spawn's first message — held until the Input transition — is never delivered"),
		allowNewToken: fs.Bool("allow-new-token", os.Getenv("WORKSPACER_ALLOW_NEW_TOKEN") == "1",
			"mint a NEW <config>/workspacer/remote-token even though the config dir holds other state. Say this once, deliberately, when the old pairing credential is gone for good: every client, phone and federation peer paired against it must be re-paired (default: $WORKSPACER_ALLOW_NEW_TOKEN=1)"),
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
		TrustedHosts:  *f.trustedHost,

		SkipClaudemonInit: *f.noClaudemonInit,
		DBPath:            *f.dbPath,
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
		tok, err := loadOrCreateTokenAllowingNew(configDir(), *f.allowNewToken)
		if err != nil {
			fmt.Fprintf(os.Stderr, "workspacer: %v\n", err)
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

	// Decide which session database this stack opens BEFORE probing a port or
	// starting a process: it is the one refusal that must happen before any
	// side effect, since the damage it prevents is done the moment a second
	// claudemon opens the file.
	dbPath, err := resolveDBPath(opts)
	if err != nil {
		return nil, err
	}
	opts.DBPath = dbPath

	plan := buildServePlan(opts)

	// Before the daemons, not beside them: a session spawned in the window
	// before the hooks land comes up hookless and stays that way for its whole
	// life. See servePlan.Init.
	runInitStep(ctx, plan.Init, logw)

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

// initStepTimeout bounds the pre-flight. It merges one small JSON file, so a
// claudemon that wedges here must not hold the whole server hostage.
const initStepTimeout = 15 * time.Second

// runInitStep runs the hook-registration pre-flight to completion and reports
// what happened. A zero spec is the --no-claudemon-init opt-out and does
// nothing.
//
// NON-FATAL on purpose, and loudly so. The two ways this fails are an
// unparseable ~/.claude/settings.json and a read-only home, and in both cases a
// server that still comes up is strictly more useful than one that does not:
// stream-transport sessions (the default — see config_defaults.json) do not
// depend on these hooks at all, so the degradation is real but partial. What
// must not happen is that it degrades QUIETLY, which is the whole failure this
// step exists to end — so the warning names the consequence rather than the
// exit code.
func runInitStep(ctx context.Context, spec childSpec, logw io.Writer) {
	if spec.Bin == "" {
		return
	}
	cctx, cancel := context.WithTimeout(ctx, initStepTimeout)
	defer cancel()

	cmd := exec.CommandContext(cctx, spec.Bin, spec.Args...)
	if len(spec.Env) > 0 {
		cmd.Env = append(os.Environ(), spec.Env...)
	}
	out, err := cmd.CombinedOutput()
	trimmed := strings.TrimSpace(string(out))
	if err != nil {
		fmt.Fprintf(logw, "[workspacer] WARNING: `%s %s` failed: %v\n",
			spec.Bin, strings.Join(spec.Args, " "), err)
		if trimmed != "" {
			fmt.Fprintf(logw, "[workspacer]   %s\n", trimmed)
		}
		fmt.Fprintln(logw, "[workspacer] WARNING: Claude Code's hooks are NOT registered in ~/.claude/settings.json.")
		fmt.Fprintln(logw, "[workspacer]   PTY sessions will report mode=unknown for their whole life, a spawn's first")
		fmt.Fprintln(logw, "[workspacer]   message is never delivered, and permission prompts produce no approvable")
		fmt.Fprintln(logw, "[workspacer]   record. Stream-transport sessions are unaffected. Fix by running")
		fmt.Fprintf(logw, "[workspacer]   `%s %s` by hand, or pass --no-claudemon-init if that file is yours to own.\n",
			spec.Bin, strings.Join(spec.Args, " "))
		return
	}
	if trimmed != "" {
		fmt.Fprintf(logw, "[workspacer] claudemon init: %s\n", trimmed)
	}
}

// resolveDBPath decides which SQLite session store this stack opens, and refuses
// the one combination that means "a second stack quietly sharing the live one".
//
// THE DEFAULT MUST NOT MOVE. `workspacer serve` and the desktop app share one
// session store on purpose — they adopt each other rather than coexisting (see
// claudemonDaemon.ts "adopted external daemon") — so the derived path mirrors
// claudemon's own default_db_path (services/claudemon/src/store/mod.rs) exactly.
// Pinning it is about making the choice VISIBLE and testable, not about changing
// it: a relocation here would strand every existing install's sessions.
//
// Two things the mirror deliberately does NOT copy:
//
//   - claudemon's third fallback, the RELATIVE ".claudemon/state.db" under the
//     process CWD. On a container that is the rootfs that gets rebuilt on every
//     start, so a session store landing there is total silent loss. We refuse
//     and name the fix instead of laundering it into a real-looking path.
//   - `std::env::var("XDG_DATA_HOME")` returning Ok("") for a set-but-empty
//     variable, which sends the Rust side to a relative path too. Same refusal.
//
// THE GUARD: bootStack already refuses ports that are busy, so the ONLY way to
// get a second claudemon on this machine is to ask for one by changing its
// ports. Doing that while leaving the database alone is not a configuration
// anybody wants — the two daemons then share the `sessions` and `events` tables,
// the newcomer's boot hydration adopts the live stack's sessions as resumable
// rows of its own, its clients can resume and act on them, and its
// fleet.quiescence sampler counts them. So alternate ports REQUIRE an explicit
// --claudemon-db-path. Refusing (rather than warning) matches the rest of this
// command: `serve` is a foreground CLI whose exit is the loudest signal it has,
// the damage is silent and to live agent state, and the way past is one flag —
// including passing the shared default explicitly if that is genuinely wanted.
func resolveDBPath(opts serveOptions) (string, error) {
	if opts.DBPath != "" {
		return opts.DBPath, nil
	}
	secondDaemon := opts.APIPort != defaultClaudemonAPIPort || opts.HookPort != defaultClaudemonHookPort

	derived, err := deriveClaudemonDBPath()
	if err != nil {
		return "", err
	}
	if secondDaemon {
		return "", fmt.Errorf(
			"refusing to open the shared session database from a second claudemon.\n"+
				"  You changed claudemon's ports (--claudemon-api-port %d, --claudemon-hook-port %d), which starts a\n"+
				"  SECOND daemon — but its database would still be %s, the same file the stack on the\n"+
				"  default ports uses. Two daemons on one store share every session and event row: the newcomer\n"+
				"  lists the live stack's agents as its own resumable sessions, its clients can resume and act on\n"+
				"  them, and its fleet signal counts them.\n"+
				"  Give this stack its own: --claudemon-db-path /some/scratch/state.db (or set XDG_DATA_HOME).\n"+
				"  If sharing really is what you want, pass --claudemon-db-path %s explicitly",
			opts.APIPort, opts.HookPort, derived, derived)
	}
	return derived, nil
}

// deriveClaudemonDBPath mirrors services/claudemon/src/store/mod.rs
// default_db_path(). Kept in lockstep with it by TestResolveDBPath; if that
// function ever moves, this must move with it or `serve` silently relocates
// every existing install's sessions.
func deriveClaudemonDBPath() (string, error) {
	if xdg, ok := os.LookupEnv("XDG_DATA_HOME"); ok {
		p := filepath.Join(xdg, "claudemon", "state.db")
		if !filepath.IsAbs(p) {
			return "", fmt.Errorf(
				"XDG_DATA_HOME=%q is not an absolute path, so claudemon's session database would land at %q —\n"+
					"  a path relative to whatever this process's working directory happens to be, which on a container\n"+
					"  is the rootfs that is rebuilt on every start. Set XDG_DATA_HOME to an absolute path, or pass\n"+
					"  --claudemon-db-path", xdg, p)
		}
		return p, nil
	}
	if home := authtoken.HomeDir(); home != "" {
		p := filepath.Join(home, ".claudemon", "state.db")
		if !filepath.IsAbs(p) {
			return "", fmt.Errorf("home directory %q is not absolute; pass --claudemon-db-path", home)
		}
		return p, nil
	}
	return "", fmt.Errorf(
		"cannot work out where claudemon's session database belongs: neither $XDG_DATA_HOME nor a home\n" +
			"  directory is resolvable. Leaving it to the daemon would put it at a path relative to this\n" +
			"  process's working directory, where it is lost on the next restart. Set $HOME, or pass\n" +
			"  --claudemon-db-path")
}
