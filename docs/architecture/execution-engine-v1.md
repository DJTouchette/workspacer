# Internal execution engine v1

`services/claudemon/src/execution.rs` is the composition boundary used by daemon
PTY and managed starts, message/decision/answer controls, interrupt/stop, and
single-session snapshots. `ClaudemonEngine` delegates the existing native launch
functions in `daemon/spawn.rs`; those functions retain the provider argument,
facade, profile and transport construction. Desktop and headless callers use the
same daemon routes. Externally attached wrappers retain their existing lifecycle.

Only the compiled `claudemon-v1` registration exists in production. The replay
implementation is inside `cfg(test)`. There is no engine selection request field,
configuration setting, plugin loader, MCP tool or workflow authority. The host
still admits the request, clamps model/permission ceilings and mints the facade
token. Trusted native implementation code retains its previous privileges; this
interface is not an OS sandbox for untrusted code.

## Persistence and compatibility

API version `1` identifies this internal interface. Implementation version `1`
identifies the known compatible claudemon storage lineage. Compatible releases
retain that value regardless of build hash. A future incompatible implementation
must use a distinct version and provide an explicit migration before resuming.
Unknown IDs, incompatible explicit versions, and malformed stored metadata refuse
admission. There is no implicit fallback or live migration.

`execution_leases` uses the existing SQLite connection. It is separate from the
`sessions` row because managed admission can precede the first hook that creates
that row. The lease is persisted before session publication with a compare-and-set
against the prepared predecessor, and its generation increments on each admitted
lifetime. Missing legacy metadata maps to the native lineage at generation zero;
the first new admission persists generation one. Hydration reports incompatible
pins as `unavailable` without losing the rest of the session list. Nullable,
additive metadata does not advance the existing SQLite schema version.

Snapshots carry only ID, API/implementation versions, generation and readiness.
Desktop, headless brain and TUI preserve this metadata; older peer snapshots omit
it rather than claiming support. Readiness describes compatible implementation
availability, not authentication or model health. Startup preflight also checks
that the requested executable exists and is executable; spawn-time native errors
still follow the existing driver error/cleanup path.

## Lifetime, events and retries

Each admission captures a registration and each command captures its binding.
Retirement waits for an in-flight command; delayed commands against a retired
binding refuse. Disabling selection prevents new admissions, including a prepared
start that has not yet acquired a lease. Existing bindings retain their engine.

Native normalized `AgentUpdate` batches use a scoped collector. A retired or
terminal engine scope cannot project another batch. Explicit replay sequence
numbers reject duplicates and out-of-order batches. Native transports keep their
existing sequence/deduplication semantics: the collector does not invent IDs for
provider events. Native Codex generation/descendant containment stays in place.
Provider-specific writes outside `apply_updates`, hooks, and raw PTY output remain
owned by their existing native guards; this change does not make those feeds an
engine-neutral durable event log.

The current daemon message request has only `text`, with no request identity.
Repeated sends therefore remain repeated sends. Approval channels retain their
native request semantics. A duplicate active session start refuses rather than
starting a second driver; a stale prepared start fails its durable compare-and-set.
There is no new end-to-end exactly-once command-delivery guarantee.

The native driver finalizer disposes its engine scope once. Existing generation
checks and stopped-state checks suppress duplicate terminal publication. Internal
bounded drain stops the binding and waits until its deadline. Timeout is explicit
and retains the pinned handle; it does not falsely report that a process died.
Provider child/process-group termination remains native-adapter-owned.

## Validation and remaining seams

Automated replay tests exercise actual registry admission, normal store commands,
normalized conversation projection, version pins, disable, sequence fences,
terminal deduplication, and disposal without provider processes or credentials.
A shared native-launch golden fixture captures argv from fake executables for all five providers through the production registry. Existing native provider/HTTP/store suites exercise delegation and containment.
Client compatibility tests preserve metadata and preserve absence from old peers.

This proves an internal replacement of execution dispatch and normalized-update
consumption. It does not prove replacing every native provider-internal mutation,
forced termination of arbitrary uncooperative engines, durable event replay,
third-party isolation, or live transfer.
Those are remaining seams, not claims of this implementation.

## Baseline validation finding

The renderer `CAP_LABELS` hub-native drift test failed at baseline `93c75191`:
`routing.preferences.get`, `.validate`, `.save`, `.reset`, and `routing.preview`
had no consent labels. The separate, localized renderer consent-label repair now
labels those existing routing methods; it does not change the engine or routing
authority boundaries. Engine snapshot/folding tests pass.
