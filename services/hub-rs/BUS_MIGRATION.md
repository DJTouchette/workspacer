# Remaining bus audit — 2026-09-29

The starting ledger has 27 pending files in `services/hub/internal/bus`.
This audit reads the retained assertions and the Rust authority/actor/transport
paths before assigning evidence. `bus.go` and `rpc.go` are not certified merely
because their callers' tests pass. The new `tests/bus_remaining.rs` adds eight
focused integration cases; it does not introduce a second bus implementation.

## Reviewed behavior groups

| Retained suites | Replacement evidence and exact behavior |
| --- | --- |
| `bus_test.go`, `local_test.go`, `localident_test.go` | `tests/bus_remaining.rs` tests a real WebSocket calling local handlers, last registration wins with one inventory name, exact local errors/correlation, handler-produced events, generated event identity/time, stable credential fingerprints and owner/operator/triage/peer provenance. `tests/http.rs` checks public liveness versus authenticated method names. `runtime/broker_tests.rs` checks exact filtering/duplicate subscriptions/payload stamping. |
| `rpc_ownership_test.go` | The new local-handler test proves shadowed registration is withheld and local results win. It checks sorted unique health inventory/count. `runtime/bus_audit_tests.rs::missing_provider_diagnostics_are_once_per_outage_and_never_log_parameters` separately proves repeat-call suppression, distinct-method logging, and re-logging after provider loss. |
| `topiccap_test.go` | A new WebSocket case rejects 257 subscriptions without retaining any, admits exactly 256, rejects oversized unsubscribe without changing membership, and delivers an allowed event afterward. Existing broker tests also enforce the aggregate subscription bound and large-frame processing guard. |
| `machine_stop_test.go` | `runtime::machine_stop_tests::stop_drains_interactive_sockets_with_4001_but_keeps_infrastructure` observes an actual WebSocket close code/reason, waits for physical drain, and proves providers/plugins/internal callers remain live. |
| `migration_test.go` | `tests/compatibility.rs` replays the same `contracts/hub-bus-cases.json` scenarios against embedded and WebSocket transports, including first-provider ownership, forged-result refusal, disconnect failure, local precedence, literal wildcard semantics and scoped denial. |
| `providertier_test.go` | The new provider test proves `provides` does not grant operator calls, rejects unauthorized registration and host topics, and admits an authorized snapshot even when the local handler shadows its register slot. Existing auth/bus tests cover the complete topic vocabulary, no provider event consumption/desync side channel, and live provider-grant narrowing/slot release. `plugin_manager` HTTP tests deny provider installation authority with a functioning owner floor. |
| `pluginambient_test.go`, `profilegrant_test.go`, `yologrant_test.go` | New plugin and control-plane-provider tests keep ambient ordinary path calls, retain public task/profile/tool-scope metadata, reject forged host events/foreign provider names and remove caller-supplied grant stamps. Raw non-object spawn payloads survive to the configured external provider. Local owner lineage survives; operator/plugin/peer lineage does not. Existing admission tests separately preserve explicitly provisioned facade authority. |
| `reportprogress_test.go` | New real local dispatch covers untrusted stripping versus host/operator passthrough and unchanged public note/decision fields. Existing admission tests cover case variants; `tests/federation.rs::router_checks_both_scopes_and_sanitizes_before_the_remote_hop` checks stripping before a real peer hop. |
| `revoke_e2e_test.go`, `delivery_layers_test.go` | New tests observe physical closure of every live plugin socket, reject reconnects, release provider registration, and execute both actor-serialized admission/revocation orderings. Queued PTY bytes and generated desync metadata are suppressed after revocation. Existing actor tests prove closed identities cannot enqueue, dispatch or retain protected desync bookkeeping before eviction. |
| `hostpin_test.go` | `server/policy.rs` preserves the exact host/actual-socket matrix; `tests/http.rs` probes every registered hub route. `plugin_manager::ui_settings_require_a_current_plugin_credential` checks owner/own-plugin versus foreign/invalid credentials in header/query forms and immediately removes settings entitlement after direct authority revocation. |
| `routinghost_test.go`, `scoped_token_test.go` | New identity and scope-matrix tests distinguish authenticated host, scoped operator and peer link, preserve hello scope/method vocabulary, and require scope-named refusal even when the unknown method is installed. Existing runtime and HTTP tests cover the broader registration/event/HTTP policy. |

