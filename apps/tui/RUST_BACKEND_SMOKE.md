# TUI / Rust backend boundary smoke

Build the shared backend once, then run from the repository root:

```sh
make test-tui-rust-backend WKS_RUST_BACKEND_BIN=/absolute/path/to/workspacer-rust
```

The named target invokes the otherwise ignored
`daemons::backend_smoke::real_rust_backend_calls_events_reconnect_and_owned_shutdown`
test. Missing or invalid supplied binary fails; the test never substitutes a
fake protocol server or silently skips. Normal TUI unit tests need no backend
binary. The hub CI job invokes this target after its own tests have built the
same backend executable; the ordinary TUI job remains independent and parallel.

The fixture creates isolated home/config/data/SQLite files and four loopback
listeners. It launches no model. Actual claudemon hook observations produce a
session, and the production TUI `BusClient` calls snapshots/conversation and
receives session events. The same client reconnects to a restarted backend on
the same endpoint and receives a new unique layout marker without a second
subscribe call. An existing-service bootstrap guard proves borrowing does not
stop the service.

Both owned lifetimes end by dropping the actual `Daemons` owner. A private
`cfg(test)` receipt records its real `Child::wait` result; the fixture requires
successful exit and all four exact ports closed. The receipt is absent from
release builds and changes no product shutdown behavior. Temporary state is
removed by an owned directory guard.

This is a transport/ownership cutover probe, not an interactive terminal-rendering,
external account/model, remote TLS or real phone test. It does not exercise the
full `ensure()` fixed-default-port discovery path; bootstrap selection and
preexisting-owner rules retain their separate unit tests. Passing this target
alone does not verify the migration's TUI gate.

Local checkpoint, 2026-09-30: the explicit probe passed against the already-built
`/tmp/workspacer-hub-target/debug/workspacer-rust` (two successful child exits and
four closed listeners per lifetime), recorded in
`/tmp/workspacer-tui-rust-backend-smoke.log`. The six ordinary daemon-owner tests
also passed; the explicit probe remains ignored only in that ordinary invocation.
TUI formatting and all-target clippy with warnings denied passed. These are Linux
checkout receipts; the new CI step must supply its own revision-bound result.
