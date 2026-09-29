# Capability caller-parameter source guard

This isolated, source-only Rust tool parses production capability registrations
and traces their caller payloads through local helpers, typed serde targets,
receiver fields, closures and map reads. It runs no providers and changes no
runtime permission policy. The shared decisions are in
`apps/desktop/tests/fixtures/capability-parameter-policy.json`; the dangerous-name
vocabulary golden remains independent.

From the repository root:

```sh
cargo fmt --manifest-path tools/capability-source-check/Cargo.toml --check
cargo test --locked --manifest-path tools/capability-source-check/Cargo.toml
cargo run --locked --manifest-path tools/capability-source-check/Cargo.toml -- --root . --check
# Diagnostics:
cargo run --locked --manifest-path tools/capability-source-check/Cargo.toml -- --root . --method agents.spawn
cargo run --locked --manifest-path tools/capability-source-check/Cargo.toml -- --root . --summary
```

`--check` requires every current full/catalog method to be registered and
classified, every detected dangerous binding to have a decision, every opaque
caller path to have an explicit `opaqueDecisions` review, and every traced
unsupported flow to remain visible as an error. An opaque review applies to
its exact path and descendants; `$` explicitly reviews the whole method payload.
A decision on an unrelated field cannot excuse it. Novel suspicious spellings
need a per-method `sourceParameterDecisions` entry rather than automatic admission to the vocabulary. These source-specific entries must match an actual traced field; the older cross-plane `parameterDecisions` retain their vocabulary-only invariant.

`go-reference.json` captures all original Go dispatch bindings, including its
84 dangerous method/field pairs and byte hashes for the original scanner,
production inputs and vocabulary. The Rust check verifies those hashes and
requires each original dangerous binding independently. This caught a hidden
MCP branch that an aggregate field-count check would have missed. Go is needed
only to recapture this optional reference, never for routine checking:

```sh
go run ./tools/go-capability-reference/main.go --root . > tools/capability-source-check/go-reference.json
go run ./tools/go-capability-reference/main.go --root . --check tools/capability-source-check/go-reference.json
```

This is a bounded source guard, not general Rust type inference or runtime
security verification. Test-only module ancestry is excluded, while unknown
platform cfg branches remain included. Map/recursive transformations are
explicit opaque evidence, with separate reviewed policy; unknown external
input receivers, dynamic untyped indexes and unsupported serde shapes fail.
Known scalar conversions stop JSON-key provenance. The helper depth limit
still fails rather than silently declaring an input inert. Runtime boundary
and behavior tests remain necessary.
