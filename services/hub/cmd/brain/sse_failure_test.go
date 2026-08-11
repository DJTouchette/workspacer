package main

// In `--brain-scope full` — every `workspacer serve`, remote, web, mobile and
// MCP deployment — this brain is the SOLE provider of agents.list,
// sessions.snapshot* and the agent.snapshot / agent.statusline publishes. All of
// it is fed by two SSE streams from claudemon. A non-2xx answer on either
// (claudemon's host_guard 403, a 404 after a route rename, any 5xx) is a
// PERMANENT failure, and handing the error body to parseSSE returned nil — "the
// stream ended normally" — so the whole live-agent plane stayed empty forever
// with zero bytes of diagnosis.
//
// The two sibling copies of this consumer already check the status:
// internal/claudemon/bridge.go and apps/desktop/src/main/lib/sseConsumer.ts.

import (
	"bytes"
	"context"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func captureBrainLog(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	prevOut, prevFlags := log.Writer(), log.Flags()
	log.SetOutput(&buf)
	log.SetFlags(0)
	t.Cleanup(func() {
		log.SetOutput(prevOut)
		log.SetFlags(prevFlags)
	})
	return &buf
}

func TestANon2xxSSEAnswerIsAnErrorNotACleanStream(t *testing.T) {
	for _, code := range []int{http.StatusForbidden, http.StatusNotFound, http.StatusBadGateway} {
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			http.Error(w, "host not allowed", code)
		}))
		cm := newClaudemonClient(srv.URL)

		if err := cm.streamEvents(t.Context(), func(string, []byte) {}); err == nil {
			t.Errorf("HTTP %d on /events was reported as a clean stream end — a permanent failure reconnects forever with the fleet empty and nothing logged", code)
		} else if !strings.Contains(err.Error(), "/events") {
			t.Errorf("HTTP %d: error %q does not name the endpoint", code, err)
		}
		if err := cm.streamStatusLines(t.Context(), func(string, []byte) {}); err == nil {
			t.Errorf("HTTP %d on /statusline/stream was reported as a clean stream end", code)
		}
		srv.Close()
	}
}

func TestAPermanentClaudemonFailureReachesTheLog(t *testing.T) {
	var hits int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		hits++
		http.Error(w, "host not allowed", http.StatusForbidden)
	}))
	defer srv.Close()

	buf := captureBrainLog(t)
	ctx, cancel := context.WithTimeout(t.Context(), 1500*time.Millisecond)
	defer cancel()

	done := make(chan struct{})
	go func() {
		runSessionStore(ctx, newClaudemonClient(srv.URL), newSessionStore())
		close(done)
	}()
	<-done

	out := buf.String()
	if hits == 0 {
		t.Fatal("the consumer never reached the server")
	}
	if !strings.Contains(out, "/events stream ended") {
		t.Fatalf("continuous 403s produced no diagnosis at all (%d requests). log was: %q", hits, out)
	}
	if !strings.Contains(out, "403") {
		t.Fatalf("the log line does not carry the status that explains it: %q", out)
	}
}

func TestAFailedSeedIsNotMistakenForAnEmptyFleet(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "nope", http.StatusInternalServerError)
	}))
	defer srv.Close()

	buf := captureBrainLog(t)
	seedStore(t.Context(), newClaudemonClient(srv.URL), newSessionStore())
	if !strings.Contains(buf.String(), "could not seed the session store") {
		t.Fatalf("a failed seed is indistinguishable from 'no agents are running'. log was: %q", buf.String())
	}
}

// A 200 stream that simply ends is a normal disconnect, not an error: the
// reconnect loop must not spam the log on every ordinary restart.
func TestACleanStreamEndIsStillQuiet(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	if err := newClaudemonClient(srv.URL).streamEvents(t.Context(), func(string, []byte) {}); err != nil {
		t.Fatalf("a clean 200 stream that ended was reported as an error: %v", err)
	}
}
