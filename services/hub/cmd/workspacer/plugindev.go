package main

// `workspacer plugin dev <plugin-dir>` is a single-plugin development loop. It
// boots the SAME backend stack `serve` does (claudemon + hub + full-scope
// brain), but isolates one plugin so the hub loads and supervises only the
// plugin under development, then hot-reloads it on file changes:
//
//   - the plugin's directory is symlinked into a throwaway plugins dir, so the
//     hub's LoadDir scans exactly this plugin and nothing else;
//   - a poll-based watcher (no fsnotify — the repo is deliberately zero-dep)
//     scans the tree on the debounce interval, ignoring VCS/dep noise and the
//     loader's own sidecar markers, and fires once a burst of edits settles;
//   - on a change it re-runs the manifest's install/build argv (streaming its
//     output; a failed build SKIPS the reload) then POSTs /plugins/reload, which
//     makes the hub stop→reload→restart→re-token the sidecar and emit
//     plugin.loaded;
//   - a bus subscription surfaces plugin.*/sidecar.* lifecycle events so the
//     developer sees crashes/health, since the sidecar supervisor logs those to
//     the bus, not to the hub's stderr.
//
// The plugin sidecar's own stdout/stderr is also streamed here in dev mode: the
// stack is booted with --plugins-stream-logs (opts.DevStreamLogs), so the hub's
// supervisor publishes each output line as a plugin.log bus event, which the
// bus subscription below prints as "[<plugin>] <line>". Plain `serve` leaves the
// flag off, so production never pays for the bus log traffic.

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

const pluginUsage = `workspacer plugin — plugin development tools

Usage:
  workspacer plugin dev <plugin-dir>   boot the backend stack and hot-reload one
                                       plugin on file changes

Run "workspacer plugin dev -h" for its flags.
`

// runPlugin dispatches the `plugin` subcommand group.
func runPlugin(args []string) int {
	if len(args) == 0 {
		fmt.Fprint(os.Stderr, pluginUsage)
		return 2
	}
	switch args[0] {
	case "dev":
		return runPluginDev(args[1:])
	case "help", "-h", "--help":
		fmt.Print(pluginUsage)
		return 0
	default:
		fmt.Fprintf(os.Stderr, "workspacer plugin: unknown subcommand %q\n\n%s", args[0], pluginUsage)
		return 2
	}
}

// runPluginDev implements `workspacer plugin dev <plugin-dir>`.
func runPluginDev(args []string) int {
	fs := flag.NewFlagSet("workspacer plugin dev", flag.ExitOnError)
	cf := registerCommonServeFlags(fs)
	debounce := fs.Duration("debounce", 400*time.Millisecond,
		"quiet period after a file change before rebuilding + reloading the plugin")
	_ = fs.Parse(args)

	rest := fs.Args()
	if len(rest) != 1 {
		fmt.Fprintln(os.Stderr, "usage: workspacer plugin dev <plugin-dir>")
		return 2
	}
	dir, err := filepath.Abs(rest[0])
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: %v\n", err)
		return 1
	}

	// Fail fast with a clear error before booting anything expensive: the dir
	// must hold a plugin.json that parses and validates.
	mf, err := plugin.Load(filepath.Join(dir, "plugin.json"))
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: %s is not a valid plugin: %v\n", dir, err)
		return 1
	}
	fmt.Fprintf(os.Stderr, "[plugin-dev] plugin %q (%s) — %s\n", mf.ID, mf.Name, pluginKind(mf))

	// Isolate this one plugin: a throwaway plugins dir containing only a symlink
	// to the developer's dir, so the hub's LoadDir picks up exactly this plugin.
	tmp, cleanup, err := makeDevPluginsDir(dir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: %v\n", err)
		return 1
	}
	defer cleanup()

	opts, ok := cf.resolveOptions()
	if !ok {
		return 1
	}
	opts.PluginsDir = tmp
	opts.DevStreamLogs = true // stream the sidecar's stdout/stderr to the dev loop

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	stk, err := bootStack(ctx, opts, os.Stderr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: %v\n", err)
		return 1
	}

	hubBase := fmt.Sprintf("http://127.0.0.1:%d", opts.HubPort)
	fmt.Fprint(os.Stderr, renderDevBanner(stk.plan.Banner, mf, dir, *debounce))

	// Surface plugin/sidecar lifecycle events (the supervisor publishes crashes +
	// health to the bus, not to stderr) so the developer sees them. Best-effort.
	go watchBusEvents(ctx, hubBase, opts.Token, os.Stderr)

	// Poll for changes and hot-reload until Ctrl-C.
	watchLoop(ctx, dir, *debounce, func() {
		reloadOnce(ctx, hubBase, opts.Token, dir, os.Stderr)
	})

	stop() // restore default signal handling: a second Ctrl-C force-quits
	stk.shutdown(os.Stderr)
	return 0
}

