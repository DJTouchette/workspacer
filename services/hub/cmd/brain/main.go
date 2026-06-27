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
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"
)

func main() {
	hubURL := flag.String("hub", envOr("HUB_BUS_URL", "ws://127.0.0.1:7895/bus"), "hub bus WebSocket URL")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "hub bus auth token (empty = no auth)")
	claudemonURL := flag.String("claudemon", envOr("WKS_CLAUDEMON_URL", "http://127.0.0.1:7891"), "claudemon API base URL")
	flag.Parse()

	reg := newRegistry(newClaudemonClient(*claudemonURL))
	bus := newBusClient(*hubURL, *token, reg.methods(), reg.handle)

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	log.Printf("brain: provider for %d capabilities → hub %s, claudemon %s",
		len(reg.methods()), *hubURL, *claudemonURL)
	bus.run(ctx)
	log.Printf("brain: shutting down")
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
