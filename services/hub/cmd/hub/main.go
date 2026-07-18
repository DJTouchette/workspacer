// Command hub is the workspacer control-plane daemon: an event bus (and, later,
// sidecar supervisor + MCP facade) that runs independently of the UI.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"log"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/claudemon"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/jobobject"
	"github.com/djtouchette/workspacer-hub/internal/layout"
	"github.com/djtouchette/workspacer-hub/internal/parentwatch"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
	"github.com/djtouchette/workspacer-hub/internal/push"
	"github.com/djtouchette/workspacer-hub/internal/sandbox"
	"github.com/djtouchette/workspacer-hub/internal/supervisor"
)

// uiDirResolver maps a plugin id to its hub-served static-UI directory.
// *plugin.Manager implements it; the indirection keeps pluginUIHandler testable.
type uiDirResolver interface {
	UIDir(id string) (string, bool)
}

// pluginUIHandler serves a webview-only plugin's static assets from its declared
// `ui` directory at /plugins/ui/<id>/…. http.Dir confines reads to that
// directory (no `..` escape), and only that subdir is exposed, so a plugin's
// manifest / .bus-token (in the dir root) are never served.
func pluginUIHandler(res uiDirResolver) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		rest := strings.TrimPrefix(r.URL.Path, "/plugins/ui/")
		id, _, _ := strings.Cut(rest, "/")
		if id == "" {
			http.NotFound(w, r)
			return
		}
		dir, ok := res.UIDir(id)
		if !ok {
			http.NotFound(w, r)
			return
		}
		http.StripPrefix("/plugins/ui/"+id+"/", http.FileServer(http.Dir(dir))).ServeHTTP(w, r)
	}
}

// pluginAdder registers/reloads a plugin by manifest. *plugin.Manager
// implements it (Add is idempotent: stop → reload → restart → re-token → emit
// plugin.loaded); the indirection keeps pluginReloadHandler testable without a
// live manager + sidecar.
type pluginAdder interface {
	Add(plugin.Manifest)
}