The exact-hash review plan `reviews/bus-remaining.json` proposes 18 retained test
files using these individually reviewed assertions. Nine rows remain pending as
listed below. The root owns applying the plan and the migration ledger; this
audit does not certify either large production file from aggregate test counts.

## Architecture differences that must stay explicit

- Rust `Hub::start` requires an explicit host token for every bus or MCP network
  listener. The Go test harness often starts an anonymous loopback server and
  exposes detailed public health. That harness mode is deliberately unavailable
  in Rust; the new startup test pins the refusal, while authenticated network
  tests retain the useful transport assertions. Embedded local owners still work
  without a network credential. Do not "fix" parity by silently allowing an
  uncredentialed socket.
- Rust has one identity-aware handler map configured before startup. Go's
  separate plain/identity handler maps and runtime registration functions collapse
  to that single owner. The last configured handler wins and is counted once.
- Rust serializes identity admission, revocation, provider ownership and event
  admission in one actor. A changed scoped authority closes its old immutable
  peer rather than mutating the identity of a live event consumer. The receive
  path prioritizes that close over queued bytes/desync frames. Preserve both
  admission-time rejection and actual receiver closure; an event test that merely
  observes silence is insufficient.
- The external `agents.spawn` forwarding path is intentionally available only
  with `control_plane_only` and an actually registered provider. Full standalone
  mode requires its owned execution service. The new test configures the real
  control-plane path to inspect forwarded grants; it does not bypass admission
  with a fake local spawn callback.

## Explicit remaining work

- `bus.go` / `rpc.go`: complete source-level audit of the whole live connection,
  routing and ownership surface. Focused suite mappings alone do not certify these
  large production files.
- `bench_test.go` and the two `raceflag_*_test.go` helpers: the legacy file includes
  an executable mature-snapshot performance budget, not only optional benchmarks.
  It measures 2,000 hub hops and a 500-hop bare-WebSocket floor, enforces a 5ms p99
  hub-share budget, and explicitly skips Windows/race/unmeasurable environments.
  A Rust replacement or reviewed retirement decision is still needed.
- `spawnceiling_test.go` / `spawnfresh_test.go`: fully map routing rejection,
  canonical/unresolvable CWD, selected-model/provider/effort substitution, audit
  records, exact-model refusal, fresh-role resume refusal and federated execution
  to their actual runtime guards. Do not substitute generic spawn success tests.
- `sanitizerdrift_test.go`: Go injects a third sanitizer and demonstrates that both
  dispatch paths execute it. Rust shares `admission::sanitize` before branching,
  and existing real peer tests cover progress, but the extensibility regression
  still needs an equally meaningful structural or mutation guard.
- `hostauthority_test.go`: guarded HTTP behavior is covered, but the old
  `ScopedIdentFor` label/scope diagnostic contract needs a precise equivalent or
  explicit retirement review; a generic 403 alone does not prove that assertion.

## Verification

Retained Go bus suite: `go test ./internal/bus -short -count=1` passed. The legacy
latency measurement is deliberately not executed by `-short` and is not certified.
Replacement integration runs passed: `bus_remaining` 8, `auth` 7,
`compatibility` 4, `federation` 5, `http` 2, `plugins` 1, `plugin_manager` 21 and
`lifecycle` 8: 56 tests. The separate Rust-launched Go-reference case remains
explicitly ignored; the Go suite above independently replayed the same fixture.

The library checkpoint executed 330 tests: 329 passed, including all named bus,
admission, server and machine-stop tests. One unrelated in-progress model decoder
test failed; its owning agent is correcting and rerunning it. This audit does
not report that library invocation as green. Initial linker SIGBUS failures under
full disk ran no tests; those failures were followed by successful executable
runs and small target groups after scoped generated-artifact cleanup.

No production bus code was changed. The new tests use isolated temporary stores,
loopback listeners and in-process fake providers, with no real provider process,
live server or production credential.
