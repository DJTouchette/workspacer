# Agent status summaries

Settings → Sessions → **Summarize agent status on demand** controls the view-tier
MCP tool `summarize_agent_status({sessionId, hub?})`. It is enabled by default:

```yaml
agents:
  statusSummary:
    enabled: true
    provider: claude
    model: haiku
```

Anthropic / Claude is stored as `claude`, the existing harness identifier. Pick a
provider and model using the installed-provider detection and existing model
catalog. Switching providers resets the model to `null` (that harness's default,
whose cost varies). Explicit null survives both config writers. An incompatible
or missing provider/model returns unavailable; there is no automatic fallback.
The default Claude/Haiku uses the existing CLI subscription/login. No new API key
is required, and the setting does not alter login or worker spawn defaults.

The v1 no-tools adapters are Claude (empty built-in tools and strict empty MCP,
hooks and slash commands disabled), Pi (no tools or extensions), and Copilot
(empty available-tools list). Codex and OpenCode remain configurable but return
`no-tools-unsupported` without invoking a model: their existing read-only sandbox
and plugin-free mode do not enforce an empty tool registry. Unsupported CLI flags
on older versions fail explicitly. No provider substitution is attempted.

The source session must be visible on the owning hub. For remote sessions, pass
the `hub` supplied by `list_agents`; the source hub's desktop runs the summary.
Headless hubs and old desktops return `desktop-summary-unavailable` (the facade
cannot distinguish those two cases). A daemon must return the versioned bounded
projection; old daemons that ignore the query and send a full transcript are
rejected before a model call. Normal conversation reads are unchanged.

A request reads at most three early usable messages and 24 recent usable events,
with 800 characters per event and a 5,000-byte serialized source budget (including
JSON escaping). Only user/assistant text and narrowly recognized `report_progress`
notes are included. Tool results, arbitrary tool inputs, usage and terminal bytes
are excluded. `earliestRetainedTask` is an excerpt, not a promise that the original
task remains in history. Absent explicit progress is null. Source sequence,
timestamps and truncation flags describe the available evidence.

`status: ok | disabled | unavailable` describes **summary availability**, never
canonical worker lifecycle. Narrative claims are model interpretations, not
independently verified tests or completion. Use the direct blocked/escalation
message and final `wks-result` report for those decisions. Summaries never delay
or replace the normal terminal wake and result delivery.

Calls are on-demand only; enabling this setting starts no timer or polling loop.
Successful host-validated answers are cached in memory, keyed by owning hub,
session, source sequence/content, configuration and contract version. Every call
checks current visibility and reads a fresh bounded projection before reuse.
Concurrent identical requests share one completion; each waiter rechecks current
source and config before receiving an answer. New source/config, removal or denied
access prevents reuse. Only validated answers are stored, never raw transcripts;
the cache holds at most 128 entries for at most ten minutes. Errors are not cached.

The completion ceiling is 30 seconds. The whole request has a tighter 24-second
budget to fit the existing 25-second federation hop. A cancelled caller does not
cancel other waiters; the last waiter cancels the completion transport. Claude's
daemon independently kills its child at its own deadline even if the HTTP caller
disconnects. No model tools, project working directory or worker session is used.

### Codex completion blockers (verified 2026-09-06)

The manager has since accepted a Codex-specific utility-only contract: clock
and user-input utilities may remain, with client requests denied or completed
unavailable without UI. Environment access, MCPs, plugins, extensions, web
search and delegation remain forbidden. This resolves the literal empty-registry
decision below, but **does not yet make the adapter safe to enable**. The
app-server configuration issue described next is a separate blocker.

#### Remaining app-server configuration isolation gap

The installed 0.153.4 binary rejects
`codex app-server --ignore-user-config --help` with exit 2 and
`unexpected argument '--ignore-user-config'`. This flag belongs to `exec`.
The matching CLI source passes `LoaderOverrides::default()` to app-server;
the server's internal `ignore_user_config` support is not exposed by that CLI
route. Its test-only user-config-path environment variable is debug-only and
is not a supported production substitute.

`-c 'mcp_servers={}'` is also insufficient. CLI overrides form a layer, and
the config merger recursively merges tables: an empty table does not delete
inherited server entries. This is source evidence, not an inspection of the
user's configuration or credentials.

