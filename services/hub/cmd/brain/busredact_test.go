package main

import (
	"bytes"
	"context"
	"log"
	"net"
	"strings"
	"testing"
	"time"
)

// The brain's reconnect line was observed on a Fly node carrying the node's
// whole HUB_TOKEN, because coder/websocket quotes the request URL back inside
// its dial error and dialURL() puts the token in that URL as ?token=.
//
// This drives the REAL path — busClient.run against a dead port — and asserts
// the token reaches no log line, so a future refactor that logs `err` instead
// of the redacted value fails here rather than on a production node.
func TestBusReconnectLogNeverCarriesTheToken(t *testing.T) {
	const token = "9f3c1ab27d5e40b8a6c2f10d4e7b83aa"

	// A port nothing listens on: bind one, then release it, so the dial fails
	// immediately (connection refused) instead of waiting out the timeout.
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	addr := l.Addr().String()
	l.Close()

	var buf bytes.Buffer
	old := log.Writer()
	flags := log.Flags()
	log.SetOutput(&buf)
	log.SetFlags(0)
	t.Cleanup(func() { log.SetOutput(old); log.SetFlags(flags) })

	b := newBusClient("ws://"+addr+"/bus", token, []string{"noop"}, nil)

	// Sanity: the dial URL really does carry the credential, so a passing test
	// means the redaction worked — not that there was nothing to redact.
	if !strings.Contains(b.dialURL(), token) {
		t.Fatalf("precondition failed: dial URL has no token: %s", b.dialURL())
	}

	ctx, cancel := context.WithTimeout(context.Background(), 400*time.Millisecond)
	defer cancel()
	b.run(ctx) // returns when ctx expires, after at least one failed attempt

	out := buf.String()
	if !strings.Contains(out, "bus disconnected") {
		t.Fatalf("expected a reconnect log line, got: %q", out)
	}
	if strings.Contains(out, token) {
		t.Fatalf("HUB_TOKEN leaked into the log: %s", out)
	}
	if !strings.Contains(out, "token=REDACTED") {
		t.Fatalf("dial URL was logged without the redaction marker: %s", out)
	}
	if !strings.Contains(out, addr) {
		t.Fatalf("redaction destroyed the diagnostic (host missing): %s", out)
	}
}

// session() is where a tokened URL enters an error; the scrub belongs there so
// every caller inherits it, not only the one log site observed leaking.
func TestSessionDialErrorIsRedactedAtTheSource(t *testing.T) {
	const token = "deadbeefdeadbeefdeadbeefdeadbeef"

	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	addr := l.Addr().String()
	l.Close()

	b := newBusClient("ws://"+addr+"/bus", token, []string{"noop"}, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	err = b.session(ctx)
	if err == nil {
		t.Fatal("expected the dial to fail against a closed port")
	}
	if strings.Contains(err.Error(), token) {
		t.Fatalf("session() returned an error carrying the token: %v", err)
	}
}
