package bus

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// makeSnapshot builds a JSON payload roughly the size of a real agent snapshot
// with `turns` conversation entries (each ~120 bytes), to measure the cost the
// migration would actually pay on a mature session.
func makeSnapshot(turns int) json.RawMessage {
	conv := make([]map[string]any, turns)
	for i := range conv {
		conv[i] = map[string]any{
			"role":      "assistant",
			"content":   "This is a representative conversation turn with some tool output and prose so the payload size is realistic for a mature session.",
			"timestamp": 1730000000000 + i,
		}
	}
	data, _ := json.Marshal(map[string]any{
		"sessionId": "sess-123", "cwd": "/home/u/proj", "ambientState": "streaming",
		"totalToolCalls": turns, "conversation": conv,
		"usage": map[string]any{"model": "claude-opus-4-8", "contextTokens": 87000, "contextLimit": 200000, "costUSD": 1.23},
	})
	return data
}

// roundTrip measures publish → deliver-over-WS for one payload size: the true
// "what does the hub hop cost" number for a snapshot of this size.
func roundTrip(b *testing.B, payload json.RawMessage) {
	srv := NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	defer hs.Close()
	wsURL := strings.Replace(hs.URL, "http://", "ws://", 1) + "/bus"
	ctx := context.Background()

	sub, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		b.Fatal(err)
	}
	defer sub.CloseNow()
	sub.SetReadLimit(8 << 20)
	_, _, _ = sub.Read(ctx) // hello
	subFrame, _ := json.Marshal(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	_ = sub.Write(ctx, websocket.MessageText, subFrame)
	_, _, _ = sub.Read(ctx) // subscribed ack

	pub, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		b.Fatal(err)
	}
	defer pub.CloseNow()
	_, _, _ = pub.Read(ctx) // hello
	pubFrame, _ := json.Marshal(Frame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Source: "workspacer", Data: payload,
	}})

	b.SetBytes(int64(len(payload)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if err := pub.Write(ctx, websocket.MessageText, pubFrame); err != nil {
			b.Fatal(err)
		}
		if _, _, err := sub.Read(ctx); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkRoundTrip_StatusOnly(b *testing.B) {
	// What an additive *lightweight* snapshot would cost (status/usage only).
	roundTrip(b, json.RawMessage(`{"sessionId":"sess-123","ambientState":"streaming","usage":{"model":"claude-opus-4-8","contextTokens":87000,"contextLimit":200000,"costUSD":1.23}}`))
}

func BenchmarkRoundTrip_FullConversation_20(b *testing.B)  { roundTrip(b, makeSnapshot(20)) }
func BenchmarkRoundTrip_FullConversation_200(b *testing.B) { roundTrip(b, makeSnapshot(200)) }

// Pure in-process fan-out cost (no WS), for reference.
func BenchmarkBrokerFanout_16subs(b *testing.B) {
	bk := broker.New()
	subs := make([]*broker.Subscription, 16)
	for i := range subs {
		subs[i] = bk.Subscribe([]string{"agent.*"})
		go func(s *broker.Subscription) {
			for range s.C {
			}
		}(subs[i])
	}
	ev := event.Envelope{Type: "agent.snapshot", Data: makeSnapshot(200)}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		bk.Publish(ev)
	}
}

// p50/p99 propagation latency at a realistic payload + cadence — the headline
// "is the hub hot path fast" number, reported as a plain test (not a bench).
func TestSnapshotLatencyDistribution(t *testing.T) {
	if testing.Short() {
		t.Skip("latency distribution skipped in -short")
	}
	srv := NewServer(broker.New())
	hs := httptest.NewServer(srv.Handler())
	defer hs.Close()
	wsURL := strings.Replace(hs.URL, "http://", "ws://", 1) + "/bus"
	ctx := context.Background()

	sub, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer sub.CloseNow()
	sub.SetReadLimit(8 << 20)
	_, _, _ = sub.Read(ctx)
	subFrame, _ := json.Marshal(Frame{Op: "subscribe", Topics: []string{"agent.*"}})
	_ = sub.Write(ctx, websocket.MessageText, subFrame)
	_, _, _ = sub.Read(ctx)

	pub, _, err := websocket.Dial(ctx, wsURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer pub.CloseNow()
	_, _, _ = pub.Read(ctx)

	payload := makeSnapshot(200) // mature session
	pubFrame, _ := json.Marshal(Frame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Source: "workspacer", Data: payload,
	}})

	const n = 2000
	lat := make([]time.Duration, 0, n)
	for i := 0; i < n; i++ {
		start := time.Now()
		if err := pub.Write(ctx, websocket.MessageText, pubFrame); err != nil {
			t.Fatal(err)
		}
		if _, _, err := sub.Read(ctx); err != nil {
			t.Fatal(err)
		}
		lat = append(lat, time.Since(start))
	}
	// crude p50/p99
	for i := 1; i < len(lat); i++ {
		for j := i; j > 0 && lat[j] < lat[j-1]; j-- {
			lat[j], lat[j-1] = lat[j-1], lat[j]
		}
	}
	p50 := lat[len(lat)*50/100]
	p99 := lat[len(lat)*99/100]
	t.Logf("snapshot (%d turns, %d bytes) propagation over hub: p50=%v p99=%v",
		200, len(payload), p50, p99)
	if p99 > 5*time.Millisecond {
		t.Errorf("p99 %v exceeds 5ms localhost budget", p99)
	}
}
