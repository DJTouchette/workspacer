# Original Go capability binding reference

A standalone standard-library-only AST oracle copied from the original
`services/hub/cmd/brain/capspec_params_test.go` scanner. It does not compile,
start, or depend on the Go hub. The original scanner hash is pinned in this
tool so edits require deliberate re-extraction and review.

Run from repository root:

```sh
go run ./tools/go-capability-reference/main.go --root . > tools/capability-source-check/go-reference.json
go run ./tools/go-capability-reference/main.go --root . --check tools/capability-source-check/go-reference.json
```

The reference preserves the original algorithm, including its depth-three
receiver-helper bound and flattened nested tags. It captures 84 dangerous
(method, parameter) bindings, not an assertion of exhaustive Go semantics.
Every non-test brain source input, the original scanner, and the independent
parameter vocabulary carry SHA-256 provenance. Rust's routine source guard
checks both provenance and individual binding closure without invoking Go.
