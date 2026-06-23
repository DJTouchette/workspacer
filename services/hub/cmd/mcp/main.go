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

	// ── Observe ────────────────────────────────────────────────────────────
	addTool[listAgentsIn](s, c, "list_agents",
		"List running Claude Code agent sessions with their state, model, context usage, and any pending approval or question. The lightweight fleet overview; use get_snapshot for full detail on one session.",
		"agents.list")
	addTool[transcriptIn](s, c, "get_transcript",
		"Fetch a session's transcript so you can see the context behind a pending approval or question before acting.",
		"sessions.transcript")
	addTool[sessionIn](s, c, "get_snapshot",
		"Get the full live snapshot for one session: conversation turns, tool calls, usage/cost, subagents, workflow runs, and any pending approval/question. Heavier than list_agents — use it to inspect a single agent in depth.",
		"sessions.snapshot")
	addTool[listAgentsIn](s, c, "list_snapshots",
		"Get full snapshots for every session at once (verbose — large payload). Prefer list_agents for an overview and get_snapshot for one session.",
		"sessions.snapshots")
	addTool[listAgentsIn](s, c, "list_models",
		"List the Claude models available to spawn_agent (ids + display names).",
		"claude.listModels")
	addTool[listAgentsIn](s, c, "get_host_cwd",
		"Get the workspacer host process's current working directory — a sensible default base for new agents.",
		"app.getCwd")
	addTool[cwdIn](s, c, "list_resumable_sessions",
		"List prior Claude Code sessions for a directory that can be resumed (the resume picker), newest first.",
		"claude.sessionsForDir")

	// ── Spawn ──────────────────────────────────────────────────────────────
	addTool[spawnAgentIn](s, c, "spawn_agent",
		"Start a new Claude Code agent session in a directory. Returns the new sessionId, which you can then drive with send_message, approve, answer, etc. Pass label to give the new agent a human-readable name shown in the UI, and parentSessionId (your own session id) so the new agent appears nested under you in the UI.",
		"agents.spawn")
	addTool[createTerminalIn](s, c, "create_terminal",
		"Open a new shell terminal session. Returns the new sessionId; write to it with terminal_input.",
		"terminals.create")

	// ── Drive ──────────────────────────────────────────────────────────────
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
	addTool[gateIn](s, c, "set_approval_gate",
		"Turn an agent's approval gate on or off. When on, the agent pauses for permission before running tools (surfaced via list_agents / get_snapshot for you to approve).",
		"claude.gate")
	addTool[terminalInputIn](s, c, "terminal_input",
		"Type raw bytes into a session's terminal (PTY) — e.g. a command followed by a carriage return (\\r), or Ctrl-C (\\u0003).",
		"sessions.terminalInput")
	addTool[terminalResizeIn](s, c, "terminal_resize",
		"Resize a session's PTY grid (cols × rows). The PTY is shared, so this reflows the desktop pane too.",
		"sessions.terminalResize")

	// ── Filesystem (on the workspacer host) ────────────────────────────────
	addTool[listDirIn](s, c, "list_dir",
		"List sub-directories of a host path (directories only, hidden skipped) — for choosing a working directory. Defaults to the user's home; returns { path, parent, home, dirs }.",
		"fs.listDir")
	addTool[listDirIn](s, c, "list_entries",
		"List files and directories at a host path (gitignore-aware), for an editor-style file tree.",
		"fs.listEntries")
	addTool[readFileIn](s, c, "read_file",
		"Read a UTF-8 text file on the workspacer host. Returns its contents.",
		"fs.read")
	addTool[writeFileIn](s, c, "write_file",
		"Write (create or overwrite) a UTF-8 text file on the workspacer host.",
		"fs.write")
	addTool[searchProjectIn](s, c, "search_project",
		"ripgrep a project directory for a query, returning matches grouped by file. Use for code search across the host project.",
		"search.project")

	// ── Config ─────────────────────────────────────────────────────────────
	addTool[listAgentsIn](s, c, "get_config",
		"Get the full workspacer config (theme, keybindings, pane and session settings).",
		"config.get")
	addTool[listAgentsIn](s, c, "get_config_path",
		"Get the path to the workspacer config file on the host.",
		"config.getPath")
	addTool[listAgentsIn](s, c, "reload_config",
		"Re-read the config file from disk and return it.",
		"config.reload")
	addObjectTool(s, c, "save_config",
		"Persist a partial config patch (deep-merged into the current config). Pass only the keys to change, e.g. {\"ui\":{\"guiFontScale\":1.3}}.",
		"config.save")

	// ── Claude profiles ────────────────────────────────────────────────────
	addTool[listAgentsIn](s, c, "list_profiles",
		"List configured Claude profiles (named CLAUDE_CONFIG_DIR + extra-args presets used when spawning agents).",
		"claude.profiles.list")
	addTool[addProfileIn](s, c, "add_profile",
		"Add a Claude profile. name is required; configDir and extraArgs optional.",
		"claude.profiles.add")
	addObjectTool(s, c, "update_profile",
		"Update a Claude profile. Pass { id, updates: { name?, configDir?, extraArgs? } }.",
		"claude.profiles.update")
	addTool[idIn](s, c, "remove_profile",
		"Remove a Claude profile by id.",
		"claude.profiles.remove")

	// ── Saved sessions (workspace arrangements) ────────────────────────────
	addTool[listAgentsIn](s, c, "list_saved_sessions",
		"List saved workspace sessions (the session picker — saved pane/agent arrangements).",
		"sessions.list")
	addTool[filenameIn](s, c, "load_saved_session",
		"Load one saved workspace session by filename.",
		"sessions.load")
	addObjectTool(s, c, "save_saved_session",
		"Save the current workspace arrangement. Pass the session blob ({ name, tabs|agents, ... }).",
		"sessions.save")
	addTool[filenameIn](s, c, "delete_saved_session",
		"Delete a saved workspace session by filename.",
		"sessions.delete")

	// ── Layout templates ───────────────────────────────────────────────────
	addTool[listAgentsIn](s, c, "list_layouts",
		"List saved layout templates (pane geometry presets).",
		"layouts.list")
	addObjectTool(s, c, "save_layout",
		"Save a layout template. Pass the layout blob ({ id?, name, ... }).",
		"layouts.save")
	addTool[idIn](s, c, "delete_layout",
		"Delete a layout template by id.",
		"layouts.delete")

	// ── Library (reusable prompts, skills, agents) ─────────────────────────
	addTool[cwdIn](s, c, "list_library",
		"List reusable library items (prompts, skills, agents) — global plus, if cwd is given, that project's items.",
		"library.list")
	addObjectTool(s, c, "save_library",
		"Save a library item. Pass the item blob (scope, kind, id, name, body, …).",
		"library.save")
	addTool[libraryRemoveIn](s, c, "remove_library",
		"Remove a library item. Pass { scope: 'global'|'project'|'claude', id, cwd?, kind?: 'prompt'|'skill'|'agent' }.",
		"library.remove")

	// ── Analytics ──────────────────────────────────────────────────────────
	addTool[listAgentsIn](s, c, "analytics_summary",
		"Get aggregate usage analytics across sessions (totals for tokens, cost, durations).",
		"analytics.summary")
	addTool[recentIn](s, c, "analytics_recent",
		"Get the most recent finished sessions with their per-session usage. Pass limit to cap the count.",
		"analytics.recent")

	// ── Notify ─────────────────────────────────────────────────────────────
	addTool[notifyIn](s, c, "notify",
		"Show a desktop notification on the workspacer machine.",
		"notifications.post")

	return s
}

