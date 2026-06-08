// Command rivet-bridge is a workspacer plugin sidecar that exposes a rivet
// project (https://github.com/DJTouchette/rivet) on the hub bus.
//
// Rivet is itself an MCP server, but it speaks MCP over stdio only and is scoped
// to a single project. This bridge runs `rivet serve` as a child process, talks
// MCP (JSON-RPC 2.0) to it, and re-registers a CURATED subset of its tools as
// hub capabilities — the inverse of cmd/mcp, which exposes hub capabilities as
// MCP tools.
//
// Why a bridge instead of per-agent MCP config: this surfaces rivet's
// deterministic intelligence (recon, witness, schema) at the FLEET level, so the
// rules-engine, dashboards, the MCP facade, and "ask the fleet" supervisor
// agents can all call recon.hotspots / witness.select without each being a
// single Claude session. Per-agent rivet (MCP config on spawn) still works and
// is complementary.
//
// Deliberately NOT bridged: vaulty (secrets must not transit the bus), rally and
// context/learn (per-session, no fleet value).
//
// LIMITATION (v1): one bridge == one project. Workspacer fleets can span repos;
// multi-project routing is future work. Point this bridge at a project root via
// --project-dir or $RIVET_PROJECT_DIR.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"
)

// providedMethods is the curated allow-list of rivet tools exposed on the bus.
// Keep this in sync with the "provides" array in plugin.json.
var providedMethods = []string{
	"recon.overview",
	"recon.search",
	"recon.symbols",
	"recon.related",
	"recon.tests",
	"recon.changes",
	"recon.context",
	"recon.hotspots",
	"recon.grep",
	"witness.select",
	"witness.staged",
	"witness.since",
	"schema.overview",
	"schema.tables",
	"schema.describe",
}

func main() {
	port := flag.String("port", "9130", "HTTP port for the health check and webview")
	busURL := flag.String("bus", "ws://127.0.0.1:7895/bus", "hub bus WebSocket URL")
	projectDir := flag.String("project-dir", os.Getenv("RIVET_PROJECT_DIR"), "project root for `rivet serve` (defaults to $RIVET_PROJECT_DIR, then cwd)")
	rivetBin := flag.String("rivet", envDefault("RIVET_BIN", "rivet"), "path to the rivet binary")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "bus auth token (defaults to $HUB_TOKEN, inherited from the hub)")
	debug := flag.Bool("debug", false, "pass --debug to rivet serve and log its stderr")
	flag.Parse()

	dir := *projectDir
	if dir == "" {
		if wd, err := os.Getwd(); err == nil {
			dir = wd
		}
		log.Printf("rivet-bridge: no --project-dir/$RIVET_PROJECT_DIR set; using %s", dir)
	}
	if abs, err := filepath.Abs(dir); err == nil {
		dir = abs
	}

	allowed := map[string]bool{}
	for _, m := range providedMethods {
		allowed[m] = true
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	rivet := newRivetClient(*rivetBin, dir, *debug)

	// Provider handler: validate against the allow-list, then forward to rivet.
	// Defence in depth — the hub only routes our registered methods, but a
	// bridge bug shouldn't let arbitrary tool names reach rivet.
	handler := func(hctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
		if !allowed[method] {
			return nil, fmt.Errorf("rivet-bridge: method %q not provided", method)
		}
		return rivet.callTool(hctx, method, params)
	}

	bus := newBusClient(*busURL, *token, providedMethods, handler)

	// Bridge rivet lifecycle onto the bus so the fleet sees availability.
	rivet.onReady = func(initResult json.RawMessage) {
		bus.publish("rivet.ready", mustJSON(map[string]any{"projectDir": dir}))
	}
	rivet.onDown = func(err error) {
		msg := ""
		if err != nil {
			msg = err.Error()
		}
		bus.publish("rivet.down", mustJSON(map[string]any{"projectDir": dir, "error": msg}))
	}

	go rivet.run(ctx)
	go bus.run(ctx)

	// HTTP: health for the supervisor, a small /api/call proxy for the webview,
	// and the static pane.
	webDir := filepath.Join(binDir(), "web")
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{
			"status":     "ok",
			"rivetReady": rivet.isReady(),
			"projectDir": dir,
		})
	})
	mux.HandleFunc("/api/call", func(w http.ResponseWriter, req *http.Request) {
		handleAPICall(w, req, rivet, allowed)
	})
	mux.Handle("/", http.FileServer(http.Dir(webDir)))

	srv := &http.Server{Addr: "127.0.0.1:" + *port, Handler: mux}
	go func() {
		log.Printf("rivet-bridge: serving health + pane on http://127.0.0.1:%s (project %s)", *port, dir)
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("rivet-bridge: http server: %v", err)
		}
	}()

	<-ctx.Done()
	log.Printf("rivet-bridge: shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_ = srv.Shutdown(shutdownCtx)
}

// handleAPICall lets the webview invoke a tool over HTTP:
// POST /api/call  {"method":"recon.hotspots","args":["--limit","20"]}
func handleAPICall(w http.ResponseWriter, req *http.Request, rivet *rivetClient, allowed map[string]bool) {
	if req.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}
	var body struct {
		Method string   `json:"method"`
		Args   []string `json:"args"`
	}
	data, _ := io.ReadAll(io.LimitReader(req.Body, 1<<20))
	if err := json.Unmarshal(data, &body); err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]any{"error": "bad json"})
		return
	}
	if !allowed[body.Method] {
		writeJSON(w, http.StatusBadRequest, map[string]any{"error": "method not provided: " + body.Method})
		return
	}
	args := mustJSON(map[string]any{"args": body.Args})
	ctx, cancel := context.WithTimeout(req.Context(), 60*time.Second)
	defer cancel()
	res, err := rivet.callTool(ctx, body.Method, args)
	if err != nil {
		writeJSON(w, http.StatusBadGateway, map[string]any{"error": err.Error()})
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_, _ = w.Write(res)
}

// binDir returns the directory containing the running binary (the plugin dir),
// falling back to the working directory.
func binDir() string {
	exe, err := os.Executable()
	if err != nil {
		if wd, werr := os.Getwd(); werr == nil {
			return wd
		}
		return "."
	}
	return filepath.Dir(exe)
}

func envDefault(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
