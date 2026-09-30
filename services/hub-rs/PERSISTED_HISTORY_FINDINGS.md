# Retained task, review, and replacement history

Read-only source audit against candidate `a265b956fb5ba8fa2b7c8f92210d52e16f578f5c`, 2026-09-30. This prep-only finding does not certify a gate or report new test execution.

All three stores have retained TypeScript owners; none should be described as a new Rust-only journal. The Rust loaders directly read the existing representations. No conversion to a new schema is required by the inspected current TypeScript writers. Existing restart tests are substantial, but they predominantly write through the Rust implementation before reopening; that distinction limits a claim of independently proven TS-to-Rust import.

| Family | Retained writer and Rust reader | Concrete existing proof | Remaining claim boundary |
| --- | --- | --- | --- |
| `dispatch-history.json` | `apps/desktop/src/main/services/dispatchHistoryStore.ts` loads/writes `{version:1,tasks,requests?}`. `services/hub-rs/src/services/task_store.rs::History/read/validate` reads version1; missing requests defaults empty. Task and request records remain JSON values. | `tests/task_store.rs::atomic_multi_task_rollback_and_cross_instance_revision_cas` checks two reopened instances, durable revisions, rollback and no-op byte preservation. `durable_dispatch_reservation_fences_adoption_and_crash_views_are_stale` reopens and refuses to invent live ownership. Request uncertainty/idempotence tests cover non-replay. TS `dispatchHistoryStore.test.ts` independently covers its writer, including empty version1 bytes and corrupt input. | No independently captured TypeScript-written nonempty document consumed by Rust was identified in the inspected owning target. The existing hand-built task helper represents retained fields, but is inserted through Rust transactions. Do not describe it as a historical import fixture. |
| `fleet-review.json` | `fleetReviewStore.ts::State/load/save` is **unversioned** `{allocations,records,readers?}`. Rust `fleet_review.rs::State/strict_load/save` uses the same representation, with missing readers defaulted empty. Allocation and evidence entries remain JSON values. | Rust `production_teardown_preserves_full_range_for_finish_wake_history_and_restart` captures real Git evidence, removes the original worktree/branch, changes manager HEAD and reopens retained evidence. TS `fleetReviewStore.test.ts` separately performs the same capture/restart assertion. Rust retention test writes cloned Rust-generated records as raw JSON before reading; that proves retained-record loading, not independent TS provenance. | No versioned fleet-review migration should be invented. An independent old-writer raw record would make the cross-language import claim explicit; existing restart already proves no dependence on current Git state. |
| `manager-replacements.json` | `managerReplacementState.ts::Journal/load/edit` writes `{version:1,operations,launches,pendingMetadata?}`. Rust `manager_replacements.rs::Journal/read/validate` retains those fields and adds optional default-empty `pendingSignatures`. Operation/launch/metadata values remain JSON. | `tests/manager_replacements.rs::restart_marks_unacknowledged_delivery_uncertain_and_never_completes_ownership` reopens an activating committed operation, then requires recovery-required, bound=false and sending→uncertain without replay. Transfer-intent, worker-parent projection, manual redirects and journal rollback tests cover ownership semantics. | The Rust `operation()` fixture spells the retained TS operation shape but writes it through `ReplacementState::edit`; it is not bytes emitted by the old writer. Current required fields/status spellings agree. No actual import defect was found. |

## Stable fields and preservation scope

Task identity requires taskId, ownerSessionId, projectCwd and attempts; attempts require dispatchId, sessionId and metrics. Requests retain their source/owner identity, revision, delivery state, digest, attempts and content. Rust validates duplicate identities and object-shaped metrics more strictly than loose JavaScript truthiness; the inspected typed TS writer emits the valid object/string shapes. Do not call acceptance of arbitrary malformed legacy JSON a supported migration requirement.

Replacement records retain operation/source/successor IDs, phase, committed/bound flags, pane/workspace IDs, worker/task IDs, launch.options (manager/operator/cwd), metadata, signatures, finishes and deliveries. The TS-created UUID and current phase/delivery vocabulary match Rust validation. Optional pendingMetadata is compatible with Rust's default-empty map; pendingSignatures is an extension, not a mandatory old field. Restored metadata never establishes live process ownership.

Nested task/request, allocation/evidence, and replacement operation/launch fields use serde_json::Value and survive ordinary decode/rewrite unless the relevant mutation explicitly changes them. Root envelopes are typed structs without flatten: unknown **top-level** keys are ignored at decode and would disappear on a subsequent rewrite. TS replacement/review code retains its parsed root object, while TS dispatch rewrites a selected version/tasks/requests envelope. No inspected current writer defines additional required top-level fields that Rust loses. Therefore claim preservation of supported current fields and nested opaque payloads, not arbitrary future envelope extensions. Read alone does not rewrite the original file.

## Smallest independent import control, if the gate claims TS-to-Rust import

One fixture per representation suffices; no universal migration harness is implied. Prefer bytes emitted by the retained TS writer in an isolated existing TS test and checked in with writer source hash, rather than a new Rust serializer producing its own input. A raw document transcribed from the writer shape is also useful but must be labeled as such.

1. Task history: one completed attempt with metrics/result evidence and a second nonterminal task; omit requests in a legacy control and include one valid unknown-delivery request in a second control. Open without rewrite, verify owner filtering/stale-not-live projection, mutate an unrelated title under normal authority, reopen, and retain nested sentinel fields and original attempt identifiers.
2. Review history: one immutable captured record with file diff plus allocation, initially omit readers. Read exact owner/worker/evidence selectors with the original Git worktree absent; refuse wrong owner. Add an authorized reader or forget another record, then reopen and confirm retained evidence bytes/fields and authorization. Use the existing real-Git captured TS output, not invented schema fields.
3. Replacement history: raw version1 operation in activating/sending state plus launches and pendingMetadata, with pendingSignatures absent. Open, run existing recover_status, require recovery-required/bound=false/sending→uncertain, preserve ownership routing and nested authority data, reopen again and prove no repeated send. Do not replay a provider launch merely to prove import.

These are narrow additional provenance controls, not identified production repairs. Existing exact-candidate CI supplies current owning-test execution. Owner may instead make the final gate claim precise: supported retained representation audited, current restart/recovery proven, and no universal historical-version or unknown-root-extension guarantee. Such scoped wording must not be relabeled as an executed cross-writer fixture.

## Preparation update — current-writer import controls

The prep branch now supplies all three independent current-writer captures under
`services/hub-rs/assets/persisted-ts/`. They are exact bytes emitted by the retained
TypeScript stores, with source/byte hashes and explicit variable UUID/time/path
provenance. The review case uses real Git commits and is readable after deleting
its isolated Git directories. No production serializer or supported schema was
changed. `persistedWriterCapture.test.ts` passed3, the source-only capture mutation
suite passed4, and main TypeScript typecheck passed. The new Rust target
`persisted_ts_imports` contains3 actual TaskStore/ReviewStore/ReplacementState
import/rewrite/reopen controls, but **has not been compiled or executed locally**;
branch CI must supply that receipt before this closes the import-proof gap.
These are current retained TS-writer artifacts, not an arbitrary historical
release/user-installation compatibility claim.