Crucially, `environments: []` does not independently remove those servers.
`McpManager::runtime_config_with_context` builds the catalog from config and
returns `McpEnvironmentAuthority::Unrestricted` for the default MCP environment
when no corresponding selection exists. The session runtime then retains a
default-environment cwd fallback and passes `effective_mcp_servers` to an eager
MCP runtime. Thus a configured default-environment MCP is not excluded by the
empty environment selection. Rejecting app-server client requests cannot
disable this server-owned MCP path.

Version-pinned evidence:

- [CLI app-server launch](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/cli/src/main.rs#L1245)
- [CLI override layer](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/config/src/overrides.rs#L9)
  and [recursive table merge](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/config/src/merge.rs#L95)
- [MCP catalog environment authority](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/src/mcp.rs#L279)
- [Effective MCP startup input](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/src/session/mcp_runtime.rs#L342)

Next prerequisite: a supported app-server configuration-isolation control that
retains CLI-owned auth and excludes inherited MCPs/instructions before thread
initialization. One concrete upstream option is exposing the existing
`ignore_user_config` loader option on app-server, followed by verification of
remaining managed-config and startup sources. That upstream change is outside
this Workspacer-only task. Do not substitute an invented flag, empty merged
table, copied credentials, or guardian identity. Codex remains fail-closed;
no bounded adapter or utility-denial transcript tests have been implemented.

#### Original empty-registry finding (decision now resolved)

The requested Codex support is **not complete**. The installed `codex-cli
0.153.4` cannot be enabled under this feature's empty-tool-registry contract
using the verified CLI/app-server interfaces:

- `codex exec --help` offers `--ignore-user-config` (retains CLI auth),
  `--ignore-rules`, `--ephemeral`, and sandbox options, but no blanket tool
  deny/allowlist. Ignoring config is not disabling tools.
- The installed `app-server generate-json-schema --experimental` exposes
  `thread/start.dynamicTools` and `thread/start.environments`. Neither
  `thread/start` nor `turn/start` has a complete tools allowlist or
  `tool_choice` override. `dynamicTools: []` only removes client-supplied tools.
- `environments: []` removes environment access, and the matching version's
  tool builder gates shell, apply-patch, and view-image on environment presence.
  It does **not** empty the registry. `add_core_utility_tools` registers
  `RequestUserInputAsyncHandler` when the model catalog advertises it, and
  `CurrentTimeHandler` when the catalog advertises `clock`, even if the current
  time feature is disabled. The synchronous question and plan settings do not
  disable those model-driven registrations.
- The matching config schema's `ToolsToml` only contains `web_search`,
  `experimental_request_user_input`, and `update_plan`. Its `tool_registry`
  settings concern collisions and metadata, not denial. Code-mode namespace
  exclusions only affect the nested tool surface. An empty MCP configuration,
  disabled plugins, read-only sandbox, or prompt instructions cannot establish
  the required empty registry.

Evidence: [app-server protocol documentation](https://developers.openai.com/codex/app-server),
[0.153.4 tool registration source](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/src/tools/spec_plan.rs#L1131),
and [0.153.4 config schema](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/config.schema.json).
The source tag matches the installed binary; the generated protocol schema
comes from that binary, not the latest online documentation.

Non-spending reproduction (use the actual installed binary, since a shell/npm
launcher may update itself; choose an output directory in an isolated checkout):

```sh
"$SUMMARY_CODEX_BIN" --version
"$SUMMARY_CODEX_BIN" exec --help
"$SUMMARY_CODEX_BIN" app-server --help
"$SUMMARY_CODEX_BIN" app-server generate-json-schema --experimental --out "$SUMMARY_SCHEMA_DIR"
```

The original design decision was whether to retain the strict contract and require an
upstream Codex control that both empties the registry and denies dispatch, or
explicitly revise the contract to permit enumerated utility tools while
forbidding environment access, MCPs, extensions, and writes. The latter still
needs an adapter implementation and effective-request/denial tests; it is now
authorized but remains unimplemented due to the isolation gap above. Replacing model metadata, impersonating an
internal guardian session, or intercepting authenticated provider requests is
not an established supported no-tools route.

OpenCode 1.15.7's `run --help` likewise describes `--pure` as disabling external
plugins only. Its existing fail-closed UI warning and adapter remain in place;
an alternative OpenCode configuration/server route was not verified in this
Codex-focused investigation. No authenticated model request, provider session,
credential read, or live daemon rebuild/restart was used for these checks.
