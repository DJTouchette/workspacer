// Command mcp is the workspacer MCP facade.
//
// It exposes the hub's capabilities — list / spawn / drive Claude agents and
// terminals — as MCP tools over HTTP, so Claude Code (or any MCP client) can
// drive workspacer headlessly via `--mcp-config`.
//
// It is a thin adapter: each tool call is forwarded to the hub bus as a
// capability `call`, and the provider (the Electron main process) executes it.
// The facade never touches workspacer state directly — it routes, exactly like
// the hub does. That keeps the substrate generic and the facade replaceable.
//
// Two HTTP transports are served from the same MCP server:
//
//	/mcp  — Streamable HTTP (the current MCP HTTP transport; uses SSE to stream)
//	/sse  — legacy SSE transport, for older clients
package main

import (
	"context"
	"encoding/json"
	"flag"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:7897", "HTTP listen address for the MCP server")
	hubURL := flag.String("hub", "ws://127.0.0.1:7895/bus", "workspacer hub bus WebSocket URL")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "hub bus token (when the hub requires auth)")
	flag.Parse()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	client := busclient.New(*hubURL, *token)
	go client.Run(ctx)

	server := newServer(client)
	getServer := func(*http.Request) *mcp.Server { return server }

	mux := http.NewServeMux()
	mux.Handle("/mcp", mcp.NewStreamableHTTPHandler(getServer, nil))
	mux.Handle("/sse", mcp.NewSSEHandler(getServer, nil))
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"status":       "ok",
			"hubConnected": client.Ready(),
		})
	})

	httpSrv := &http.Server{Addr: *addr, Handler: mux}
	go func() {
		log.Printf("mcp facade listening on %s (streamable: http://%s/mcp, sse: http://%s/sse)", *addr, *addr, *addr)
		log.Printf("bridging to hub %s", *hubURL)
		if err := httpSrv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("mcp: %v", err)
		}
	}()

	<-ctx.Done()
	stop()
	log.Println("mcp facade shutting down")
	shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = httpSrv.Shutdown(shutCtx)
}

// newServer wires every hub capability to an MCP tool. Names are snake_case
// (the MCP tool convention); descriptions are written for the model.
func newServer(c *busclient.Client) *mcp.Server {
	s := mcp.NewServer(&mcp.Implementation{
		Name:    "workspacer",
		Title:   "Workspacer",
		Version: "0.1.0",
	}, nil)

	// Read.
	addTool[listAgentsIn](s, c, "list_agents",
		"List running Claude Code agent sessions with their state, model, context usage, and any pending approval or question.",
		"agents.list")
	addTool[transcriptIn](s, c, "get_transcript",
		"Fetch a session's transcript so you can see the context behind a pending approval or question before acting.",
		"sessions.transcript")

	// Create.
	addTool[spawnAgentIn](s, c, "spawn_agent",
		"Start a new Claude Code agent session in a directory. Returns the new sessionId, which you can then drive with send_message, approve, answer, etc.",
		"agents.spawn")
	addTool[createTerminalIn](s, c, "create_terminal",
		"Open a new shell terminal session. Returns the new sessionId; write to it with terminal_input.",
		"terminals.create")

	// Drive.
	addTool[sendMessageIn](s, c, "send_message",
		"Send a prompt/message to a running agent session.",
		"agents.sendMessage")
	addTool[approveIn](s, c, "approve",
		"Resolve a pending permission prompt for an agent: 'yes', 'no', or 'always'.",
		"claude.approve")
	addTool[answerIn](s, c, "answer",
		"Answer an agent's AskUserQuestion picker by option number, free text, or a list of answers.",
		"claude.answer")
	addTool[signalIn](s, c, "signal",
		"Send a POSIX signal to an agent session, e.g. SIGINT to interrupt or SIGTERM to stop it.",
		"claude.signal")
	addTool[terminalInputIn](s, c, "terminal_input",
		"Type raw bytes into a session's terminal (PTY) — e.g. a command followed by a carriage return (\\r), or Ctrl-C (\\u0003).",
		"sessions.terminalInput")

	// Notify.
	addTool[notifyIn](s, c, "notify",
		"Show a desktop notification on the workspacer machine.",
		"notifications.post")

	return s
}

// addTool registers one MCP tool that forwards its typed input to a hub
// capability and returns the capability's JSON result as text. In is the tool's
// input shape (which becomes its input schema); the output is passed through
// untyped, so no output schema is advertised.
func addTool[In any](s *mcp.Server, c *busclient.Client, name, desc, method string) {
	mcp.AddTool(s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			res, err := c.Call(ctx, method, in)
			if err != nil {
				return &mcp.CallToolResult{
					IsError: true,
					Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
				}, nil, nil
			}
			text := string(res)
			if text == "" || text == "null" {
				text = "ok"
			}
			return &mcp.CallToolResult{
				Content: []mcp.Content{&mcp.TextContent{Text: text}},
			}, nil, nil
		})
}

// Tool input shapes. Field json tags must match each hub capability's expected
// params; jsonschema tags become the per-field descriptions the model sees.

type listAgentsIn struct{}

type transcriptIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
}

type spawnAgentIn struct {
	Cwd             string `json:"cwd,omitempty" jsonschema:"working directory for the new agent (defaults to the user's home)"`
	Model           string `json:"model,omitempty" jsonschema:"model id to use, e.g. claude-opus-4-8 (optional)"`
	ProfileID       string `json:"profileId,omitempty" jsonschema:"workspacer Claude profile id to use (optional)"`
	SkipPermissions bool   `json:"skipPermissions,omitempty" jsonschema:"start the agent with --dangerously-skip-permissions"`
}

type createTerminalIn struct {
	Shell string `json:"shell,omitempty" jsonschema:"shell to run (defaults to the platform default shell)"`
	Cwd   string `json:"cwd,omitempty" jsonschema:"working directory (defaults to the user's home)"`
	Cols  int    `json:"cols,omitempty" jsonschema:"initial terminal width in columns"`
	Rows  int    `json:"rows,omitempty" jsonschema:"initial terminal height in rows"`
}

type sendMessageIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Text      string `json:"text" jsonschema:"the prompt/message to send to the agent"`
}

type approveIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Decision  string `json:"decision" jsonschema:"one of: yes, no, always"`
	Reason    string `json:"reason,omitempty" jsonschema:"optional reason to record with the decision"`
}

type answerIn struct {
	SessionID string   `json:"sessionId" jsonschema:"the target session id"`
	Option    *int     `json:"option,omitempty" jsonschema:"the numeric option to pick"`
	Text      *string  `json:"text,omitempty" jsonschema:"a free-text answer"`
	Answers   []string `json:"answers,omitempty" jsonschema:"a list of answers for a multi-part question"`
}

type signalIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Signal    string `json:"signal" jsonschema:"signal name, e.g. SIGINT or SIGTERM"`
}

type terminalInputIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Data      string `json:"data" jsonschema:"raw bytes to write to the PTY"`
}

type notifyIn struct {
	Title string `json:"title,omitempty" jsonschema:"notification title"`
	Body  string `json:"body,omitempty" jsonschema:"notification body"`
}
