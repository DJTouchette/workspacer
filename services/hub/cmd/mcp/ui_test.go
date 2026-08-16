package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestUiToolsPublishCommands proves the whole navigation chain below the
// renderer: calling a UI tool publishes the right command.* envelope on the
// bus, with the arguments a subscriber (the desktop renderer) consumes.
func TestUiToolsPublishCommands(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"

	// A subscriber standing in for the desktop renderer's hub client.
	sub, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("subscriber dial: %v", err)
	}
	defer sub.CloseNow()
	subMsg, _ := json.Marshal(busFrame{Op: "subscribe", Topics: []string{"command.*"}})
	if err := sub.Write(ctx, websocket.MessageText, subMsg); err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	events := make(chan event.Envelope, 8)
	go func() {
		for {
			_, data, err := sub.Read(ctx)
			if err != nil {
				return
			}
			var f struct {
				Op    string          `json:"op"`
				Event *event.Envelope `json:"event"`
			}
			if json.Unmarshal(data, &f) == nil && f.Op == "event" && f.Event != nil {
				events <- *f.Event
			}
		}
	}()

	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client, authtoken.ScopeTriage)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	next := func() event.Envelope {
		t.Helper()
		select {
		case ev := <-events:
			return ev
		case <-ctx.Done():
			t.Fatal("no command event arrived")
			return event.Envelope{}
		}
	}

	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "focus_agent",
		Arguments: map[string]any{"sessionId": "sess-42"},
	})
	if err != nil || res.IsError {
		t.Fatalf("focus_agent failed: err=%v res=%s", err, textOf(res))
	}
	ev := next()
	if ev.Type != "command.focus_agent" || ev.Source != "mcp-facade" {
		t.Fatalf("unexpected envelope: %+v", ev)
	}
	if !strings.Contains(string(ev.Data), `"sessionId":"sess-42"`) {
		t.Errorf("focus_agent data missing sessionId: %s", ev.Data)
	}

	// open_browser is sugar over command.open_pane with paneType browser + url.
	res, err = cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "open_browser",
		Arguments: map[string]any{"url": "https://workspacer.dev/docs"},
	})
	if err != nil || res.IsError {
		t.Fatalf("open_browser failed: err=%v res=%s", err, textOf(res))
	}
	ev = next()
	if ev.Type != "command.open_pane" {
		t.Fatalf("open_browser published %q, want command.open_pane", ev.Type)
	}
	if !strings.Contains(string(ev.Data), `"paneType":"browser"`) ||
		!strings.Contains(string(ev.Data), `"url":"https://workspacer.dev/docs"`) {
		t.Errorf("open_browser data wrong: %s", ev.Data)
	}
}
