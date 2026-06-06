package busclient

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// fakeHub stands up a minimal bus server: it accepts a WS, greets with hello,
// and for each "call" frame replies with whatever handle returns (the test
// fills in Op/Result/Error; the id is echoed automatically).
func fakeHub(t *testing.T, handle func(f frame) frame) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ws, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		ws.SetReadLimit(1 << 20)
		ctx := r.Context()
		hello, _ := json.Marshal(frame{Op: "hello"})
		_ = ws.Write(ctx, websocket.MessageText, hello)
		for {
			_, data, err := ws.Read(ctx)
			if err != nil {
				return
			}
			var f frame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			out := handle(f)
			out.ID = f.ID
			b, _ := json.Marshal(out)
			_ = ws.Write(ctx, websocket.MessageText, b)
		}
	}))
}

func wsURL(s *httptest.Server) string {
	return strings.Replace(s.URL, "http", "ws", 1)
}

func waitReady(t *testing.T, c *Client) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if c.Ready() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("client never became ready")
}

func TestCallSuccess(t *testing.T) {
	srv := fakeHub(t, func(f frame) frame {
		// Echo the method + params back so we can assert the request shape.
		out, _ := json.Marshal(map[string]any{
			"method": f.Method,
			"params": json.RawMessage(f.Params),
		})
		return frame{Op: "result", Result: out}
	})
	defer srv.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c := New(wsURL(srv), "")
	go c.Run(ctx)
	waitReady(t, c)

	res, err := c.Call(ctx, "agents.sendMessage", map[string]string{"sessionId": "s1", "text": "hi"})
	if err != nil {
		t.Fatalf("Call: %v", err)
	}
	var got struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	if err := json.Unmarshal(res, &got); err != nil {
		t.Fatalf("unmarshal result: %v", err)
	}
	if got.Method != "agents.sendMessage" {
		t.Errorf("method = %q, want agents.sendMessage", got.Method)
	}
	if !strings.Contains(string(got.Params), `"sessionId":"s1"`) {
		t.Errorf("params = %s, want sessionId=s1", got.Params)
	}
}

func TestCallError(t *testing.T) {
	srv := fakeHub(t, func(f frame) frame {
		return frame{Op: "error", Error: "no provider for " + f.Method}
	})
	defer srv.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c := New(wsURL(srv), "")
	go c.Run(ctx)
	waitReady(t, c)

	_, err := c.Call(ctx, "agents.list", struct{}{})
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if !strings.Contains(err.Error(), "no provider") {
		t.Errorf("error = %v, want it to mention 'no provider'", err)
	}
}

func TestCallConcurrent(t *testing.T) {
	// Reply with the call's own method so each concurrent caller can verify it
	// got *its* reply and not another's — exercises id correlation.
	srv := fakeHub(t, func(f frame) frame {
		out, _ := json.Marshal(map[string]string{"echo": f.Method})
		return frame{Op: "result", Result: out}
	})
	defer srv.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c := New(wsURL(srv), "")
	go c.Run(ctx)
	waitReady(t, c)

	const n = 20
	errs := make(chan error, n)
	for i := 0; i < n; i++ {
		method := "m" + string(rune('a'+i%26))
		go func(method string) {
			res, err := c.Call(ctx, method, struct{}{})
			if err != nil {
				errs <- err
				return
			}
			var got struct{ Echo string }
			_ = json.Unmarshal(res, &got)
			if got.Echo != method {
				errs <- &mismatch{want: method, got: got.Echo}
				return
			}
			errs <- nil
		}(method)
	}
	for i := 0; i < n; i++ {
		if err := <-errs; err != nil {
			t.Fatalf("concurrent call: %v", err)
		}
	}
}

type mismatch struct{ want, got string }

func (m *mismatch) Error() string { return "reply mismatch: want " + m.want + " got " + m.got }

func TestCallNotConnected(t *testing.T) {
	// Point at a dead address; Call should give up (bounded by ctx) rather than
	// hang forever.
	c := New("ws://127.0.0.1:1/bus", "")
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	go c.Run(ctx)

	_, err := c.Call(ctx, "agents.list", struct{}{})
	if err == nil {
		t.Fatal("expected error when not connected, got nil")
	}
}

func TestCallConnectionDropFailsPending(t *testing.T) {
	// The server accepts one call, then drops the connection without replying;
	// the in-flight call must fail (ErrConnLost) instead of blocking forever.
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ws, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		ctx := r.Context()
		hello, _ := json.Marshal(frame{Op: "hello"})
		_ = ws.Write(ctx, websocket.MessageText, hello)
		_, _, _ = ws.Read(ctx) // receive the call, then sever the link
		ws.CloseNow()
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	c := New(wsURL(srv), "")
	go c.Run(ctx)
	waitReady(t, c)

	done := make(chan error, 1)
	go func() {
		_, err := c.Call(ctx, "agents.list", struct{}{})
		done <- err
	}()

	select {
	case err := <-done:
		if err == nil {
			t.Fatal("expected error after connection drop, got nil")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("Call did not return after connection drop")
	}
}

func TestNewAppendsToken(t *testing.T) {
	c := New("ws://host/bus", "secret")
	if !strings.Contains(c.url, "token=secret") {
		t.Errorf("url = %q, want token query param", c.url)
	}
	c2 := New("ws://host/bus?x=1", "s p")
	if !strings.Contains(c2.url, "&token=s+p") {
		t.Errorf("url = %q, want &token= appended and escaped", c2.url)
	}
}
