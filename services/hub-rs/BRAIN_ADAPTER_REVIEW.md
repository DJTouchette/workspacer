# Brain bootstrap and daemon adapter

Reviewed the complete `cmd/brain/main.go` (164 lines) and `claudemon.go`
(528 lines). These rows account for composition and the daemon boundary;
they do not certify every registry handler in `handlers.go`.

## Ownership and startup

Full standalone/native composition owns an `EmbeddedDaemon` through `Backend`.
Runtime installs native services, journals, lifecycle admission, session
projection, terminal leases, demand streams, file/library observers, threshold
sweeps and wake backstops. There is no separate brain process or private Node
companion to launch or close. The parent signal/PID/stdin protocol belongs to
the standalone launcher; the embedding GUI explicitly owns shutdown.

The session service subscribes before its authoritative seed, reconciles
launch state, primes finish observation and then observes typed updates.
Lag reloads authoritative terminal state without importing unrelated retained
history. Finish/wake processing precedes sidebar visibility filtering. Status
lines have their own typed subscription, and conversation demand owns its
receiver and resynchronization generation. Cleanup stops admission, drains
queued replies and joins observers before the owned engine is joined.

Upstream workers use explicit `provider_relay::Scope::{Full,Catalog}` and
reviewed export sets intersected with installed methods. Their named
`brain.info` receipt identifies the scope/node. Unknown scope values are
refused rather than silently becoming full scope. Electron now supplies both
its catalog and live methods (`DELEGATE_CATALOG_TO_BRAIN=false`) behind a
hub-only control plane. This avoids running a competing catalog provider.

`ExternalDaemon` is a separate, explicitly borrowed hub-only observation link:
event bridging, usage and model catalogs, with bounded reads and joined
cancellation. It never becomes a remote execution engine or assumes ownership
of that daemon. Its event bridge is the replacement for the old lightweight
hub bridge, not a substitute for the full brain's seeded session store.

## Request and response mapping

| Old adapter | Owned implementation |
| --- | --- |
| Session list, archived list, complete list | Typed `Command::Sessions` or fixed `/sessions` query flags in sessions, recent/history and lifecycle reconciliation. |
| Transcript cwd, conversation cursor, child thread | Fixed relative router paths with encoded cwd and validated path segments; JSON values retain sparse/unknown fields. Child reads still pass through the daemon's parent-exposure check. |
| PTY/managed spawn DTOs | `spawn_plan::Plan` constructs the same endpoint payloads; lifecycle checks pinned identity, canonical model/window fields and `first_message_queued` before claiming delivery. Managed resume, permission, env, instructions and extra arguments remain provider-specific. |
| Message, raw input and bytes | Same daemon message/input handlers; raw input retains base64 and `newline:false`. The registered caller validates known fields before transport. Definite refusal and uncertain outcome remain distinct. |
| Gate, approval, question answer, resize and signal | Same owned router endpoints. Gate defaults remain false for omitted/null. Approval omits empty reason. Answer precedence runs only after all known fields validate; null entries in the two Go string-list DTOs become empty strings. Resize and SIGTERM retain their actual engine effects. |
| Permission/model/handoff receipts | Native live controls project confirmed daemon receipts, preserve queued model disposition and requested model/window pairs, and return the daemon's handoff path/markdown. An old daemon's unsupported PTY-model error translation is retired because full mode owns the matching engine version. |
| HTTP GET/POST response bodies | In-process `Router::oneshot` uses the real router and limits. It returns parsed JSON, not byte-identical whitespace or object order. Actual producer endpoints used by this adapter return JSON; malformed/non-JSON success is refused rather than guessed. Responses are bounded to 32 MiB instead of unbounded reads. |
| HTTP failures and deadlines | Requests have bounded admission and thirty-second waits. Definitive refusal statuses are typed; an ambiguous server failure after an effect stays unknown. A successful JSON `ok:false` is a refusal, not a successful write. Missing/failed transport lookup no longer guesses a PTY and attempts keyboard input. |

The obsolete full-brain HTTP base and arbitrary facade URL are replaced by
owned engine/facade configuration. Upstream hub identity and provider/caller
credentials remain explicit. Logs do not regain the old raw daemon URL or
child-secret authority through this migration.

## Evidence and limits

`tests/engine_adapter.rs` uses a real owned engine and registered handlers plus
an inert wrapper WebSocket, without launching a provider. The before receipt
captured four malformed gate values being accepted, six malformed answer
carriers emitting PTY input, four adjacent field-validation gaps, and rejection
of a valid Go null-string-list default. The after receipt passes, with positive
gate/input floors, null/default/precedence controls and a same-stream marker
that proves invalid input did not remain queued.

- Adapter regression: `/tmp/workspacer-engine-adapter-after.log`, 1 passed.
- Owned seed/name projection/shutdown: `/tmp/workspacer-embedded-spine-final.log`,
  1 passed.
- Prior actual claudemon response classification:
  `/tmp/workspacer-engine-outcome-tests.log`, 2 passed, 898 filtered. This was
  a dependency-crate test run, not inferred from a hub library run.
- The earlier 391-test hub checkpoint includes typed lag, live-stream demand,
  external observer cancellation and wake-backstop tests. These are historical
  supporting receipts, not a full-current-source CI claim.

A subsequent Unix-only `local_spawn` integration passed with real registered
stream-answer numeric-kind and child-conversation paths
(`/tmp/workspacer-managed-adapter-proof.log`, 1 passed). Inert local Claude and
Codex fixtures preserve two numeric text answers plus an option-kind positive
control, enforce parent exposure before reading child rollout data, and verify
provider process cleanup. This complements the portable inert-wrapper proof;
it is not a Windows managed-provider execution receipt.
The subsequently reproduced serial-command stall is addressed by a bounded
eight-future dispatcher; see `../claudemon/EMBEDDED_COMMAND_DISPATCH.md` for
queue/deadline/cleanup semantics and its separate 905-pass library receipt.
No general throughput or latency claim follows from the causal gate test.
