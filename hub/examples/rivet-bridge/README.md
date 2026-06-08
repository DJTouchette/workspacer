# rivet-bridge

A workspacer hub plugin that exposes a [rivet](https://github.com/DJTouchette/rivet)
project on the hub bus.

Rivet is a "project capability layer for Claude Code" — an MCP server giving an
agent deterministic codebase intelligence (**recon**), smart test selection
(**witness**), and read-only DB awareness (**schema**), plus secrets (vaulty),
ticket sync (rally), and living context docs.

Rivet already works per-agent: point a Claude session's MCP config at
`rivet serve` and that one agent gets everything. **This bridge is for the other
altitude** — making rivet's deterministic capabilities callable at the *fleet*
level, over the hub bus, by things that aren't a single Claude session:

- the **rules-engine** (`on agent.state_changed → witness.select → notify`)
- **dashboards** / Fleet Deck (render `recon.hotspots` across agents)
- the **MCP facade** (`cmd/mcp`) — so a remote Claude reaches rivet through the hub
- **"ask the fleet"** supervisor agents that need recon without re-exploring

It's the inverse of `cmd/mcp`: that exposes hub capabilities *as* MCP tools;
this consumes an MCP server and *re-exposes* its tools as hub capabilities.

## How it works

```
hub bus ──call recon.symbols──▶ rivet-bridge ──MCP tools/call──▶ rivet serve
                                  (this plugin)   (stdio JSON-RPC)   (subprocess)
```

Rivet has no HTTP/SSE transport — MCP is stdio only — so the bridge spawns
`rivet serve` as a child, speaks line-delimited JSON-RPC 2.0 to it, and
supervises/restarts it with backoff. On the bus side it registers as a
capability **provider** and answers incoming `call` frames.

## Provided capabilities

A **curated** subset of rivet's tools (see `provides` in `plugin.json`):

| Namespace | Methods |
|-----------|---------|
| `recon.*`   | overview, search, symbols, related, tests, changes, context, hotspots, grep |
| `witness.*` | select, staged, since |
| `schema.*`  | overview, tables, describe |

Each forwards 1:1 to the matching rivet MCP tool. Params are passed through as
the MCP `arguments` object. Rivet's recon/witness/schema tools take an `args`
string array, so a call looks like:

```json
{ "method": "recon.symbols", "params": { "args": ["MyHandler"] } }
```

`witness.since` takes `{ "ref": "<git-ref>" }`.

The bridge returns `{ "tool": "<name>", "output": "<text>", "isError": <bool> }`.

### Deliberately NOT bridged

- **vaulty** — secrets must not transit the bus.
- **rally**, **context/learn** — per-session, per-project state with no fleet value.

These stay available to agents via per-agent rivet MCP config.

## Events

- `rivet.ready` — emitted when the MCP session initializes (`{projectDir}`).
- `rivet.down` — emitted when `rivet serve` exits (`{projectDir, error}`).

## Configuration

| Flag | Env | Default | Meaning |
|------|-----|---------|---------|
| `--project-dir` | `RIVET_PROJECT_DIR` | cwd | project root for `rivet serve` (must anchor `.rivet/`) |
| `--rivet` | `RIVET_BIN` | `rivet` | path to the rivet binary |
| `--token` | `HUB_TOKEN` | — | bus auth token; sent as `?token=`. Inherited from the hub's env when supervised, so no setup needed if the hub has `HUB_TOKEN` in its environment (e.g. remote share). |
| `--bus` | — | `ws://127.0.0.1:7895/bus` | hub bus URL |
| `--port` | — | `9130` | health + pane HTTP port |
| `--debug` | — | off | pass `--debug` to rivet and log its stderr |

**Important — set `RIVET_PROJECT_DIR`.** The hub supervisor runs plugins in
their own install dir, so without this the bridge points `rivet serve` at the
wrong place. Export it before launching the hub.

### Limitation (v1): one bridge, one project

A workspacer fleet can span multiple repos/worktrees, but a single
`rivet serve` is bound to one project root. Multi-project routing (a bridge that
selects the rivet instance per the calling agent's cwd) is future work.

## Build & run standalone

```sh
go build -o rivet-bridge .
RIVET_PROJECT_DIR=/path/to/your/project ./rivet-bridge --debug
```

Then open `http://127.0.0.1:9130/` for the pane, or drive it from the bus.
Under the hub, the `install` command in `plugin.json` builds it automatically.

## Pane

A minimal webview (`web/index.html`) with a button per tool and an args box,
calling the bridge's `POST /api/call` proxy. Health badge shows rivet up/down.
