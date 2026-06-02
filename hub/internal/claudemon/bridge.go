// Package claudemon bridges the claudemon daemon onto the hub bus. It consumes
// claudemon's /events SSE stream and re-publishes each session update as an
// agent.* event, making claudemon the first producer on the bus.
package claudemon

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

const reconnectWait = time.Second

// Publisher is the slice of the broker the bridge needs.
type Publisher interface {
	Publish(event.Envelope)
}

// Bridge streams claudemon SSE and republishes onto the bus.
type Bridge struct {
	url    string
	pub    Publisher
	client *http.Client
}

// NewBridge targets a claudemon /events URL (e.g. http://127.0.0.1:7891/events).
func NewBridge(url string, pub Publisher) *Bridge {
	return &Bridge{
		url: url,
		pub: pub,
		// No client timeout: SSE is a long-lived stream.
		client: &http.Client{},
	}
}

// Run connects and republishes until ctx is cancelled, reconnecting on drop.
func (b *Bridge) Run(ctx context.Context) {
	for {
		if ctx.Err() != nil {
			return
		}
		_ = b.stream(ctx)
		if ctx.Err() != nil {
			return
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(reconnectWait):
		}
	}
}

func (b *Bridge) stream(ctx context.Context) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, b.url, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "text/event-stream")
	resp, err := b.client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	return parseSSE(ctx, resp.Body, func(name string, data []byte) {
		if ev, ok := mapEvent(name, data); ok {
			b.pub.Publish(ev)
		}
	})
}

// sessionUpdate mirrors the fields of claudemon's SessionUpdate we care about.
type sessionUpdate struct {
	SessionID string `json:"session_id"`
	Event     string `json:"event"`
	State     struct {
		Mode string `json:"mode"`
		CWD  string `json:"cwd"`
	} `json:"state"`
}

// agentState is the payload of the agent.state_changed events we emit.
type agentState struct {
	SessionID string `json:"sessionId"`
	HookEvent string `json:"hookEvent"`
	Mode      string `json:"mode"`
	CWD       string `json:"cwd,omitempty"`
}

// mapEvent translates a claudemon SSE frame into a bus envelope.
func mapEvent(name string, data []byte) (event.Envelope, bool) {
	switch name {
	case "session.update", "": // claudemon names it session.update
		var su sessionUpdate
		if err := json.Unmarshal(data, &su); err != nil || su.SessionID == "" {
			return event.Envelope{}, false
		}
		return event.New("agent.state_changed", "claudemon", agentState{
			SessionID: su.SessionID,
			HookEvent: su.Event,
			Mode:      su.State.Mode,
			CWD:       su.State.CWD,
		}), true
	default:
		return event.Envelope{}, false
	}
}

// parseSSE reads a Server-Sent Events stream, calling emit(eventName, data) for
// each complete event. Blocks until the reader is exhausted or errors (closing
// the body on ctx cancel unblocks it).
func parseSSE(ctx context.Context, r io.Reader, emit func(name string, data []byte)) error {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1<<20)
	var name string
	var data []byte
	flush := func() {
		if len(data) > 0 {
			emit(name, data)
		}
		name, data = "", nil
	}
	for sc.Scan() {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		line := sc.Text()
		switch {
		case line == "":
			flush()
		case strings.HasPrefix(line, ":"):
			// comment / heartbeat — ignore
		case strings.HasPrefix(line, "event:"):
			name = strings.TrimSpace(line[len("event:"):])
		case strings.HasPrefix(line, "data:"):
			chunk := strings.TrimPrefix(line[len("data:"):], " ")
			if data != nil {
				data = append(data, '\n')
			}
			data = append(data, chunk...)
		}
	}
	return sc.Err()
}
