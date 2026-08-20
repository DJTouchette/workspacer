package busclient

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// trackingListener records accepted conns so the stopper can hard-close them —
// http.Server.Shutdown/Close never touch hijacked (WebSocket) connections, so
// without this the "hub died" half of the reconnect test would wait forever.
type trackingListener struct {
	net.Listener
	mu    sync.Mutex
	conns []net.Conn
}

func (l *trackingListener) Accept() (net.Conn, error) {
	c, err := l.Listener.Accept()
	if err == nil {
		l.mu.Lock()
		l.conns = append(l.conns, c)
		l.mu.Unlock()
	}
	return c, err
}

func (l *trackingListener) closeConns() {
	l.mu.Lock()
	defer l.mu.Unlock()
	for _, c := range l.conns {
		_ = c.Close()
	}
	l.conns = nil
}

// startHub serves a real bus on the given listener and returns a stopper that
// also severs every live (hijacked) connection.
func startHub(t *testing.T, ln net.Listener) func() {
	t.Helper()
	tl := &trackingListener{Listener: ln}
	srv := &http.Server{Handler: bus.NewServer(broker.New()).Handler()}
	go func() { _ = srv.Serve(tl) }()
	return func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		_ = srv.Shutdown(ctx)
		tl.closeConns()
	}
}

// publishOnce dials the hub as a separate trusted client and publishes one event.
func publishOnce(t *testing.T, ctx context.Context, busURL, topic, data string) {
	t.Helper()
	conn, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("publisher dial: %v", err)
	}
	defer conn.CloseNow()
	ev := event.New(topic, "test", json.RawMessage(data))
	out, _ := json.Marshal(map[string]any{"op": "publish", "event": ev})
	if err := conn.Write(ctx, websocket.MessageText, out); err != nil {
		t.Fatalf("publish write: %v", err)
	}
}

// TestSubscribeReceivesEventsAcrossReconnect is Phase 1's done-when: a
// busclient subscribes, observes a published event, keeps observing after the
// hub is killed and restarted on the same address (the subscription is re-sent
// on reconnect), and its pattern filter holds.
func TestSubscribeReceivesEventsAcrossReconnect(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	// A listener whose port we control, so the restarted hub has the SAME url.
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := ln.Addr().(*net.TCPAddr).Port
	busURL := "ws://127.0.0.1:" + strconv.Itoa(port) + "/bus"
	stop := startHub(t, ln)

	events := make(chan event.Envelope, 16)
	c := New(busURL, "")
	c.OnEvent(func(ev event.Envelope) { events <- ev })
	c.Subscribe("agent.*")
	go c.Run(ctx)

	waitReady := func() {
		t.Helper()
		deadline := time.Now().Add(5 * time.Second)
		for !c.Ready() {
			if time.Now().After(deadline) {
				t.Fatal("client never connected")
			}
			time.Sleep(20 * time.Millisecond)
		}
	}
	next := func() event.Envelope {
		t.Helper()
		select {
		case ev := <-events:
			return ev
		case <-time.After(5 * time.Second):
			t.Fatal("no event arrived")
			return event.Envelope{}
		}
	}

	waitReady()
	// The hub delivers to subscribers registered before the publish; a fresh
	// subscribe can race the publisher's dial, so retry the publish briefly.
	publishOnce(t, ctx, busURL, "agent.spawned", `{"sessionId":"s1"}`)
	ev := next()
	if ev.Type != "agent.spawned" || string(ev.Data) != `{"sessionId":"s1"}` {
		t.Fatalf("unexpected event: %+v", ev)
	}

	// Pattern filter: a topic outside agent.* must NOT arrive.
	publishOnce(t, ctx, busURL, "other.topic", `{}`)
	publishOnce(t, ctx, busURL, "agent.state_changed", `{"sessionId":"s1"}`)
	ev = next()
	if ev.Type != "agent.state_changed" {
		t.Fatalf("filter leak: got %q, want agent.state_changed (other.topic must be dropped)", ev.Type)
	}

	// Kill the hub, restart on the same port, and prove the subscription
	// survived the reconnect without another Subscribe call.
	stop()
	deadline := time.Now().Add(5 * time.Second)
	for c.Ready() {
		if time.Now().After(deadline) {
			t.Fatal("client never noticed the hub dying")
		}
		time.Sleep(20 * time.Millisecond)
	}
	ln2, err := net.Listen("tcp", "127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		t.Fatalf("relisten: %v", err)
	}
	stop2 := startHub(t, ln2)
	defer stop2()
	waitReady()

	publishOnce(t, ctx, busURL, "agent.spawned", `{"sessionId":"s2"}`)
	ev = next()
	if ev.Type != "agent.spawned" || string(ev.Data) != `{"sessionId":"s2"}` {
		t.Fatalf("post-reconnect event wrong: %+v", ev)
	}
}

// TestSubscribeWhileConnectedSendsImmediately covers the late-Subscribe path:
// patterns added after the connection is up take effect without a reconnect.
func TestSubscribeWhileConnectedSendsImmediately(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	busURL := "ws://" + ln.Addr().String() + "/bus"
	stop := startHub(t, ln)
	defer stop()

	events := make(chan event.Envelope, 4)
	c := New(busURL, "")
	c.OnEvent(func(ev event.Envelope) { events <- ev })
	go c.Run(ctx)
	deadline := time.Now().Add(5 * time.Second)
	for !c.Ready() {
		if time.Now().After(deadline) {
			t.Fatal("client never connected")
		}
		time.Sleep(20 * time.Millisecond)
	}

	c.Subscribe("workflow.*")
	// The subscribe frame and the publish travel over separate connections
	// with no ordering guarantee, and no fixed pre-publish sleep is long
	// enough on a loaded CI runner (100ms lost the race on Windows). So
	// publish REPEATEDLY: the claim under test is that a late Subscribe takes
	// effect WITHOUT a reconnect, and one eventually-delivered event proves
	// it — publishes that land before the subscribe are the race, not the
	// claim.
	deadline = time.Now().Add(5 * time.Second)
	for {
		publishOnce(t, ctx, busURL, "workflow.started", `{}`)
		select {
		case ev := <-events:
			if ev.Type != "workflow.started" {
				t.Fatalf("got %q", ev.Type)
			}
			return
		case <-time.After(150 * time.Millisecond):
		}
		if time.Now().After(deadline) {
			t.Fatal("late Subscribe never took effect")
		}
	}
}
