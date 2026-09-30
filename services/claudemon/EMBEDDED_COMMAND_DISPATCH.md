# Owned embedded command dispatch

The command owner polls at most eight router futures concurrently, with the
existing 64-entry admission queue and thirty-second caller/dispatch deadlines.
Each request retains its own reply channel. There are no per-request spawned
tasks: `FuturesUnordered` lives inside the existing command owner.

The change removes a demonstrated serialization bottleneck. A gated route
prevented an unrelated embedded read from completing even though a real HTTP
request against the same router completed. The regression released the gate
and joined both owners before asserting failure, distinguishing a blocked
dispatcher from a broken route. This is causal concurrency evidence, not a
general throughput or latency benchmark.

Ownership remains explicit:

- A closed queued reply is skipped before dispatch.
- Dropping a caller after dispatch begins does not pretend to undo a possible
  effect. The operation finishes under its existing deadline; lost replies
  remain uncertain and must not be automatically replayed.
- Closing the sender side drains already admitted work.
- Aborting and joining the command owner drops its active dispatch futures
  and queued replies. Existing engine ownership still governs work spawned by
  the actual API handlers; this change introduces no detached request tasks.
- Full admission remains an immediate “not submitted” error. Successful JSON,
  definitive refusal and ambiguous failure classification are unchanged.

Five private unit fixtures use phase gates, receipt barriers, execution/drop
counters, queue saturation and a paused clock. They cover a fast sibling,
eight active requests plus queue pressure, cancelled queued/started callers,
draining sender closure, abort cleanup and the unchanged thirty-second limit.
The fixture is included only under `cfg(test)` from `tests/support`, so its
fake routes are not part of the daemon API inventory.

Validation on Linux:

- Before: the fast-sibling regression failed as expected after the explicit
  gate release (`/tmp/workspacer-command-concurrency-before-stripped.log`).
- After: five control tests and two existing response-classification tests
  passed (`/tmp/workspacer-command-concurrency-after.log`).
- Full claudemon library: 905 passed, zero failed, four ignored
  (`/tmp/workspacer-command-full-lib.log`). The ignored checks require real
  Codex/Claude credentials or read real Codex/Copilot home directories.
- The native-launch argv golden and first-publication control both passed;
  the golden also passed five separate repetitions, each selecting one test.
- Desktop route inventory: seven passed without new exclusions.

An initial direct-manifest build exhausted disk before running tests; it is
not red-test evidence. This manifest does not inherit the hub's stripped dev
profile. Local owning checks used `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0`, preserving dependency versions and the source
test behavior. Hub integration and final platform CI receipts are separate.