// pluginReloadHandler hot-reloads one plugin from its on-disk directory: it
// re-reads <dir>/plugin.json and re-Adds it. This backs `workspacer plugin
// dev`'s file-watch loop. It performs no auth of its own — the caller wraps it
// with the same token guard as the sibling /plugins/* routes.
func pluginReloadHandler(add pluginAdder) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct {
			Dir string `json:"dir"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.Dir == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing dir"})
			return
		}
		m, err := plugin.Load(filepath.Join(body.Dir, "plugin.json"))
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		add.Add(m)
		log.Printf("reloaded plugin %s from %s", m.ID, body.Dir)
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "id": m.ID})
	}
}

// defaultLayoutFile returns the path where the shared layout document is
// persisted across hub restarts: <user-config-dir>/workspacer-hub/layout.json,
// falling back to the working directory if the config dir is unavailable.
func defaultLayoutFile() string {
	dir, err := os.UserConfigDir()
	if err != nil || dir == "" {
		return "layout.json"
	}
	return filepath.Join(dir, "workspacer-hub", "layout.json")
}

// defaultPushDir returns where the VAPID keypair + push subscriptions live:
// <user-config-dir>/workspacer-hub, falling back to the working directory.
func defaultPushDir() string {
	dir, err := os.UserConfigDir()
	if err != nil || dir == "" {
		return "."
	}
	return filepath.Join(dir, "workspacer-hub")
}

func main() {
	addr := flag.String("addr", "127.0.0.1:7895", "listen address for the bus + health endpoints")
	claudemonEvents := flag.String("claudemon-events", "", "claudemon /events SSE URL to bridge onto the bus (e.g. http://127.0.0.1:7891/events)")
	pluginsDir := flag.String("plugins-dir", "", "directory of plugin subdirs (each with a plugin.json) to load + supervise")
	examplesDir := flag.String("examples-dir", "", "directory of bundled example plugins users can add from the UI (read-only catalog)")
	webappDir := flag.String("webapp-dir", os.Getenv("WORKSPACER_WEBAPP_DIR"), "directory of the built web app (dist/web) to serve at /app/ for full remote parity; empty = disabled")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "shared secret required to reach /bus + mutating routes (empty = no auth, localhost-only default)")
	tokensFile := flag.String("tokens-file", authtoken.DefaultPath(), "capability-scoped tokens file (tokens.json, minted by `workspacer token create`); empty = scoped tokens disabled")
	layoutFile := flag.String("layout-file", defaultLayoutFile(), "path to persist the shared workspace layout document (empty = memory only)")
	pushDir := flag.String("push-dir", defaultPushDir(), "directory holding the VAPID keypair + Web Push subscriptions (for the /m PWA's background notifications)")
	brainScope := flag.String("brain-scope", "off", "supervise the brain capability provider: off | full (whole surface, headless) | catalog (file-backed subset, when the desktop app owns the live caps)")
	brainBin := flag.String("brain-bin", "", "path to the brain binary to supervise; empty = auto-detect (sibling of the hub binary, then PATH)")
	claudemonURL := flag.String("claudemon", "http://127.0.0.1:7891", "claudemon API base URL the supervised brain talks to")
	flag.Parse()

	b := broker.New()
	srv := bus.NewServer(b)
	srv.SetToken(*token)
	if *token != "" {
		log.Printf("bus auth enabled (token required on /bus, /remote, /plugins/install, /plugins/remove)")
	}

	// RPC authorization is per connection: the bus tags each caller at handshake
	// (host token → trusted, per-plugin token → that plugin's declared caps,
	// scoped user token → its tier's method allowlist) and gates calls
	// accordingly. The plugin manager registers per-plugin tokens with srv below.

	// Capability-scoped user tokens (view / triage / operator), persisted next to
	// the host remote-token and minted with `workspacer token create`. The store
	// re-reads the file when it changes, so minting/revoking takes effect on the
	// next connection without restarting the hub — no minting endpoint needed.
	// The host token itself never goes through this path: it stays trusted
	// (implicit operator), which is what keeps every existing pairing working.
	if *tokensFile != "" {
		store := authtoken.NewStore(*tokensFile)
		srv.SetScopedTokenLookup(func(tok string) (bus.ScopedIdent, bool) {
			rec, ok := store.Lookup(tok)
			if !ok {
				return bus.ScopedIdent{}, false
			}
			return bus.ScopedIdent{Scope: string(rec.Scope), Methods: rec.Scope.Methods()}, true
		})
	}

	// Shared workspace layout document — the hub owns this so the desktop and
	// the web remote mirror each other (tmux-style). Registered as in-process
	// capabilities (layout.get / layout.set); changes broadcast as layout.changed.
	lay := layout.New(b, *layoutFile)
	srv.RegisterLocal("layout.get", lay.Get)
	srv.RegisterLocal("layout.set", lay.Set)
	if *layoutFile != "" {
		log.Printf("layout document persisted at %s", *layoutFile)
	}

	// Web Push: registered as in-process bus RPC (push.key / push.subscribe /
	// push.unsubscribe), so the /m PWA subscribes over its existing authed bus
	// connection. A snapshot watcher (started below) turns "needs you"
	// transitions into lock-screen notifications. Best-effort: a failure here
	// (e.g. unwritable state dir) disables push but never blocks the hub.
	pushMgr, err := push.New(*pushDir)
	if err != nil {
		log.Printf("push: disabled (%v)", err)
		pushMgr = nil
	} else {
		srv.RegisterLocal("push.key", pushMgr.RPCKey)
		srv.RegisterLocal("push.subscribe", pushMgr.RPCSubscribe)
		srv.RegisterLocal("push.unsubscribe", pushMgr.RPCUnsubscribe)
	}

	// guard wraps a mutating/sensitive route so it requires the bus token.
	guard := func(h http.HandlerFunc) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			if !srv.Authorized(r) {
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
			h(w, r)
		}
	}

	// Windows: confine ourselves (and thus the brain + every plugin sidecar we
	// spawn) in a kill-on-job-close job object, so the whole tree dies with the
	// hub no matter how the hub dies. No-op elsewhere.
	if err := jobobject.Confine(); err != nil {
		log.Printf("job object confinement unavailable (non-fatal): %v", err)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	// Self-exit if the launcher (the desktop app) dies, so we don't orphan and
	// keep port 7895 (and the supervised brain) alive. No-op when run manually.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	parentwatch.Watch(cancel)

	// Load + supervise plugins; expose their contributions at /plugins. The
	// manager registers per-plugin bus tokens with srv so capability calls are
	// scoped to what each plugin declared.
	mgr := plugin.NewManager(b, srv)
	// Sidecars launch under OS filesystem confinement. WORKSPACER_PLUGIN_SANDBOX
	// = off | best-effort (default) | enforce. Enforce refuses to start a sidecar
	// on a platform with no confinement mechanism (fail closed).
	mgr.SetSandboxMode(sandbox.ParseMode(os.Getenv("WORKSPACER_PLUGIN_SANDBOX")))
	srv.AddRoute("/plugins", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(mgr.List())
	})
	// Per-plugin bus tokens, keyed by plugin id. Token-guarded: only the trusted
	// host may read them (it injects each into the matching plugin's webview URL).
	// Never exposed on the public /plugins endpoint.
	srv.AddRoute("/plugins/tokens", guard(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(mgr.Tokens())
	}))
	// Mint an ephemeral, capability-scoped token for one open plugin pane, with
	// dynamic scopes (e.g. ${agentCwd}) bound to this pane's agent. The trusted
	// host calls this when it opens an agent-scoped plugin pane and injects the
	// returned token into that pane's webview URL — so the webview gets the
	// plugin's capabilities confined to that agent's working directory, instead
	// of the static per-plugin token (which has no dynamic filesystem reach).
	// Token-guarded: only the trusted host may mint.
	srv.AddRoute("/plugins/pane-token", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct {
			PluginID string `json:"pluginId"`
			AgentCwd string `json:"agentCwd"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.PluginID == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing pluginId"})
			return
		}
		bindings := map[string]string{}
		if body.AgentCwd != "" {
			bindings["agentCwd"] = body.AgentCwd
		}
		tok, err := mgr.PaneToken(body.PluginID, bindings)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]string{"token": tok})
	}))
	// Revoke a pane token when its pane closes. Token-guarded; idempotent.
	srv.AddRoute("/plugins/pane-token/revoke", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct {
			Token string `json:"token"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.Token == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing token"})
			return
		}
		mgr.RevokePaneToken(body.Token)
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	}))
	// Per-plugin setting VALUES (the configurable options a plugin declares in its
	// manifest). Token-guarded like /plugins/tokens — the trusted host reads the
	// merged values to inject into a pane and writes the user's changes back.
	//   GET  ?pluginId=<id>            → { values: { <key>: value, … } } (defaults + overlay)
	//   POST { pluginId, values }      → { values: <merged> }   (validated + persisted;
	//                                     a null value reverts that key to its default)
	// A write also publishes plugin.settings.changed on the bus, so sidecars and the
	// web/remote renderer see the new values without re-fetching.
	srv.AddRoute("/plugins/settings", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.Method == http.MethodGet {
			id := r.URL.Query().Get("pluginId")
			values, err := mgr.GetSettings(id)
			if err != nil {
				w.WriteHeader(http.StatusBadRequest)
				_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"values": values})
			return
		}
		var body struct {
			PluginID string         `json:"pluginId"`
			Values   map[string]any `json:"values"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.PluginID == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing pluginId"})
			return
		}
		merged, err := mgr.SetSettings(body.PluginID, body.Values)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"values": merged})
	}))
	// Static UI for webview-only plugins (no sidecar): serve a plugin's declared
	// `ui` directory at /plugins/ui/<id>/…. Unguarded like the sidecar plugins'
	// own loopback UI servers — the real boundary is /bus, which is token-scoped.
	// http.Dir confines reads to the ui directory (no `..` escape), and only that
	// subdir is served, so the plugin's manifest / .bus-token stay private.
	srv.AddRoute("/plugins/ui/", pluginUIHandler(mgr))
	// Mobile / remote-control web client. Self-contained single page that talks
	// the bus protocol over /bus. Token-guarded since it's the remote entrypoint.
	srv.AddRoute("/remote", guard(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(remoteHTML)
	}))
	// Mobile-first client (fleet / needs-you / chat / spawn) — the default phone
	// entry, and an installable PWA. Unguarded: an installed PWA launches at
	// start_url /m with no token in the URL, so the shell must load; the client
	// then gates on its stored token and the real boundary stays /bus. The HTML
	// carries no secrets.
	srv.AddRoute("/m", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(mobileHTML)
	})
	// PWA assets for /m — all public (the browser fetches them without our token).
	// manifest + SW revalidate often (so updates ship); icons long-cache.
	srv.AddRoute("/manifest.webmanifest", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/manifest+json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-cache")
		_, _ = w.Write(manifestJSON)
	})
	srv.AddRoute("/sw.js", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/javascript; charset=utf-8")
		w.Header().Set("Cache-Control", "no-cache")
		w.Header().Set("Service-Worker-Allowed", "/")
		_, _ = w.Write(swJS)
	})
	icon := func(body []byte) http.HandlerFunc {
		return func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", "image/png")
			w.Header().Set("Cache-Control", "public, max-age=86400")
			_, _ = w.Write(body)
		}
	}
	srv.AddRoute("/icon-192.png", icon(icon192))
	srv.AddRoute("/icon-512.png", icon(icon512))
	srv.AddRoute("/icon-maskable-512.png", icon(iconMaskable512))
	srv.AddRoute("/apple-touch-icon.png", icon(appleTouchIcon))
	// Static xterm assets for the remote's live terminal mirror. Unguarded:
	// they're public library code, and <script>/<link> tags can't carry the
	// bus token. Long-cache since they're content-pinned to a vendored version.
	staticAsset := func(contentType string, body []byte) http.HandlerFunc {
		return func(w http.ResponseWriter, _ *http.Request) {
			w.Header().Set("Content-Type", contentType)
			w.Header().Set("Cache-Control", "public, max-age=86400")
			_, _ = w.Write(body)
		}
	}
	srv.AddRoute("/xterm.js", staticAsset("application/javascript; charset=utf-8", xtermJS))
	srv.AddRoute("/xterm.css", staticAsset("text/css; charset=utf-8", xtermCSS))
	srv.AddRoute("/addon-fit.js", staticAsset("application/javascript; charset=utf-8", addonFitJS))
	// Full web app: the *real* React renderer (same bundle as the Electron build),
	// served from the filesystem for true remote parity. It speaks the bus over
	// /bus exactly like /remote, but renders every pane the hub backs. Same auth
	// split as /remote + xterm: the entry document is token-guarded; the hashed
	// asset bundle is public + long-cached (the real boundary is /bus). Enabled
	// only when --webapp-dir points at a build with an index.html.
	if *webappDir != "" {
		if _, err := os.Stat(filepath.Join(*webappDir, "index.html")); err == nil {
			assets := http.StripPrefix("/app/", http.FileServer(http.Dir(*webappDir)))
			srv.AddRoute("/app/", func(w http.ResponseWriter, r *http.Request) {
				rel := strings.TrimPrefix(r.URL.Path, "/app/")
				if rel == "" || rel == "index.html" {
					if !srv.Authorized(r) {
						http.Error(w, "unauthorized", http.StatusUnauthorized)
						return
					}
					w.Header().Set("Content-Type", "text/html; charset=utf-8")
					http.ServeFile(w, r, filepath.Join(*webappDir, "index.html"))
					return
				}
				w.Header().Set("Cache-Control", "public, max-age=86400")
				assets.ServeHTTP(w, r)
			})
			log.Printf("serving web app from %s at /app/", *webappDir)
		} else {
			log.Printf("--webapp-dir %s has no index.html; /app disabled (run: npm run build:renderer:web)", *webappDir)
		}
	}
	// Inspect a plugin before installing: download + read its manifest so the UI
	// can show what it is and what it requires (Go/Rust/Python/Node) up front. No
	// code is run and nothing is installed. Token-guarded like install — it makes
	// the hub fetch an arbitrary URL.
	srv.AddRoute("/plugins/inspect", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct{ URL string }
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.URL == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing url"})
			return
		}
		m, err := plugin.Inspect(body.URL)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		_ = json.NewEncoder(w).Encode(m)
	}))
	// Install a plugin from a GitHub URL: download → extract → load → supervise.
	srv.AddRoute("/plugins/install", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct{ URL string }
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.URL == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing url"})
			return
		}
		// Publish per-stage progress on the bus so the install dialog can show
		// "downloading / extracting / building" instead of a frozen button.
		progress := func(stage string) {
			b.Publish(event.New("plugin.install.progress", "hub", map[string]string{"url": body.URL, "stage": stage}))
		}
		m, err := plugin.Install(*pluginsDir, body.URL, progress)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		mgr.Add(m)
		log.Printf("installed plugin %s from %s", m.ID, body.URL)
		_ = json.NewEncoder(w).Encode(m)
	}))
	// Hot-reload a single plugin from its on-disk directory: re-read plugin.json
	// and re-Add it, which idempotently stops the old sidecar, restarts it with
	// fresh manifest/token, and emits plugin.loaded. This backs `workspacer plugin
	// dev`'s file-watch loop — the dev command POSTs the plugin's real directory
	// here after each change (and after its build step, if any). Token-guarded
	// like its /plugins/* siblings.
	//
	// The dir must contain plugin.json. mgr.Add reloads paths (sidecar cwd, ui
	// assets, per-plugin token file) from the manifest's own Dir, and plugin.Load
	// sets Dir to the directory we pass — so passing the developer's real plugin
	// dir (not the symlink the hub scanned) keeps every relative path resolving.
	srv.AddRoute("/plugins/reload", guard(pluginReloadHandler(mgr)))
	// Remove a plugin: stop its sidecar + delete its directory.
	srv.AddRoute("/plugins/remove", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct{ ID string }
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.ID == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing id"})
			return
		}
		// Remove returns the plugin dir atomically under the manager lock, which
		// eliminates the TOCTOU window that existed when List() and Remove()
		// were two separate calls.
		dir := mgr.Remove(body.ID)
		if dir != "" {
			if err := os.RemoveAll(dir); err != nil {
				log.Printf("plugins/remove: RemoveAll %s: %v", dir, err)
			}
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	}))
	// Enable/disable a plugin without uninstalling it: toggles its .disabled
	// marker and reloads it (starting/stopping the sidecar). Returns the manifest.
	srv.AddRoute("/plugins/setEnabled", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct {
			ID      string `json:"id"`
			Enabled bool   `json:"enabled"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.ID == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing id"})
			return
		}
		m, err := mgr.SetEnabled(body.ID, body.Enabled)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		log.Printf("plugin %s enabled=%v", body.ID, body.Enabled)
		_ = json.NewEncoder(w).Encode(m)
	}))
	// Catalog of bundled example plugins the user can add (read-only). Unguarded
	// like /plugins: it only lists manifests that ship inside the app.
	srv.AddRoute("/plugins/examples", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		manifests := []plugin.Manifest{}
		if *examplesDir != "" {
			manifests, _ = plugin.LoadDir(*examplesDir)
		}
		_ = json.NewEncoder(w).Encode(manifests)
	})
	// Add one bundled example by manifest id: copy it from the examples dir into
	// the writable plugins dir, run its install step, and supervise it. No
	// network — the source ships in the app. Token-guarded like /plugins/install.
	srv.AddRoute("/plugins/examples/install", guard(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		var body struct{ ID string }
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.ID == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "missing id"})
			return
		}
		if *examplesDir == "" {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "no examples directory configured"})
			return
		}
		// Resolve the example's source dir by matching manifest id.
		manifests, _ := plugin.LoadDir(*examplesDir)
		srcDir := ""
		for _, m := range manifests {
			if m.ID == body.ID {
				srcDir = m.Dir
				break
			}
		}
		if srcDir == "" {
			w.WriteHeader(http.StatusNotFound)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": "no such example: " + body.ID})
			return
		}
		m, err := plugin.InstallFromDir(*pluginsDir, srcDir)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		mgr.Add(m)
		log.Printf("added example plugin %s", m.ID)
		_ = json.NewEncoder(w).Encode(m)
	}))
	if *pluginsDir != "" {
		manifests, errs := plugin.LoadDir(*pluginsDir)
		for _, e := range errs {
			log.Printf("plugin load error: %v", e)
		}
		log.Printf("loaded %d plugin(s) from %s", len(manifests), *pluginsDir)
		mgr.AddAll(manifests)
		defer mgr.Stop()
	}

	// Supervise the brain capability provider when asked. It's a separate process
	// (the hub only routes), spawned with the hub's own bus/token/claudemon
	// settings; the supervisor restarts it on crash and SIGTERMs it on shutdown.
	if *brainScope != "off" {
		bin := resolveBrainBin(*brainBin)
		if bin == "" {
			log.Printf("brain-scope=%s but no brain binary found (pass --brain-bin, or build it with `make build-hub`); not supervising", *brainScope)
		} else {
			var env []string
			if *token != "" {
				env = append(env, "HUB_TOKEN="+*token)
			}
			brainSup := supervisor.New(supervisor.Spec{
				Name:    "brain",
				Command: bin,
				Args:    brainArgs(*addr, *claudemonURL, *brainScope),
				Env:     env,
			}, b)
			brainSup.Start()
			defer brainSup.Stop()
			log.Printf("supervising brain (scope=%s) from %s", *brainScope, bin)
		}
	}

	httpSrv := &http.Server{Addr: *addr, Handler: srv.Handler()}

	// Bridge claudemon onto the bus (the first producer) when configured.
	if *claudemonEvents != "" {
		log.Printf("bridging claudemon events from %s", *claudemonEvents)
		go claudemon.NewBridge(*claudemonEvents, b).Run(ctx)
	}

	// Watch agent snapshots and fire background Web Push on the "needs you" edge.
	if pushMgr != nil {
		go pushMgr.Watch(ctx, b)
	}

	go func() {
		log.Printf("hub listening on %s (ws://%s/bus, http://%s/health)", *addr, *addr, *addr)
		if err := httpSrv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("hub: %v", err)
		}
	}()

	<-ctx.Done()
	stop()

	log.Println("hub shutting down")
	shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = httpSrv.Shutdown(shutCtx)
}
