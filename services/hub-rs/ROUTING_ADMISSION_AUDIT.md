# Routing admission audit restoration — 2026-09-29

The retained Go bus writes one routing audit row for every spawn reaching its
ceiling/freshness gate, including allowed, clamped and refused requests. Rust's
`RoutingService::audit_spawn` existed but had no production caller. This left
`decisionId` without its spawn-side receipt even when policy correctly refused
or changed a request.

The restored path uses `SpawnAudit` and the existing best-effort local append
sink. It records the actual caller scope/fingerprint, decision ID, canonical CWD,
allowlisted model/routing fields, changed field names and ceiling/freshness
outcome. Prompt, environment, arbitrary request objects and bearer credentials
are not copied. The record explicitly says `phase: routing`: it does not certify
that the provider later accepted or completed work. Sink failure remains an
availability diagnostic and does not widen or narrow launch authority.

The external control-plane provider path performs its gate once and writes its
receipt before forwarding. The owned spawn coordinator resolves once before slow
setup and again afterward; one operation-scoped guard coalesces those checks into
one row, retaining the last resolution's effective tuple and restrictions. The
additional effective-profile check contributes to that same row, including a
profile override refusal. No background logger or independent service lifetime
was introduced. Local file append remains the existing best-effort synchronous
sink, matching the actual retained Go sink's single-append implementation.

Freshness refusal now names the requested resume session and `routing.yaml`.
Exact-model refusal identifies the policy and explicitly says no substitute was
launched. These are caller-visible retained regression assertions.

Validation: the full library suite passed 334 tests; `agent_spawn` passed 15,
including owned allow/clamp/refusal bookkeeping. The new runtime regression
exercises the actual external-provider admission path and checks exact one-row
counts, denied-call non-forwarding, decision correlation and secret omission.
The separate latency target's two deterministic tests passed; its optimized
measurement remains explicitly unexecuted locally.

Remaining: source-side policy before qualified federation spawn still needs its
own audit/fix. The current runtime only applies this external-provider gate to a
local bare method; the receiver's policy is a different authority from the
origin's policy. Therefore the complete Go `spawnfresh_test.go`,
`spawnceiling_test.go`, `rpc.go` and `bus.go` rows are not certified by this change.
