// The help tool: on-demand usage docs for the facade's tools.
//
// Tool descriptions are deliberately one line each — every connected agent
// pays context for the schemas whether or not it uses them — so the guidance
// that used to live in hand-written system prompts lives here instead, fetched
// only by agents that actually go to use the tools. The tool list itself is
// rendered from the build registry, so help can never describe a tool the
// caller's tier doesn't hold.
package main

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type helpIn struct {
	Topic string `json:"topic,omitempty" jsonschema:"a tool group to expand (one of the groups the overview lists); omit for the overview"`
}

// groupGuidance is the per-group usage text — the "how", where the tool
// descriptions carry only the "what". Written for the model.
var groupGuidance = map[string]string{
	"observe": strings.TrimSpace(`
Start with list_agents (cheap fleet overview), then go deeper on ONE session.
Reading another agent's activity, cheapest first:
- get_conversation with sinceSeq: track the returned seq per session and pass
  it back next time, so you only ever digest new turns. This is the right tool
  for polling "what has agent X done since I last looked".
- get_snapshot: full live detail for one session (turns, tools, usage, pending
  approval/question). Heavier; use when you need everything at once.
- get_transcript: the raw transcript. Largest payload — prefer the two above,
  or spawn a cheap worker (spawn_agent with toolScope "view") whose whole job
  is to read it and reply with a digest, so it never enters your own context.
When you reference a session in an answer, write its id as session:<sessionId>
so the UI renders a clickable link.`),
	"spawn": strings.TrimSpace(`
spawn_agent starts a new coding-agent session and returns its sessionId.
- Pass label (short human name) and parentSessionId (your own session id) so
  the new agent nests under you in the UI.
- Give the new agent workspacer tools only when it needs them, at the LOWEST
  tier that works: toolScope "view" for summarizer/reader workers, "triage" to
  also approve/reply/interrupt, "operator" for everything (spawning included).
  mcpFacade:true is the legacy spelling of operator.
- Drive it afterwards with send_message; watch it with get_conversation.`),
	"drive": strings.TrimSpace(`
send_message queues a prompt for an agent (delivered when it can accept input).
approve resolves a pending permission prompt (yes/no/always — "always" persists
a standing allow, so use it deliberately). answer resolves an AskUserQuestion
picker. signal sends SIGINT (interrupt) / SIGTERM (stop). set_approval_gate
parks every tool call for approval — useful before letting an agent continue
unattended. terminal_input types raw bytes into a PTY session (shells from
create_terminal); prefer send_message for agents.`),
	"files": strings.TrimSpace(`
Host-filesystem access (the machine workspacer runs on): list_dir / list_entries
to explore, read_file / write_file for file IO, search_project for ripgrep
across a project. Use these to inspect or brief work, not to do the coding
yourself — spawn an agent in the directory instead.`),
	"config": strings.TrimSpace(`
get_config returns the full workspacer config; save_config deep-merges a
partial patch (pass ONLY the keys you change). reload_config re-reads disk.`),
	"profiles": strings.TrimSpace(`
Claude profiles are named CLAUDE_CONFIG_DIR + extra-args presets applied at
spawn (spawn_agent's profileId). list/add/update/remove.`),
	"sessions": strings.TrimSpace(`
Saved workspace sessions are pane/agent ARRANGEMENTS (the session picker), not
live agents — list_agents is the live fleet. list/load/save/delete by filename.`),
	"layouts": strings.TrimSpace(`
Layout templates are saved pane-geometry presets. list/save/delete.`),
	"library": strings.TrimSpace(`
The library holds reusable prompts, skills, and agent definitions, global or
per-project (pass cwd). list/save/remove.`),
	"analytics": strings.TrimSpace(`
analytics_summary aggregates usage/cost across sessions; analytics_recent
returns the latest finished sessions with per-session usage.`),
	"notify": strings.TrimSpace(`
notify shows a desktop notification on the workspacer machine — use it to
surface something that needs the user's attention.`),
	"ui": strings.TrimSpace(`
These steer the user's DESKTOP screen — use them deliberately.
- Right for guided tours ("show me around workspacer", first-time onboarding):
  narrate each step in chat, then focus_agent / open_pane / open_browser so the
  user sees what you describe. open_spawn_dialog shows how to start an agent
  without actually spawning one.
- open_browser opens the built-in browser pane on a URL — use it to walk the
  user through web docs (e.g. the workspacer guides) alongside your narration.
- Otherwise PREFER notify: a notification lets the user click through when
  ready, while navigation yanks their screen mid-thought. Never re-focus
  repeatedly; once per step of a tour the user asked for.
- Fire-and-forget: "ok" means the command was sent, not that the UI acted. On a
  headless/remote-only setup nothing consumes these.`),
	"plugins": strings.TrimSpace(`
Plugin tools are contributed by installed workspacer plugins and forwarded to
the plugin's own sidecar. They appear only when your session was granted them
at spawn (pluginTools). Errors like "no provider" mean the plugin is not
running.`),
}

// addHelpTool registers the help tool on a tier's server, rendering from that
// tier's registry.
func addHelpTool(b *build) {
	tools := append([]toolInfo(nil), b.tools...)
	scope := string(b.scope)
	mcp.AddTool(b.s, &mcp.Tool{
		Name:        "help",
		Description: "Usage guide for the workspacer tools: call with no arguments for the grouped overview, or with a topic (group name) for detailed guidance. Call this before first using the other workspacer tools.",
	}, func(_ context.Context, _ *mcp.CallToolRequest, in helpIn) (*mcp.CallToolResult, any, error) {
		return &mcp.CallToolResult{
			Content: []mcp.Content{&mcp.TextContent{Text: renderHelp(scope, tools, in.Topic)}},
		}, nil, nil
	})
}

// renderHelp renders the overview (topic == "") or one group's detail.
func renderHelp(scope string, tools []toolInfo, topic string) string {
	byGroup := map[string][]toolInfo{}
	var order []string
	for _, t := range tools {
		if _, seen := byGroup[t.Group]; !seen {
			order = append(order, t.Group)
		}
		byGroup[t.Group] = append(byGroup[t.Group], t)
	}

	topic = strings.ToLower(strings.TrimSpace(topic))
	if topic == "" {
		var sb strings.Builder
		fmt.Fprintf(&sb, "Workspacer tools — tier: %s. Call help with a topic for usage guidance.\n", scope)
		for _, g := range order {
			names := make([]string, 0, len(byGroup[g]))
			for _, t := range byGroup[g] {
				names = append(names, t.Name)
			}
			fmt.Fprintf(&sb, "- %s: %s\n", g, strings.Join(names, ", "))
		}
		return strings.TrimRight(sb.String(), "\n")
	}

	group, ok := byGroup[topic]
	if !ok {
		known := append([]string(nil), order...)
		sort.Strings(known)
		return fmt.Sprintf("unknown topic %q — this tier's topics: %s", topic, strings.Join(known, ", "))
	}
	var sb strings.Builder
	fmt.Fprintf(&sb, "%s tools (tier: %s)\n", topic, scope)
	for _, t := range group {
		fmt.Fprintf(&sb, "- %s: %s\n", t.Name, t.Desc)
	}
	if g := groupGuidance[topic]; g != "" {
		fmt.Fprintf(&sb, "\n%s", g)
	}
	return sb.String()
}
