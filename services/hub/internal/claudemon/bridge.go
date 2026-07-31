// Package claudemon bridges the claudemon daemon onto the hub bus. It consumes
// claudemon's /events SSE stream and re-publishes each session update as an
// agent.* event, making claudemon the first producer on the bus.
package claudemon

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Reconnect backoff, matching the desktop sseConsumer's 200ms→5s schedule.
const (
	reconnectInitialWait = 200 * time.Millisecond
	reconnectMaxWait     = 5 * time.Second
	// How long a stream must stay open before a drop counts as a fresh incident
	// rather than a continuation of the current backoff.
	minProductiveStream = 5 * time.Second
)

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
//
// Mirrors the desktop's sseConsumer: report the error, back off exponentially
// rather than hammering, and reset the backoff only after a connection that
// stayed open long enough to be productive. The flat one-second retry this
// replaced spun at 1 req/s forever against a daemon that was down or answering
// 404, and discarded every error on the way — so the one signal that would have
// explained a silent bridge never reached a log.
func (b *Bridge) Run(ctx context.Context) {
	backoff := reconnectInitialWait
	for {
		if ctx.Err() != nil {
			return
		}
		openedAt := time.Now()
		err := b.stream(ctx)
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			log.Printf("claudemon bridge: stream ended: %v (retry in %s)", err, backoff)
		}
		if time.Since(openedAt) >= minProductiveStream {
			// It ran long enough to have been working; treat the next failure as
			// a fresh incident rather than continuing a long backoff.
			backoff = reconnectInitialWait
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(backoff):
		}
		if backoff < reconnectMaxWait {
			backoff *= 2
			if backoff > reconnectMaxWait {
				backoff = reconnectMaxWait
			}
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
	// A non-2xx is a real failure, not a stream: parseSSE on an error body just
	// blocks until the connection drops, so the bridge looked connected while
	// republishing nothing.
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("claudemon %s: HTTP %d", b.url, resp.StatusCode)
	}
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
	// Flush any trailing event that arrived without a final blank line.
	flush()
	if err := sc.Err(); err != nil {
		log.Printf("claudemon/bridge: SSE scanner error: %v", err)
		return err
	}
	return nil
}
