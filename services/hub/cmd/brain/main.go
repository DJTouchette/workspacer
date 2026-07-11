// Command brain is workspacer's headless capability provider. It connects to
// the hub bus as a PROVIDER and registers the "spawn + drive + observe" agent
// capabilities, backing each by claudemon's HTTP API plus profile/argv logic.
//
// These are the same capabilities the Electron app registers in
// hubCapabilities.ts — but provided headlessly, so the MCP facade, the web
// client, and (in time) the TUI get the full surface WITHOUT the desktop app
// running. The hub never executes a capability; it routes a caller's `call` to
// whichever provider registered the method, and this is that provider.
//
//	hub first, claudemon running, then:
//	  go run ./cmd/brain --hub ws://127.0.0.1:7895/bus --claudemon http://127.0.0.1:7891
package main

import (
	"context"
	"encoding/json"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/parentwatch"
)

func main() {
	hubURL := flag.String("hub", envOr("HUB_BUS_URL", "ws://127.0.0.1:7895/bus"), "hub bus WebSocket URL")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "hub bus auth token (empty = no auth)")
	claudemonURL := flag.String("claudemon", envOr("WKS_CLAUDEMON_URL", "http://127.0.0.1:7891"), "claudemon API base URL")
	scope := flag.String("scope", envOr("WKS_BRAIN_SCOPE", "full"), "capability scope: full (everything, headless) | catalog (file-backed subset, run alongside the desktop app)")
	flag.Parse()

	cm := newClaudemonClient(*claudemonURL)
	reg := newRegistry(cm)
	methods := reg.methodsForScope(*scope)
	bus := newBusClient(*hubURL, *token, methods, reg.handle)

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	// Self-exit if our launcher (the hub supervisor) dies, so a force-killed hub
	// doesn't leave us orphaned on the bus. No-op when run manually.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	parentwatch.Watch(cancel)

	// In full scope the brain owns the live agent view: a session store fed by
	// claudemon's /events stream, answering agents.list / sessions.snapshot* and
	// pushing each change to the bus as an `agent.snapshot` event. (In catalog
	// scope the desktop app owns this, so we skip it.)
	if *scope != "catalog" {
		meta := newMetaStore()
		reg.meta = meta
		store := newSessionStore()
		// Enrich (name/parent/supervisor) then overlay the desktop snapshot field
		// names (sessionId/status/ambientState/…, marked sparse) so /m and the
		// web renderer can read brain-served rows.
		store.enrich = func(snap json.RawMessage) json.RawMessage {
			return compatSnapshot(enrichSnapshot(snap, meta))
		}
		// The shared desktop fleet-visibility rule (visibility.go), backed by the
		// hub-local layout document. It gates both the list/snapshot reads and
		// every agent.snapshot publish — a hidden stopped session's update must
		// not resurrect it on a client.
		vis := newVisibility(func(ctx context.Context) (json.RawMessage, error) {
			return bus.call(ctx, "layout.get", map[string]any{})
		}, 5*time.Second)
		reg.vis = vis
		store.onChange = func(_ string, snap json.RawMessage) {
			if !vis.visible(context.Background(), snap) {
				return
			}
			bus.publish("agent.snapshot", snap)
		}
		reg.store = store
		go runSessionStore(ctx, cm, store)
		// Live cost/context: follow the high-frequency statusline stream and push
		// a light `agent.statusline` event (sessionId + the status line) per tick.
		go runStatusLines(ctx, cm, store, func(id string, sl json.RawMessage) {
			payload, err := json.Marshal(map[string]any{"sessionId": id, "statusLine": sl})
			if err == nil {
				bus.publish("agent.statusline", payload)
			}
		})
		// PTY-over-bus: lease-gated terminal forwarders republishing claudemon's
		// byte stream as pty.bytes.<sessionId> events.
		term := newTerminalHub(cm, bus.publish)
		reg.term = term
		go term.sweep(ctx)
	}

	log.Printf("brain: scope=%s, provider for %d capabilities → hub %s, claudemon %s",
		*scope, len(methods), *hubURL, *claudemonURL)
	bus.run(ctx)
	log.Printf("brain: shutting down")
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