// pluginKind describes what a manifest contributes, for the dev banner.
func pluginKind(mf plugin.Manifest) string {
	switch {
	case mf.Server != nil:
		return "sidecar server"
	case mf.UI != "":
		return "webview-only (hub-served UI)"
	default:
		return "capability provider (no sidecar)"
	}
}

// makeDevPluginsDir creates a throwaway plugins directory whose only entry is a
// symlink to the developer's plugin dir, so the hub loads exactly this plugin.
// cleanup removes the temp dir; it never removes the developer's dir (only the
// link and its parent temp dir).
func makeDevPluginsDir(pluginDir string) (dir string, cleanup func(), err error) {
	tmp, err := os.MkdirTemp("", "wks-plugin-dev-")
	if err != nil {
		return "", nil, fmt.Errorf("create dev plugins dir: %w", err)
	}
	link := filepath.Join(tmp, filepath.Base(pluginDir))
	if err := os.Symlink(pluginDir, link); err != nil {
		_ = os.RemoveAll(tmp)
		return "", nil, fmt.Errorf("symlink plugin into dev plugins dir: %w", err)
	}
	return tmp, func() { _ = os.RemoveAll(tmp) }, nil
}

// --- change detection ------------------------------------------------------

// devWatchIgnore reports whether a directory or file name must be skipped by the
// watcher: VCS/dependency noise (heavy + irrelevant to a reload) and the
// loader's own sidecar markers, which the hub writes into the plugin dir — a
// reload rewrites .bus-token, so watching it would loop forever.
func devWatchIgnore(name string) bool {
	switch name {
	case ".git", "node_modules",
		".bus-token", ".settings.json", ".install-source", ".disabled":
		return true
	}
	return false
}

// scanState is a cheap fingerprint of a directory tree: the newest mtime and the
// number of files seen. A newer mtime catches edits/creates; a different count
// catches creates/deletes that don't move the max mtime.
type scanState struct {
	maxMod time.Time
	count  int
}

// changed reports whether b differs from a.
func (a scanState) changed(b scanState) bool {
	return b.count != a.count || b.maxMod.After(a.maxMod)
}

// scanTree fingerprints root, pruning devWatchIgnore names. Per-entry errors are
// ignored (a file vanishing mid-walk simply isn't counted); a walk of a missing
// root yields the zero state.
func scanTree(root string) scanState {
	var st scanState
	_ = filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if devWatchIgnore(d.Name()) {
			if d.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if d.IsDir() {
			return nil
		}
		info, err := d.Info()
		if err != nil {
			return nil
		}
		st.count++
		if m := info.ModTime(); m.After(st.maxMod) {
			st.maxMod = m
		}
		return nil
	})
	return st
}

// watchLoop polls dir every interval and calls onChange once a burst of edits
// settles (debounce): a scan that differs from the last is recorded but does NOT
// fire; onChange fires on the first subsequent quiet scan. Returns when ctx is
// done. onChange runs synchronously on the poll goroutine, so a reload in flight
// naturally back-pressures further polling.
func watchLoop(ctx context.Context, dir string, interval time.Duration, onChange func()) {
	if interval <= 0 {
		interval = 400 * time.Millisecond
	}
	prev := scanTree(dir)
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	dirty := false
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			cur := scanTree(dir)
			if prev.changed(cur) {
				prev = cur
				dirty = true
				continue // keep waiting for the burst to settle
			}
			if dirty {
				dirty = false
				onChange()
			}
		}
	}
}

// --- reload ----------------------------------------------------------------

// reloadOnce runs one change→reload cycle: re-read the manifest, run its build
// step (if any), then trigger a hub reload. A failed build SKIPS the reload so a
// broken tree never restarts the sidecar with a stale/broken binary. Every
// outcome is printed with a timestamp so the developer can follow the loop.
func reloadOnce(ctx context.Context, hubBase, token, dir string, logw io.Writer) {
	ts := time.Now().Format("15:04:05")
	mf, err := plugin.Load(filepath.Join(dir, "plugin.json"))
	if err != nil {
		fmt.Fprintf(logw, "[plugin-dev %s] change detected but manifest is invalid, skipping reload: %v\n", ts, err)
		return
	}
	if len(mf.Install) > 0 {
		fmt.Fprintf(logw, "[plugin-dev %s] change detected — building: %s\n", ts, strings.Join(mf.Install, " "))
		if err := runDevInstall(ctx, dir, mf.Install, logw); err != nil {
			fmt.Fprintf(logw, "[plugin-dev %s] build FAILED, skipping reload: %v\n", ts, err)
			return
		}
	} else {
		fmt.Fprintf(logw, "[plugin-dev %s] change detected — reloading\n", ts)
	}
	if err := postReload(ctx, hubBase, token, dir); err != nil {
		fmt.Fprintf(logw, "[plugin-dev %s] reload FAILED: %v\n", ts, err)
		return
	}
	fmt.Fprintf(logw, "[plugin-dev %s] reloaded %q ok\n", ts, mf.ID)
}

