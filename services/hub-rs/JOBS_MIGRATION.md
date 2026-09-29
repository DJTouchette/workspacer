# Jobs migration evidence and deliberate differences

The Rust service owns specs, history, scheduling and execution. There is no
separate scheduler process. Its production loop retains the 30-second tick and
hub-local daily schedule; a private clock/tick seam exercises the real loop in
tests without waiting minutes or launching models. Supported triggers remain
interval, daily, once and manual—there is no cron-expression parser.

The reference tests cover content-based reloads (including same size and mtime),
missing and malformed files, schedule anchors, duplicate/missing identity
writeback, edits observed by the live scheduler, owner writes merging an external
edit, proposal review/limits/notifications, context vetoes, invocation overlap,
once disarming, history limits/restart, and docs/preset validity. The Go corpus
adapter remains in `tests/jobs.rs`; captured authoring cases retain their source
hash and are validated by Rust rather than requiring a Go runner.

Explicit valid JSON `null`, `{}`, `{"jobs":null}`, and `{"jobs":[]}` clear the
schedule, matching actual Go decoding. Missing, empty, whitespace-only,
truncated and otherwise malformed documents keep the last good state. Nullable
scalar spec fields use Go zero values: in particular `enabled:null` is false,
not true; `days:null` is an empty day filter. Nullable invalid rows still fail
normal job validation. Empty optional history fields survive a save/reopen.

Canonical field spelling is intentionally stricter than Go's case-insensitive
JSON decoder. Known case variants, including the long-s/Kelvin ASCII fold
aliases, are refused so they cannot silently erase an approval label or guard.

`skipUnlessMatch` uses a Go-syntax adapter over Rust regex. The optional
`tools/go-regex-reference` CLI captures independent reference cases and accepted
property names; builds and runtime do not need Go. Its metadata records the
reference Go/Unicode versions. ASCII shorthand classes, boundaries, quoted
literals, class syntax and repetition shapes/budgets have direct oracle cases.
**Unicode property membership and case folding use the Rust library's Unicode
tables**, which may be newer than the Go 15.0 tables used by the capture. Neither
the name inventory nor this audit claims equality of every Unicode codepoint or
formal equivalence of the two engines for every possible pattern.

Shell capture preserves stdout/stderr ordering in one bounded pipe and shares
owned Unix group/Windows Job cleanup. Cancellation does not detach a blocking
pipe reader. Commands scrub ambient hub/facade authority variables, consistent
with other owned launches; Go inherited them. Output over 64 MiB fails instead
of consuming unlimited memory. Text caps slice at byte offsets and replace any
split UTF-8 sequences with replacement characters via `String::from_utf8_lossy`;
they do not preserve character boundaries or promise byte-identical Go strings.
Windows shell source uses cmd's raw quoting
rules; its real executable-and-path-with-spaces fixture requires Windows CI.
These are explicit behavior/ownership differences, not byte-identical process
or output claims.

Manager review on 2026-09-29 reran the full Rust library plus the jobs and
quiescence integration targets: 311 + 8 + 3 tests passed, with no ignored tests.
Command (using the existing local Cargo cache):

```bash
CARGO_HOME=/tmp/workspacer-hub-cargo CARGO_TARGET_DIR=/tmp/workspacer-hub-target CARGO_INCREMENTAL=0 cargo test --locked --manifest-path services/hub-rs/Cargo.toml -j2 --lib --test jobs --test quiescence
```

The corresponding evidence is in `reviews/jobs-remaining.json` and
`reviews/quiescence-watcher.json`. Witness selected only the watcher unit tests
and left jobs/source files unmapped; the explicit target run above supplies the
broader check. This Linux run does not close the Windows runtime gate.
