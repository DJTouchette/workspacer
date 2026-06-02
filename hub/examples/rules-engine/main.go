// Command rules-engine is a workspacer plugin sidecar: a tiny always-on
// event→action interpreter that sits on the hub bus. It subscribes to bus
// events (agent.state_changed, ui.*, …), evaluates a user-editable rule list,
// and fires actions — calling capabilities (notifications.post,
// agents.sendMessage), publishing commands (command.focus_agent, …), or hitting
// external webhooks (ntfy/Slack/Pushover → your phone).
//
// It also serves the rule-editor webview and a small /rules HTTP API, and
// persists rules to rules.json next to the binary. The brain lives here (not in
// the webview) so rules keep firing while the editor pane is closed.
//
// See hub/docs/rules-engine-plugin.md for the full spec.
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
	"syscall"
	"time"
)

func main() {
	port := flag.String("port", "9120", "HTTP port for the /rules API and webview")
	busURL := flag.String("bus", "ws://127.0.0.1:7895/bus", "hub bus WebSocket URL")
	pollInterval := flag.Duration("poll", 20*time.Second, "agents.list poll interval for cost/usage rules")
	flag.Parse()

	dir := binDir()
	rulesPath := filepath.Join(dir, "rules.json")
	statePath := filepath.Join(dir, "state.json")
	webDir := filepath.Join(dir, "web")

	eng := newEngine(rulesPath, statePath)
	if err := eng.load(); err != nil {
		log.Printf("rules-engine: could not load %s (%v); starting with no rules", rulesPath, err)
	} else {
		log.Printf("rules-engine: loaded %d rule(s) from %s", eng.count(), rulesPath)
	}
	eng.loadState()
	if eng.isPaused() {
		log.Printf("rules-engine: starting PAUSED (kill-switch on)")
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	// Bus client: connects (with reconnect) and drives the engine.
	bus := newBusClient(*busURL, eng)
	eng.bus = bus
	go eng.run(ctx) // evaluation worker (keeps eval off the bus read-loop)
	go bus.run(ctx)
	go eng.pollLoop(ctx, *pollInterval)

	// HTTP: health, /rules API, and the static webview editor.
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{"status": "ok", "rules": eng.count(), "paused": eng.isPaused()})
	})
	mux.HandleFunc("/rules", eng.handleRules)
	mux.HandleFunc("/state", eng.handleState)
	mux.HandleFunc("/log", eng.handleLog)
	mux.Handle("/", http.FileServer(http.Dir(webDir)))

	srv := &http.Server{Addr: "127.0.0.1:" + *port, Handler: mux}
	go func() {
		log.Printf("rules-engine: serving editor + /rules on http://127.0.0.1:%s", *port)
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("rules-engine: http server: %v", err)
		}
	}()

	<-ctx.Done()
	log.Printf("rules-engine: shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_ = srv.Shutdown(shutdownCtx)
}

// binDir returns the directory containing the running binary (the plugin dir),
// falling back to the working directory.
func binDir() string {
	exe, err := os.Executable()
	if err != nil {
		if wd, err := os.Getwd(); err == nil {
			return wd
		}
		return "."
	}
	return filepath.Dir(exe)
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