// forward sends params to a hub capability and renders the JSON result as an MCP
// tool result. Shared by the typed and freeform tool registrars.
func forward(ctx context.Context, c *busclient.Client, method string, params any) (*mcp.CallToolResult, any, error) {
	res, err := c.Call(ctx, method, params)
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
}

// addTool registers one MCP tool that forwards its typed input to a hub
// capability and returns the capability's JSON result as text. In is the tool's
// input shape (which becomes its input schema); the output is passed through
// untyped, so no output schema is advertised.
func addTool[In any](s *mcp.Server, c *busclient.Client, name, desc, method string) {
	mcp.AddTool(s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			return forward(ctx, c, method, in)
		})
}

// addObjectTool registers a tool whose entire arguments object is forwarded
// verbatim as the capability's params. For capabilities that take a free-form
// object (a config patch, a saved-session blob) too nested to model as a typed
// struct — the schema is an open object so the model can pass any shape.
func addObjectTool(s *mcp.Server, c *busclient.Client, name, desc, method string) {
	mcp.AddTool(s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in map[string]any) (*mcp.CallToolResult, any, error) {
			return forward(ctx, c, method, in)
		})
}

// Tool input shapes. Field json tags must match each hub capability's expected
// params; jsonschema tags become the per-field descriptions the model sees.

