# Automatic manager task capture

The local desktop owns an authenticated request inbox. A host request ID is
persisted before chat delivery; the inbox is the source of actionable user
content, independently of provider transcript rendering. No classifier call or
daemon/provider protocol expansion is involved.

## User behavior

Manager chat saves a request, then attempts normal chat delivery. Inspector shows
pending requests and tasks as the manager resolves them. A question or status
request can resolve without a task. Corrections retain an existing task ID;
independent work creates another task, even before any worker is dispatched.

A followup such as “publish nightly after these fixes” becomes a separate pinned
workflow task with concrete same-project task dependencies. Inspector shows the
dependency title and waiting/ready state. Clicking the title selects that task.
Internal IDs remain inside Details. No scheduler or publishing action is added.

Delivery status is distinct from request interpretation and task success:

- Pending: saved but not yet submitted, or held for manager replacement.
- Accepted: the daemon acknowledged admission, possibly into its queue. This is
  not evidence that the model consumed the turn.
- Rejected: the daemon explicitly refused delivery; resolution is unavailable.
  Retrying the restored draft keeps its logical request ID and adds an attempt.
- Unknown: admission acknowledgement is ambiguous. The manager may fetch and
  resolve the authoritative inbox request, without pretending chat delivery was
  confirmed. The host never automatically replays that provider input.

## Manager tools and authority

`list_manager_requests` returns trusted host metadata and task readiness.
`get_manager_request` returns exact unresolved `userContent` separately; it
remains user input, not trusted system instructions. The authenticated caller
owns the request. Neither user content nor MCP arguments can choose another
manager's identity.

`resolve_manager_request` atomically resolves 1–8 stable intent keys under a
request revision CAS. Supported kinds are create, followUp, update, question and
none. Task updates additionally require the current task revision. An identical
retry returns the committed task IDs; a conflict returns current state. Creation
pins the currently selected project policy; it never changes workflow settings.
A release-specific policy must already be selected under appropriate user scope.

`accept_task_outcome` records the manager's explicit interpretation of concrete
stored step results under task revision CAS. Schema validity alone, idle workers,
waived steps and failed/blocked steps cannot satisfy a dependency. Recorded
outcomes must continue to match the accepted evidence. Readiness informs the
manager; resolution and acceptance grant no push, publication, destructive or
credential authority. Cancellation changes task intent without stopping workers.

## Persistence and compatibility

Requests and tasks share the existing locked, atomic, private 0600 JSON history.
Logical request IDs differ from delivery-attempt IDs and handoff delivery IDs.
Ownership transfer moves unresolved requests and task/dependency lineage in the
same transaction, preserving original source-session provenance. Captured handoff
messages retain source references and do not receive IDs appended to chat text.

Original content is retained only while unresolved (up to 64 KiB per request).
Resolution deletes it, retaining bounded provenance and interpretation. The inbox
caps retained requests at 256, retries at eight, and uses the existing total
history byte limit. Pending requests and live task/dependency references cannot
be evicted to admit new work; capacity exhaustion fails visibly.

The rollout is additive. Local desktop composer and manager entry points prepare
requests. Remote/headless clients report capture unavailable and retain legacy
chat behavior. Ordinary non-manager chat and synthetic fleet events never create
inbox requests. Once a manager resolves inbox requests, new work dispatches need a
committed task; historical tasks and their continuations retain compatibility.
Managers check the inbox once on each user/wake turn, then end their turn normally.

## Review and validation

Integrated baseline: `0664d03c0cc7f389fe438fa6d3ea94fb3e60eae3`, including compact
Inspector/reference CAS and Windows handoff validation. No primary runtime or
nightly artifact is modified by this branch.

Start with `managerRequestService.ts`, `dispatchHistoryStore.ts`,
`claudemonSessionClient.ts`, and the authenticated `manager_requests.go` tools.
The feature workflow runs service/real-HTTP transport regressions, renderer tests,
and the existing real authenticated MCP→bus→desktop integration using private
fixtures and no real providers. Local execution is prohibited on the OOM-affected
host; CI results must be reported against the actual branch commit.
