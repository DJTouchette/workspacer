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
The fleet may span machines: when hubs are federated, list_agents and
list_snapshots include every connected peer hub's sessions, and each remote
row carries a "hub" field naming its machine (local rows have none). That hub
value must round-trip: pass it as the hub input on the per-session tools
(get_snapshot, get_conversation, get_transcript, send_message, approve,
answer, signal, set_approval_gate) to reach the remote session; omit it for
local sessions.
Reading another agent's activity, cheapest first:
- get_conversation with lastMessage:true: returns { seq, lastMessage } — just
  the session's final assistant message. The right way to read a finished
  worker's report without paying for its whole conversation.
- get_conversation with textOnly:true: only the user/assistant text turns
  (tool calls, tool results, and usage blobs stripped) — the dialogue without
  the machinery. Both reductions compose with sinceSeq; lastMessage wins when
  both are set.
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
- skipPermissions omitted = the workspacer config default (the same one the
  desktop spawn dialog pre-selects: claude.skipPermissionsDefault, or a bypass
  defaultPermissionMode); an explicit true/false always wins. Requested or
  defaulted, a bypass is honored only when your session token carries the
  full-access grant — ungranted spawns start with approvals on.
- resultSchema (optional) asks the worker for a MACHINE-READABLE report as well
  as its prose: pass a JSON Schema and the worker is told to end its final
  message with a fenced wks-result block matching it, which arrives back on your
  worker-finished wake already parsed and VALIDATED. Use it whenever you will
  transcribe the outcome into a brief — it turns restating a report into copying
  fields. Typical shape: {"type":"object","required":["commit"],"properties":
  {"commit":{"type":"string"},"filesChanged":{"type":"array","items":
  {"type":"string"}},"checksRun":{"type":"array","items":{"type":"string"}},
  "caveats":{"type":"string"},"followUps":{"type":"array","items":
  {"type":"string"}}}}. Additive: the prose report is unaffected, and a worker
  that skips or botches the block reports that beside its prose rather than
  failing. Validated keywords are type/properties/required/items/enum/
  additionalProperties; anything else is ignored (it can under-constrain, never
  wrongly reject).
- Drive it afterwards with send_message; watch it with get_conversation.
- To spawn on a federated peer machine, pass hub (a hub name seen on
  list_agents rows). The peer clamps remote spawns itself — permission bypass
  is refused there — and driving the new agent needs the same hub value.`),
	"drive": strings.TrimSpace(`
send_message queues a prompt for an agent (delivered when it can accept input).
approve resolves a pending permission prompt (yes/no/always — "always" persists
a standing allow, so use it deliberately). answer resolves an AskUserQuestion
picker. signal sends SIGINT (interrupt) / SIGTERM (stop). set_approval_gate
parks every tool call for approval — useful before letting an agent continue
unattended. The gate is a workspacer-side hold, SEPARATE from the session's
Claude permission mode: gate off does not stop the session's own permission
prompts (only its permission mode governs those; the gate response reports the
current mode). terminal_input types raw bytes into a PTY session (shells from
create_terminal); prefer send_message for agents. For a session on a federated
peer (its list_agents row has a "hub" field), pass that hub value on
send_message / approve / answer / signal / set_approval_gate.`),
	"files": strings.TrimSpace(`
Host-filesystem access (the machine workspacer runs on): list_dir / list_entries
to explore, read_file / write_file for file IO, search_project for ripgrep
across a project. Use these to inspect or brief work, not to do the coding
yourself — spawn an agent in the directory instead.`),
	"jobs": strings.TrimSpace(`
Jobs are recurring or one-off tasks the HUB runs unattended: spawn an agent
with a prompt, run a shell command, or call a capability — on an interval, at a
daily time, once, or manually. They keep firing with the app closed.
You can read them (list_jobs, job_history), run or delete an existing one, and
PROPOSE new ones — you cannot arm one. A proposal is saved disabled with your
name on it and never runs until the user approves it in Settings → Jobs, so
tell the user it is waiting for review; do not report it as scheduled.
A spec is {"name","enabled","trigger","action"}:
- trigger: {"kind":"interval","everyMinutes":60} | {"kind":"daily","at":"09:00",
  "days":[1,2,3,4,5]} (0=Sunday, omit for every day) | {"kind":"once","once":
  "<RFC3339>"} | {"kind":"manual"}