// listAgentsIn is the empty input shared by every no-argument tool.
type listAgentsIn struct{}

type transcriptIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
}

type sessionIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
}

type cwdIn struct {
	Cwd string `json:"cwd,omitempty" jsonschema:"a project/working directory on the host"`
}

type idIn struct {
	ID string `json:"id" jsonschema:"the target id"`
}

type filenameIn struct {
	Filename string `json:"filename" jsonschema:"the saved session's filename"`
}

type gateIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	On        bool   `json:"on" jsonschema:"true to require approval before tools run, false to let the agent run freely"`
}

type terminalResizeIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Cols      int    `json:"cols" jsonschema:"terminal width in columns"`
	Rows      int    `json:"rows" jsonschema:"terminal height in rows"`
}

type listDirIn struct {
	Path string `json:"path,omitempty" jsonschema:"the host directory to list (defaults to the user's home)"`
}

type readFileIn struct {
	Path string `json:"path" jsonschema:"absolute path of the file to read on the host"`
}

type writeFileIn struct {
	Path     string `json:"path" jsonschema:"absolute path of the file to write on the host"`
	Contents string `json:"contents" jsonschema:"the new file contents"`
}

type searchProjectIn struct {
	Query         string `json:"query" jsonschema:"the search query"`
	Cwd           string `json:"cwd" jsonschema:"the project directory to search"`
	CaseSensitive bool   `json:"caseSensitive,omitempty" jsonschema:"match case (default false)"`
	WholeWord     bool   `json:"wholeWord,omitempty" jsonschema:"match whole words only (default false)"`
	Regex         bool   `json:"regex,omitempty" jsonschema:"treat the query as a regular expression (default false)"`
}

type addProfileIn struct {
	Name      string   `json:"name" jsonschema:"display name for the profile"`
	ConfigDir string   `json:"configDir,omitempty" jsonschema:"CLAUDE_CONFIG_DIR for this profile (optional)"`
	ExtraArgs []string `json:"extraArgs,omitempty" jsonschema:"extra CLI args passed to claude for this profile (optional)"`
}

type libraryRemoveIn struct {
	Scope string `json:"scope" jsonschema:"one of: global, project, claude"`
	ID    string `json:"id" jsonschema:"the library item id"`
	Cwd   string `json:"cwd,omitempty" jsonschema:"project directory (required for project/claude scope)"`
	Kind  string `json:"kind,omitempty" jsonschema:"one of: prompt, skill, agent"`
}

type recentIn struct {
	Limit int `json:"limit,omitempty" jsonschema:"max number of recent sessions to return"`
}

type spawnAgentIn struct {
	Cwd             string `json:"cwd,omitempty" jsonschema:"working directory for the new agent (defaults to the user's home)"`
	Model           string `json:"model,omitempty" jsonschema:"model id to use, e.g. claude-opus-4-8 (optional)"`
	ProfileID       string `json:"profileId,omitempty" jsonschema:"workspacer Claude profile id to use (optional)"`
	SkipPermissions bool   `json:"skipPermissions,omitempty" jsonschema:"start the agent with --dangerously-skip-permissions"`
	Label           string `json:"label,omitempty" jsonschema:"a short human label for the new agent, shown as its name in the UI"`
	ParentSessionId string `json:"parentSessionId,omitempty" jsonschema:"the spawning agent's own session id; set this so the new agent appears nested under you in the UI"`
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
