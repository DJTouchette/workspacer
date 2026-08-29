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
	"github.com/djtouchette/workspacer-hub/internal/redact"
)

func main() {
	hubURL := flag.String("hub", envOr("HUB_BUS_URL", "ws://127.0.0.1:7895/bus"), "hub bus WebSocket URL")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "hub bus auth token (empty = no auth)")
	claudemonURL := flag.String("claudemon", envOr("WKS_CLAUDEMON_URL", "http://127.0.0.1:7891"), "claudemon API base URL")
	mcpFacadeURL := flag.String("mcp-facade", os.Getenv("WKS_MCP_FACADE_URL"), "workspacer MCP facade URL to inject into spawned sessions (empty = disabled)")
	scope := flag.String("scope", envOr("WKS_BRAIN_SCOPE", "full"), "capability scope: full (everything, headless) | catalog (file-backed subset, run alongside the desktop app)")
	flag.Parse()

	cm := newClaudemonClient(*claudemonURL)
	reg := newRegistry(cm)
	reg.scope = *scope
	reg.mcpFacadeURL = *mcpFacadeURL
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
			return enrichAndCompat(snap, meta)
		}
		// The shared desktop fleet-visibility rule (visibility.go), backed by the
		// hub-local layout document. It gates both the list/snapshot reads and
		// every agent.snapshot publish — a hidden stopped session's update must
		// not resurrect it on a client.
		vis := newVisibility(func(ctx context.Context) (json.RawMessage, error) {
			return bus.call(ctx, "layout.get", map[string]any{})
		}, 5*time.Second)
		reg.vis = vis
		// The worker-finished wake (finishwake.go): the ONE thing that makes a
		// headless Fleet Manager work, because its doctrine is never to poll.
		// It is fed BEFORE the visibility filter on purpose — a session the
		// shared layout hides is still a dispatch that came home, and hiding a
		// row from a sidebar is not a reason to deny its manager the report.
		fin := newFinishWatcher(reg)
		reg.fin = fin
		store.onSeed = fin.prime
		store.onChange = func(_ string, snap json.RawMessage) {
			fin.observe(ctx, snap)
			if !vis.visible(context.Background(), snap) {
				return
			}
			bus.publish("agent.snapshot", snap)
		}
		reg.store = store
		go runSessionStore(ctx, cm, store)
		// Live cost/context: follow the high-frequency statusline stream and push
		// a light `agent.statusline` event (sessionId + the status line) per tick.
		go runStatusLines(ctx, cm, store, visibleStatusLinePublisher(store, vis, bus.publish))
		// PTY-over-bus: lease-gated terminal forwarders republishing claudemon's
		// byte stream as pty.bytes.<sessionId> events.
		term := newTerminalHub(cm, bus.publish)
		reg.term = term
		go term.sweep(ctx)
		// The transcript delta feed, forwarded only for the sessions the hub
		// reports demand for (conversation.go). This is what lets a web client
		// stop re-downloading a growing reply on a clock; a hub too old to
		// speak the demand op simply never asks for anything here.
		conv := newConversationHub(ctx, cm, bus.publish, visibleSessionRule(store, vis))
		bus.demandPrefixes = []string{conversationTopicPrefix}
		bus.resetDemand = conv.reset
		bus.onDemand = func(topic string, wanted bool) {
			if len(topic) <= len(conversationTopicPrefix) || topic[:len(conversationTopicPrefix)] != conversationTopicPrefix {
				return
			}
			conv.setDemand(topic[len(conversationTopicPrefix):], wanted)
		}
		// terminals.open asks a CLIENT to open a visible terminal pane; it has
		// no pane of its own to open, so the bus is its only way to be answered.
		reg.publish = bus.publish
		// agents.notifyWhen's one-shot watches are evaluated on a sweep rather
		// than on every snapshot push — a threshold on spend is not a real-time
		// signal, and a sweep cannot be starved by a chatty session.
		go reg.runThresholdSweeps(ctx)
		// The catch-up backstop for a wake that never landed — a brain restart,
		// a dropped /events edge, a delivery that threw. Without it a manager
		// that misses one wake stays dark until a human notices.
		go fin.runBackstop(ctx)
	}

	facade := "disabled"
	if *mcpFacadeURL != "" {
		facade = redact.URL(*mcpFacadeURL)
	}
	log.Printf("brain: scope=%s, provider for %d capabilities → hub %s, claudemon %s, mcpFacade %s",
		*scope, len(methods), redact.URL(*hubURL), *claudemonURL, facade)
	bus.run(ctx)
	log.Printf("brain: shutting down")
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
