package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// conversationRig is a fake claudemon /conversation/stream plus a recording
// publish sink. Each SSE connection immediately serves one delta for s1 and one
// for s2, then holds open — so a consumer that (re)connects at any point still
// sees both sessions' traffic.
type conversationRig struct {
	srv   *httptest.Server
	conns atomic.Int32

	mu        sync.Mutex
	published []struct {
		topic string
		data  string
	}
	notify chan struct{}
}

func newConversationRig(t *testing.T) *conversationRig {
	t.Helper()
	rig := &conversationRig{notify: make(chan struct{}, 64)}
	mux := http.NewServeMux()
	mux.HandleFunc("/conversation/stream", func(w http.ResponseWriter, r *http.Request) {
		rig.conns.Add(1)
		defer rig.conns.Add(-1)
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, "event: conversation.delta\ndata: "+
			`{"session_id":"s1","seq":2,"reset":false,"items":[{"kind":"assistant_text","text":"hi"}]}`+"\n\n")
		io.WriteString(w, "event: conversation.delta\ndata: "+
			`{"session_id":"s2","seq":1,"reset":false,"items":[{"kind":"assistant_text","text":"secret"}]}`+"\n\n")
		if fl, ok := w.(http.Flusher); ok {
			fl.Flush()
		}
		<-r.Context().Done()
	})
	rig.srv = httptest.NewServer(mux)
	t.Cleanup(rig.srv.Close)
	return rig
}

func (r *conversationRig) publish(topic string, data json.RawMessage) {
	r.mu.Lock()
	r.published = append(r.published, struct {
		topic string
		data  string
	}{topic, string(data)})
	r.mu.Unlock()
	select {
	case r.notify <- struct{}{}:
	default:
	}
}

// waitFor blocks until pred is true of the published list, or fails the test.
func (r *conversationRig) waitFor(t *testing.T, what string, pred func(published []struct{ topic, data string }) bool) {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for {
		r.mu.Lock()
		snapshot := make([]struct{ topic, data string }, len(r.published))
		for i, p := range r.published {
			snapshot[i] = struct{ topic, data string }{p.topic, p.data}
		}
		r.mu.Unlock()
		if pred(snapshot) {
			return
		}
		select {
		case <-r.notify:
		case <-deadline:
			t.Fatalf("timed out waiting for %s; published so far: %+v", what, snapshot)
		}
	}
}

func TestConversationForwardsDemandedSessionsOnly(t *testing.T) {
	rig := newConversationRig(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	visible := func(id string) bool { return id != "hidden" }
	hub := newConversationHub(ctx, newClaudemonClient(rig.srv.URL), rig.publish, visible)

	hub.setDemand("s1", true)

	// The ready handshake goes out synchronously on the first demand — it is
	// the client's proof that the whole push path exists — then the SSE
	// consumer's s1 delta follows VERBATIM. s2 is on claudemon's fleet-wide
	// stream but nobody demanded it: not a byte.
	rig.waitFor(t, "ready + s1 delta", func(p []struct{ topic, data string }) bool {
		return len(p) >= 2
	})

	rig.mu.Lock()
	defer rig.mu.Unlock()
	if rig.published[0].topic != "agent.conversation.s1" {
		t.Fatalf("first publish topic = %q, want agent.conversation.s1", rig.published[0].topic)
	}
	var ready struct {
		SessionID string `json:"session_id"`
		Ready     bool   `json:"ready"`
	}
	if err := json.Unmarshal([]byte(rig.published[0].data), &ready); err != nil || !ready.Ready || ready.SessionID != "s1" {
		t.Fatalf("first publish is not the ready handshake: %s", rig.published[0].data)
	}
	if rig.published[1].topic != "agent.conversation.s1" {
		t.Fatalf("delta topic = %q, want agent.conversation.s1", rig.published[1].topic)
	}
	want := `{"session_id":"s1","seq":2,"reset":false,"items":[{"kind":"assistant_text","text":"hi"}]}`
	if rig.published[1].data != want {
		t.Fatalf("delta not forwarded verbatim:\n got %s\nwant %s", rig.published[1].data, want)
	}
	for _, p := range rig.published {
		if p.topic == "agent.conversation.s2" {
			t.Fatalf("an undemanded session's delta was forwarded: %+v", p)
		}
	}
}

func TestConversationStreamRunsOnlyWhileDemanded(t *testing.T) {
	rig := newConversationRig(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	hub := newConversationHub(ctx, newClaudemonClient(rig.srv.URL), rig.publish, func(string) bool { return true })

	// An idle fleet holds no connection to claudemon at all.
	if n := rig.conns.Load(); n != 0 {
		t.Fatalf("consumer running before any demand: %d conns", n)
	}

	hub.setDemand("s1", true)
	waitCond(t, "SSE consumer started", func() bool { return rig.conns.Load() == 1 })

	// A second demanded session shares the ONE fleet-wide stream.
	hub.setDemand("s2", true)
	time.Sleep(50 * time.Millisecond)
	if n := rig.conns.Load(); n != 1 {
		t.Fatalf("want 1 shared SSE conn for 2 sessions, got %d", n)
	}

	// Releasing one keeps it; releasing the last stops it.
	hub.setDemand("s1", false)
	time.Sleep(50 * time.Millisecond)
	if n := rig.conns.Load(); n != 1 {
		t.Fatalf("stream stopped while s2 still demanded: %d conns", n)
	}
	hub.setDemand("s2", false)
	waitCond(t, "SSE consumer stopped", func() bool { return rig.conns.Load() == 0 })
}

func TestConversationResetForgetsDemand(t *testing.T) {
	rig := newConversationRig(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	hub := newConversationHub(ctx, newClaudemonClient(rig.srv.URL), rig.publish, func(string) bool { return true })

	hub.setDemand("s1", true)
	waitCond(t, "SSE consumer started", func() bool { return rig.conns.Load() == 1 })

	// A bus reconnect resets: demand is a fact about live subscriptions on the
	// hub, not ours to remember across a disconnect. The consumer must stop —
	// or a client that vanished while we were gone keeps costing stream bytes.
	hub.reset()
	waitCond(t, "SSE consumer stopped on reset", func() bool { return rig.conns.Load() == 0 })

	// And the hub's replay re-establishes it from scratch: the session is
	// FRESH again, so the ready handshake goes out again (the client also
	// dropped to its tick and is waiting for exactly that proof).
	before := len(rig.publishedSnapshot())
	hub.setDemand("s1", true)
	rig.waitFor(t, "ready after reset+replay", func(p []struct{ topic, data string }) bool {
		return len(p) > before
	})
}

func TestConversationHiddenSessionIsRefused(t *testing.T) {
	rig := newConversationRig(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	// Same rule as the snapshot and the status line: fleet visibility gates the
	// publish, and a nil rule fails CLOSED.
	hub := newConversationHub(ctx, newClaudemonClient(rig.srv.URL), rig.publish, func(string) bool { return false })

	hub.setDemand("s1", true)
	waitCond(t, "SSE consumer started", func() bool { return rig.conns.Load() == 1 })
	// Give the delta time to arrive and be (correctly) dropped.
	time.Sleep(150 * time.Millisecond)
	if p := rig.publishedSnapshot(); len(p) != 0 {
		t.Fatalf("a hidden session's transcript reached the bus: %+v", p)
	}
}

func (r *conversationRig) publishedSnapshot() []struct{ topic, data string } {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]struct{ topic, data string }, len(r.published))
	for i, p := range r.published {
		out[i] = struct{ topic, data string }{p.topic, p.data}
	}
	return out
}

func waitCond(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}
