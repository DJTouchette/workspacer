# Fixture sweep migration evidence

The Go `internal/sweepguard` package has three responsibilities: executed-case
accounting, process-wide host-gate enforcement, and monorepo file discovery.
Its three ledger entries remain pending; implementing one responsibility does
not prove the others retired safely.

## Executed-case accounting

`tests/support/sweepguard.rs` preserves Go Tally's verdict aliases, separate
allow/deny/other execution counts, skipped-case diagnostics, total enumeration,
population ratchet, and both/deny/every execution floors. Count calls belong
after fixture setup and before assertions. Missing assertions still fail the
test through their own panic; a setup that cannot run must never count as run.

The helper is used by real Rust consumers:

- `tests/files.rs`: all eight active containment cases, at least three allows
  and five denies. Previously an empty cases array could pass this test.
- `tests/stores.rs`: all twelve selected session filename cases, at least three
  accepts and nine refusals. Previously only fixture population was checked.
- `src/services/library.rs`: all seven library directory cases, at least three
  accepts and four refusals. Windows now attempts real directory symlinks;
  unavailable privilege records the affected case and error, then the executed
  refusal floor fails. It no longer silently omits `needsSymlinks` rows.

The tally tests mutate real fixture populations to empty, allow-only, deny-only,
skipped-denies and one-case-short forms. The library negative test injects a
permission failure into the real fixture setup: seven enumerated cases become
five executed and two skipped, and the floor fails with both named reasons.
These tests do not establish successful Windows symlink execution locally.
Windows CI must provide Developer Mode or the equivalent symlink privilege;
the existing containment and selected-session fixture tests already require
that capability. Run the library fixture on the actual Windows runner before
claiming its platform gate passed. A privilege failure is an explicit failed
coverage check, not evidence that the library implementation is incorrect.

Linux validation on 2026-09-29: `cargo test --test sweepguard --test files
--test stores` passed 4 + 5 + 8 tests; `cargo test --lib
selected_directory_contract` passed both library tests. Commands used
`--locked --manifest-path services/hub-rs/Cargo.toml -j2` with the existing
`/tmp/workspacer-hub-cargo` Cargo home and `/tmp/workspacer-hub-target` target.
Witness did not map this scope completely, so these explicit owning targets
were run. This is targeted validation, not a new whole-suite/platform claim.

## Remaining sweepguard responsibility

Do not record all Go sweepguard sources as ported based on this utility:

- Go GateCounter maintains distinct named tests, last-verdict-wins state,
  exact group counts, and sorted failure diagnostics. RunGates executes from
  TestMain even when a floor test disappears; it stands down for filtered runs
  and preserves an existing failing exit code. Rust libtest has no equivalent
  TestMain hook here. A replacement needs actual group/runner integration,
  including a failure test where every group member is skipped or removed.
- Go Root/ReadRepoFile distinguish missing checkout from renamed root markers
  and missing fixtures. Rust's migrated readers typically use compile-time
  `include_str!` or paths from `CARGO_MANIFEST_DIR`; missing files already fail,
  but a reviewed inventory must establish the replacement of every Go caller
  before retiring the old discovery and out-of-module Go cache machinery.

Keeping these rows pending prevents helper unit tests from masquerading as
proof that every migrated suite actually calls its enforcement layer.
