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

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/claudemon"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/layout"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

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

func main() {
	addr := flag.String("addr", "127.0.0.1:7895", "listen address for the bus + health endpoints")
	claudemonEvents := flag.String("claudemon-events", "", "claudemon /events SSE URL to bridge onto the bus (e.g. http://127.0.0.1:7891/events)")
	pluginsDir := flag.String("plugins-dir", "", "directory of plugin subdirs (each with a plugin.json) to load + supervise")
	webappDir := flag.String("webapp-dir", os.Getenv("WORKSPACER_WEBAPP_DIR"), "directory of the built web app (dist/web) to serve at /app/ for full remote parity; empty = disabled")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "shared secret required to reach /bus + mutating routes (empty = no auth, localhost-only default)")
	layoutFile := flag.String("layout-file", defaultLayoutFile(), "path to persist the shared workspace layout document (empty = memory only)")
	flag.Parse()

	b := broker.New()
	srv := bus.NewServer(b)
	srv.SetToken(*token)
	if *token != "" {
		log.Printf("bus auth enabled (token required on /bus, /remote, /plugins/install, /plugins/remove)")
	}

	// RPC authorization is per connection: the bus tags each caller at handshake
	// (host token → trusted, per-plugin token → that plugin's declared caps) and
	// gates calls accordingly. The plugin manager registers per-plugin tokens
	// with srv below.

	// Shared workspace layout document — the hub owns this so the desktop and
	// the web remote mirror each other (tmux-style). Registered as in-process
	// capabilities (layout.get / layout.set); changes broadcast as layout.changed.
	lay := layout.New(b, *layoutFile)
	srv.RegisterLocal("layout.get", lay.Get)
	srv.RegisterLocal("layout.set", lay.Set)
	if *layoutFile != "" {
		log.Printf("layout document persisted at %s", *layoutFile)
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

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	// Load + supervise plugins; expose their contributions at /plugins. The
	// manager registers per-plugin bus tokens with srv so capability calls are
	// scoped to what each plugin declared.
	mgr := plugin.NewManager(b, srv)
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
	// Mobile / remote-control web client. Self-contained single page that talks
	// the bus protocol over /bus. Token-guarded since it's the remote entrypoint.
	srv.AddRoute("/remote", guard(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(remoteHTML)
	}))
	// Mobile-first client (fleet / needs-you / chat) — the default phone entry.
	// Same bus protocol + token guard as /remote.
	srv.AddRoute("/m", guard(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(mobileHTML)
	}))
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
	if *pluginsDir != "" {
		manifests, errs := plugin.LoadDir(*pluginsDir)
		for _, e := range errs {
			log.Printf("plugin load error: %v", e)
		}
		log.Printf("loaded %d plugin(s) from %s", len(manifests), *pluginsDir)
		mgr.AddAll(manifests)
		defer mgr.Stop()
	}

	httpSrv := &http.Server{Addr: *addr, Handler: srv.Handler()}

	// Bridge claudemon onto the bus (the first producer) when configured.
	if *claudemonEvents != "" {
		log.Printf("bridging claudemon events from %s", *claudemonEvents)
		go claudemon.NewBridge(*claudemonEvents, b).Run(ctx)
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