- action: {"kind":"spawn","spawn":{"cwd","prompt","provider","model"}} |
  {"kind":"shell","shell":{"command","cwd"}} |
  {"kind":"call","call":{"method","params"}}
A spawn action may run CONTEXT STEPS first — code whose output is fed to the
agent, and whose guards can cancel the run so no model is woken:
  "context":[{"kind":"shell","shell":{"command":"go test ./..."},
              "skipIfEmpty":true,"skipUnlessMatch":"FAIL","ignoreExitCode":true}]
Their output is substituted into the prompt at {{output}} (or {{output.1}},
{{output.2}}… with several steps). Prefer a guarded spawn over an unguarded
one: a job that wakes a model nightly to answer "nothing to do" is waste the
user pays for. Calls may not target jobs.* or hub:<peer>/, and there is a
maximum of four context steps.`),
	"lifecycle": strings.TrimSpace(`
Stopping a worker is TWO steps, and both are verbs now:
1. signal({sessionId, signal:"SIGTERM"}) stops the process. (SIGINT just
   interrupts the current turn.)
2. close_session({sessionId}) DISMISSES the row: it leaves list_agents and the
   fleet stops counting it. Before this existed, the only confirmation a worker
   had really died was sending it another signal and reading a 404.
respawn_with({sessionId, amendment}) is the OTHER half of that move: it clones
the stopped worker's ORIGINAL task and its cwd/model/provider/effort/parent,
appends your correction under a heading that supersedes anything above it, and
starts a fresh agent with both — so you state the DIAGNOSIS, not the whole task
again. Override model/effort/label/cwd/toolScope as needed; pass worktree:true
to start clean instead of continuing in the original's worktree. It refuses
without an amendment (a clone with no correction just repeats itself), and the
successor's permission mode is re-judged by the same grant check a fresh
spawn_agent gets — a bypassed original does not make a bypassed clone.

close_session refuses while the session is still working — hiding a running
agent from list_agents while it keeps spending is worse than a stale row — and
is idempotent, so closing an already-forgotten session succeeds. It stops the
daemon side too for a session that had not already ended, so "dismissed" is not
a lie. The user's desktop PANE is theirs to close; this is the fleet's view.`),
	"watch": strings.TrimSpace(`
notify_when({sessionId, tokens|usd|idleSeconds, notifySessionId?}) is how you
keep an eye on a running worker WITHOUT polling. Never loop on list_agents or
get_conversation to "keep an eye on" something — that is a hang, and it locks
the user out. Arm a watch and STOP.
- Give at least one threshold. tokens is CUMULATIVE (input + output), so it
  catches scope creep a context percentage would not; usd is cumulative cost;
  idleSeconds catches a worker that stopped without finishing.
- Crossing delivers a [fleet] wake naming exactly what crossed.
- ONE-SHOT: the watch is discarded when it fires. Arm another if you still want
  to watch — that is a decision you should make deliberately, not a loop.
- The wake goes to the target's PARENT by default (you, for a worker you
  dispatched); notifySessionId overrides it.
- Watches live in memory: a workspacer restart clears them.`),
	"brief": strings.TrimSpace(`
brief_append({project, section, line}) adds ONE line to a project's
.workspacer/brief.md. Use it instead of read_file + write_file: it is
inspect-then-edit under a lock, so it cannot clobber a line a worker or the user
wrote while you were composing yours, and it is strictly additive — it never
rewrites, reorders or reformats anything already in the file.
- project is the project DIRECTORY (your own cwd for your fleet brief); the
  .workspacer/brief.md path under it is composed for you.
- section is Now | Direction | Recently | User. A typo is refused, not guessed.
- Recently PREPENDS (a dated log, newest first); the rest append.
- The brief is created, with all four sections, if the project has none.
It can only ADD. Removing a stale line, or archiving, is still a file edit.`),
	"projects": strings.TrimSpace(`
project_status({}) returns the git state of EVERY configured project in one
call: branch, upstream, unpushed (commits ahead), behind, and whether the tree
is dirty. Use it for a standup instead of shelling out per repo. Pass dirs to
limit it. A directory that is not a repo comes back as a row carrying an error,
so one bad path never costs you the other rows. "unpushed" is ABSENT (not 0)
when a branch has no upstream — that is "nowhere to push", not "nothing to
push".`),
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
- open_guide opens the built-in Workspacer Guide pane — a chat with a dedicated
  tour agent. Hand off "how do I use workspacer" questions there instead of
  answering at length yourself.
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