// runDevInstall runs the plugin's install/build argv in dir, streaming stdout +
// stderr to logw with an [install] prefix so build output is visible live.
// Bounded by a 5-minute wall-clock timeout, matching the plugin package's own
// install step. (We can't reuse plugin.runInstall: it's unexported and buffers
// output rather than streaming it.)
func runDevInstall(ctx context.Context, dir string, argv []string, logw io.Writer) error {
	cctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(cctx, argv[0], argv[1:]...)
	cmd.Dir = dir
	pw := &prefixWriter{w: logw, prefix: "[install] "}
	cmd.Stdout = pw
	cmd.Stderr = pw
	err := cmd.Run()
	pw.flush()
	return err
}

// postReload asks the hub to hot-reload the plugin at dir (POST /plugins/reload).
// It sends the developer's real dir so mgr.Add resolves the sidecar cwd, ui
// assets, and token file against the real tree. Authenticated with the bus token
// (Bearer), matching the sibling /plugins/* routes.
func postReload(ctx context.Context, hubBase, token, dir string) error {
	body, _ := json.Marshal(map[string]string{"dir": dir})
	cctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(cctx, http.MethodPost, hubBase+"/plugins/reload", strings.NewReader(string(body)))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		var e struct {
			Error string `json:"error"`
		}
		_ = json.NewDecoder(resp.Body).Decode(&e)
		if e.Error != "" {
			return fmt.Errorf("hub: %s", e.Error)
		}
		return fmt.Errorf("hub returned HTTP %d", resp.StatusCode)
	}
	return nil
}

// --- bus lifecycle events --------------------------------------------------

// busFrame is the minimal subset of the hub's bus.Frame the dev loop reads +
// writes: a subscribe request and inbound event frames.
type busFrame struct {
	Op     string          `json:"op"`
	Topics []string        `json:"topics,omitempty"`
	Event  *event.Envelope `json:"event,omitempty"`
}

// watchBusEvents subscribes to plugin.*/sidecar.* on the bus and prints each
// event, so sidecar crashes and health transitions (published by the supervisor
// to the bus, not to stderr) reach the developer. Best-effort: any failure just
// falls back to the [hub] logs. Returns when ctx is done or the socket drops.
func watchBusEvents(ctx context.Context, hubBase, token string, logw io.Writer) {
	wsURL := "ws" + strings.TrimPrefix(hubBase, "http") + "/bus"
	if token != "" {
		wsURL += "?token=" + url.QueryEscape(token)
	}
	c, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		fmt.Fprintf(logw, "[plugin-dev] bus event stream unavailable (%v) — relying on [hub] logs for lifecycle\n", err)
		return
	}
	defer c.CloseNow()
	if err := wsjson.Write(ctx, c, busFrame{Op: "subscribe", Topics: []string{"plugin.*", "sidecar.*"}}); err != nil {
		return
	}
	for {
		var f busFrame
		if err := wsjson.Read(ctx, c, &f); err != nil {
			return // ctx cancelled or socket closed on shutdown
		}
		if f.Op != "event" || f.Event == nil {
			continue
		}
		ev := f.Event
		ts := time.Now().Format("15:04:05")
		// A sidecar log line: print it as the sidecar wrote it (prefixed with the
		// plugin name), not as a raw bus envelope — this is the sidecar's own
		// stdout/stderr streamed through by the supervisor (Spec.LogLines).
		if ev.Type == "plugin.log" {
			var l struct {
				Name   string `json:"name"`
				Stream string `json:"stream"`
				Line   string `json:"line"`
			}
			if err := json.Unmarshal(ev.Data, &l); err == nil {
				marker := l.Name
				if l.Stream == "stderr" {
					marker += " err"
				}
				fmt.Fprintf(logw, "[%s] %s\n", marker, l.Line)
				continue
			}
		}
		if len(ev.Data) > 0 {
			fmt.Fprintf(logw, "[plugin-dev %s] bus %s (from %s): %s\n", ts, ev.Type, ev.Source, string(ev.Data))
		} else {
			fmt.Fprintf(logw, "[plugin-dev %s] bus %s (from %s)\n", ts, ev.Type, ev.Source)
		}
	}
}

// renderDevBanner is the ready banner for `plugin dev`: how to reach the running
// hub plus what's being watched.
func renderDevBanner(b bannerInfo, mf plugin.Manifest, dir string, debounce time.Duration) string {
	var sb strings.Builder
	sb.WriteString("\nworkspacer plugin dev — watching for changes\n\n")
	rows := [][2]string{
		{"plugin", fmt.Sprintf("%s (%s)", mf.ID, pluginKind(mf))},
		{"watching", dir},
		{"debounce", debounce.String()},
		{"bus", b.BusURL},
		{"hub", b.HubURL},
		{"token", b.Token},
	}
	for _, r := range rows {
		fmt.Fprintf(&sb, "  %-10s %s\n", r[0], r[1])
	}
	sb.WriteString("\nEdit files in the plugin dir to rebuild + reload it. Press Ctrl-C to stop.\n")
	return sb.String()
}
