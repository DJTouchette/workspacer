# Core bus source review notes

This records source-level review alongside the assertion mappings in
`BUS_MIGRATION.md`. The completed exact-hash accounting is in `reviews/bus-core.json`; this note summarizes its boundaries.

| Retained implementation responsibilities | Rust owner and evidence |
| --- | --- |
| Frame envelope, nullable parameters/results, hello vocabulary, 64MiB transport limit and protocol errors | `protocol.rs`, `server.rs`, shared bus corpus replay on both embedded/network adapters and HTTP authentication tests |
| Scoped versus plugin versus host provenance, separate provider registration grants, host-only desktop/upload methods, topic ownership | `auth.rs`, `admission.rs`, immutable actor peer identity; scope matrix, topic vocabulary, plugin namespace, upload provenance and private desktop-name tests |
| Host/Origin checks using the actual landed socket and explicit proxy hostnames | `server/policy.rs`, whole-router middleware, real-route HTTP sweep and wildcard-listener tests |
| Plugin handshake/revocation serialization, all live sockets closed, pending provider ownership released, scoped file revocation and grant narrowing | Actor `Command::Plugin`/`Command::Connect`, `Core::disconnect`/`sweep`, `Connection::recv`; deterministic both-ordering tests, actual network closure and disk revalidation tests |
| Subscription/demand caps and matching, replay/release by distinct interested clients, no metadata leak for disallowed streams | `Core::topics`/`publish`, broker and bus actor tests, topic256/257 network boundary and permitted-desync floor |
| Local handler replacement/precedence, truthful provider registration acknowledgement, first-owner ownership, health inventory, outage diagnostics | Unified `Options.handlers` map, actor provider map, existing no-provider diagnostic subprocess test and new real-wire local/provider tests |
| Pending-call ownership/correlation, provider disconnection, per-operation deadlines, independent slow readers/writers | Actor pending map/reliable queues, lifecycle timeout tests, provider result forgery corpus, broker slow-peer and cancellation tests |
| Spawn/progress/replay provenance cleanup, canonical authority keys and local/federated shared admission | `admission::sanitize` called before dispatch branching, shared key corpus and real-peer progress tests; cfg(test) third-sanitizer real two-hub regression requires local/source/destination pass provenance |
| Routing capability/freshness gate and decision correlation | Owned coordinator and external-provider gate now record one routing-phase outcome per operation. Qualified/booked source routing, canonical model carriers and paired local-project versus remote-execution cwd have passed real-peer and paired-origin tests. |

Explicit architecture differences must remain visible in eventual core-file
reviews:

- All Rust network listeners require a host credential; anonymous Go loopback
  harness startup is intentionally replaced by tested refusal. In-process host
  ownership remains available without a socket.
- A nonempty invalid Authorization header is authoritative and fails closed; it
  cannot fall through to a valid query token. This is explicitly covered by the
  current Rust HTTP tests.
- Rust uses one identity-aware handler map configured before startup, rather
  than separate Go plain/identity maps. The two old registration signatures do
  not survive as separate runtime APIs.
- Token/identity replacement closes existing immutable peers. It does not keep
  an earlier grant snapshot alive after registration metadata changes.
- Rust's actor uses bounded reliable/event queues and closes a saturated reliable
  consumer; Go uses per-connection forward goroutines and socket write locks.
  Assertions concern preserved ordering/correlation, independence of healthy
  peers, and bounded failure—not identical goroutine or buffer internals.
- Local Rust handlers have bounded operation deadlines and owned shutdown.
  Go's local callback goroutine itself had no provider timer. Cancellation and
  negotiated provider context are additive Rust protocol facilities.

Completed source accounting is recorded in `reviews/bus-core.json` with separate
responsibility maps for the connection/HTTP/event plane and the RPC/admission
plane. Both Go files were read in full, their production wiring and callback
consumers were traced, and the Rust actor/auth/server/service owners were
compared. Each responsibility names concrete assertions; aggregate receipts
alone are not the justification.

Bounded residual reviews remain linked in `reviews/bus-hostauthority.json`,
`reviews/bus-freshness.json`, `reviews/bus-sanitizer-drift.json` and
`reviews/bus-ceiling.json`. The latest library checkpoint passed 369 tests and
the new real WebSocket `bus_ceiling` target passed. Performance has its separate
optimized CI receipt in `reviews/bus-latency.json`.

Additional explicit differences in the final plan include total resource bounds,
strict malformed credentials/origins, fixed production scope lists rather than
arbitrary injected grant fixtures, stronger execution-provider receipt ownership,
and refusal when routing cannot name a safe concrete model. These are not claims
of byte-identical arbitrary Go callback behavior. `agents.dispatchPrepare` only
allocates a credential-bound lease; the actual worker spawn has its own retained
policy gate. No Go source is deleted by this accounting.
