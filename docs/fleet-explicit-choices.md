# Explicit Fleet Manager choices

Task tracking, workflow selection and model routing are defaults. The manager
must honor explicit user choices instead of creating extra tasks or substituting
a role's preferred model.

| User choice | Manager action |
| --- | --- |
| “Use Luna for this step” | `dispatch_workflow_step` with `modelSelection:{provider,model,effort?}`; no automatic routing |
| “Run it without a task” | Resolve the inbox intent as `untracked`, then `spawn_agent` with `trackTask:false` and the normal parent session |
| “Track the audit, but no workflow” | Resolve a create/followUp intent with `workflowId:null`, then use ordinary `spawn_agent` with the returned task ID |
| “Use this workflow for this task” | Set `workflowId` to an enabled definition ID on the intent; project/global defaults stay unchanged |
| “This task has the wrong workflow” | An update intent may change/remove `workflowId` before any worker starts, under the existing task revision check |

An explicitly chosen model is not replaced by automatic routing. Manual spawns
use `provider`, `model`, and `exactModel:true`, without carrying a stale
`capability` or `decisionId` from a different model. The composed dispatcher does
this automatically for `modelSelection` and identifies the receipt as
`selection.source:"explicit"`; it does not fabricate a routing decision.

Directory model ceilings still apply. If a ceiling would substitute another
model, an exact-model spawn is refused before launch and the refusal is audited. Launch providers also reject model substitutions
reported by an adopted older hub before starting a worker.
Filesystem access, profile grants, permission bypass grants, task ownership and
review-worker freshness remain enforced. Explicit model selection does not
retroactively alter an already-running worker or erase recorded outcomes.
Paired/federated exact-model requests require an updated peer stack advertising
support; older peers are not silently trusted to honor the pin.

An untracked worker remains visible and nested under its manager, and completion
and progress wakes still work. No Task or workflow attempt is created for it.
Session usage and remote transport journals still exist; “untracked” refers to
Task bookkeeping, not invisible execution. Do not combine `trackTask:false` with
a task, workflow step or retry-history link. `respawn_with` accepts the same
opt-out and preserves parentage; explicit model overrides no longer inherit the
old model's capability label.

Freeform tracked tasks retain task identity, references and bounded recent
attempts in `manager_context`, but acquire no prescribed implementation/review
steps. `next_workflow_step` returns explicit freeform guidance. Workflow outcome
acceptance remains specific to pinned workflows; freeform history is not reported
as a completed workflow. Once an attempt exists, its pinned policy is not
rewritten: use a separately requested one-off when appropriate.

The existing defaults remain for ordinary code work. Read-only audits and
mechanical chores should not be forced into an implementation workflow, and an
explicit no-task request must not be worked around by creating another task.
