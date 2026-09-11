# Fleet Manager context efficiency

The September 10, 2026 audit used a read-only snapshot of the running manager
(459 conversation items, 177 tool calls). It contained 22 inbox results totaling
375,887 UTF-8 bytes, plus 16 separate original-request reads. Resolved history
was being returned on every turn. These are payload measurements, not model
token counts, active context occupancy, or billing estimates.

The manager now uses `list_manager_requests({view:"pending"})`. This returns up
to eight eligible unresolved requests with original user content, bounded to
16 KiB of embedded UTF-8 content. Oversized originals are marked
`contentDeferred` and remain available intact through `get_manager_request`.
After resolving a batch, `remaining` indicates whether another batch is needed.
An empty check stays under 100 bytes regardless of retained history. Omit `view`
to inspect the original full metadata/task inventory. Ownership, delivery
eligibility, original-content trust and revision checks still apply.

Workflow and request tools expose only the arguments their operation consumes.
The 17 workflow/inbox tool schemas plus descriptions measure 9,109 bytes in the
MCP regression harness. `compact:true` on task-returning tools omits pinned
template bodies, retaining policy, template parameters, result schemas, task
identities and outcome evidence. `next_workflow_step` without compact mode
still returns the complete pinned task. The host always uses the full template;
this optimization applies only to the answer returned to the manager.

The manager doctrine source shrank from 31,237 to 11,946 bytes (62%). Standup
and checkpoint instructions are also shorter. The operating rules now reuse
wake evidence and current next-step instructions, load tool help only as
needed, avoid an automatic all-project brief sweep, and keep worker prompts
focused on task-specific criteria and context. Selected review policy,
independent review, escalation validation, persistent authorization, and the
host-owned versus standalone handoff distinction remain explicit. No models,
context limits, wake routing or active sessions were changed.

These changes reduce payloads and avoid redundant model round trips. There is
no claim of measured end-to-end model latency or a universal token-saving
percentage. Existing sessions keep instructions already delivered to them;
fresh/replacement managers receive the new doctrine. The compact tools are
opt-in and available to existing sessions after the updated host/facade load.

## Repeat the audit

From `apps/desktop`:

```sh
npm run audit:fleet-context -- --session <session-id>
npm run audit:fleet-context -- --file /path/to/conversation.json
```

The command performs only a daemon read (or reads a saved snapshot) and prints
counts/byte totals per tool. It does not print prompt or tool-result contents.
Nested `exec` calls remain attributed to `exec`; a windowed snapshot is not the
entire session.

## Validation

- MCP tests and Go race tests for the facade and headless brain.
- Request tests: batch draining, rejected/pending/resolved exclusion, original
  content and revision preservation, foreign ownership, oversized/non-ASCII
  content, historical compatibility, and idle-response size.
- Real authenticated MCP → hub → desktop dispatch-chain harness: 14 scenarios,
  including the combined read and compact pinned-task response.
- Fleet workflow, context-menu and manager-handoff browser harnesses: 22 tests,
  including narrow/wide layouts and light/dark themes.
- Main/renderer regression suites, TypeScript checks and formatting.
