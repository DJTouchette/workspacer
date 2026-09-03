// UI navigation tools.
//
// These drive the desktop renderer for the user — focus an agent, open a pane
// (including the built-in browser on a URL), open a plugin pane, pre-fill the
// new-agent dialog. They ride the existing `command.*` event family the
// renderer already consumes (useUiCommands.ts), published fire-and-forget on
// the bus — the same mechanism plugin panes use, so there is nothing renderer-
// side that is facade-specific.
//
// Unlike every other facade tool, these are EVENT-backed (a publish, not a
// capability call), so their tier cannot be derived from the authtoken method
// allowlists. The gate is explicit instead: triage and up. Steering the user's
// screen is an attention action — a read-only view worker has no business
// grabbing focus — and that decision is pinned by tiers_test.go.
//
// They are also desktop-only by nature: on a headless hub nothing consumes
// command.* and a publish is a silent no-op (fire-and-forget has no reply to
// say so). The help text tells the model to prefer notify unless the user
// asked to be shown around.
package main

import (
	"context"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// tierRank orders the scopes so event-backed tools can declare a minimum tier.
func tierRank(s authtoken.Scope) int {
	switch s {
	case authtoken.ScopeView:
		return 0
	case authtoken.ScopeTriage:
		return 1
	case authtoken.ScopeOperator:
		return 2
	}
	return -1 // unknown scope ranks below everything — fail closed
}

// uiEventSource is the Source stamped on facade-published command events.
const uiEventSource = "mcp-facade"

// addUiTool registers one command-event tool if the build's tier is at least
// minTier. Each tool supplies its envelope via `ev` with the topic as a
// LITERAL at its event.New site — that is what capspec's publish-site scanner
// reads, so every command topic stays classified in eventtopics.go. The
// handler publishes and reports "ok" — fire-and-forget is the command.*
// contract (the renderer's resulting ui.* activity is the implicit
// confirmation), so "ok" means "sent", not "the UI did it".
func addUiTool[In any](b *build, minTier authtoken.Scope, name, desc, topic string, ev func(In) event.Envelope) {
	if tierRank(b.scope) < tierRank(minTier) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: "publish " + topic, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			if err := b.c.Publish(ctx, ev(in)); err != nil {
				return &mcp.CallToolResult{
					IsError: true,
					Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
				}, nil, nil
			}
			return &mcp.CallToolResult{
				Content: []mcp.Content{&mcp.TextContent{Text: "ok"}},
			}, nil, nil
		})
}

// addUiTools registers the navigation group on a tier's server.
func addUiTools(b *build) {
	addUiTool(b, authtoken.ScopeTriage, "focus_agent",
		"Bring one agent session to the front of the workspacer desktop UI (switches the user's view to it).",
		"command.focus_agent",
		func(in focusAgentIn) event.Envelope {
			return event.New("command.focus_agent", uiEventSource, in)
		})
	addUiTool(b, authtoken.ScopeTriage, "open_pane",
		"Open a workspacer pane in the desktop UI by type — e.g. settings, analytics, library, overview, sessions, review, terminal, browser.",
		"command.open_pane",
		func(in openPaneIn) event.Envelope {
			return event.New("command.open_pane", uiEventSource, in)
		})
	addUiTool(b, authtoken.ScopeTriage, "open_browser",
		"Open workspacer's built-in browser pane on a URL, shown to the user in the desktop UI. "+
			"A file:// URL works too when it points at a file under the user's home or a configured "+
			"project directory; a markdown file opens in the preview pane instead.",
		"command.open_pane",
		func(in openBrowserIn) event.Envelope {
			return event.New("command.open_pane", uiEventSource, openPaneIn{PaneType: "browser", URL: in.URL})
		})
	addUiTool(b, authtoken.ScopeTriage, "open_plugin",
		"Open an installed plugin's pane in the desktop UI by its pane type (e.g. djtouchette.jira).",
		"command.open_plugin",
		func(in openPluginIn) event.Envelope {
			return event.New("command.open_plugin", uiEventSource, in)
		})
	addUiTool(b, authtoken.ScopeTriage, "open_spawn_dialog",
		"Open the New Agent dialog in the desktop UI, optionally pre-filled with a directory — shows the user how to start an agent WITHOUT spawning one.",
		"command.open_spawn_dialog",
		func(in spawnDialogIn) event.Envelope {
			return event.New("command.open_spawn_dialog", uiEventSource, in)
		})
	addUiTool(b, authtoken.ScopeTriage, "open_guide",
		"Open the Workspacer Guide pane in the desktop UI — the built-in tour/help chat. Use to hand the user to the in-app guide instead of explaining workspacer yourself.",
		"command.open_guide",
		func(in openGuideIn) event.Envelope {
			return event.New("command.open_guide", uiEventSource, in)
		})
	addUiTool(b, authtoken.ScopeTriage, "run_ui_action",
		"Run one desktop keyboard action by registry id — the command layer's vocabulary as a tool: pane zoom/swap (zoom-pane, swap-pane-left), tab/agent navigation (next-tab, next-attention, alternate-agent, jump-pinned + digit), panel toggles (toggle-sidebar, toggle-fleet), chat paging (chat-scroll-down). The desktop validates the id and REFUSES the decision verbs (approve/deny) — approvals ride their own scoped tools.",
		"command.run_action",
		func(in runActionIn) event.Envelope {
			return event.New("command.run_action", uiEventSource, in)
		})
}

type focusAgentIn struct {
	SessionID string `json:"sessionId" jsonschema:"the session id of the agent to bring to the front"`
}

type openPaneIn struct {
	PaneType string `json:"paneType" jsonschema:"the pane type to open: settings, analytics, library, overview, sessions, review, terminal, or browser"`
	Cwd      string `json:"cwd,omitempty" jsonschema:"working directory for panes that take one (e.g. terminal)"`
	URL      string `json:"url,omitempty" jsonschema:"browser pane only: the URL to open"`
}

type openBrowserIn struct {
	URL string `json:"url" jsonschema:"the URL to open in the built-in browser pane"`
}

type runActionIn struct {
	Action string `json:"action" jsonschema:"the registry action id, e.g. zoom-pane, next-tab, toggle-sidebar, chat-scroll-down"`
	Digit  int    `json:"digit,omitempty" jsonschema:"slot for digit-taking actions (jump-pinned, 1-9)"`
}

type openPluginIn struct {
	Type string `json:"type" jsonschema:"the plugin pane type, e.g. djtouchette.jira"`
}

type spawnDialogIn struct {
	Cwd string `json:"cwd,omitempty" jsonschema:"directory to pre-fill in the dialog"`
}

// openGuideIn is deliberately empty: the guide pane is a singleton in the
// global workspace and takes no arguments.
type openGuideIn struct{}
