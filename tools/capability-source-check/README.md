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

Local function declarations use their lexical declaration scope, including
forward declarations and shadowing. Known empty key arrays perform no reads;
unknown conditional arrays retain every possible key. `inspectionDecisions`
separately permits reviewed map-key validation at a caller path, only when the
trace actually observes key inspection and no payload transformation there.
It cannot authorize serialization, value inspection, storage, or forwarding;
those still require a full opaque review or fail as unsupported flows.

`go-reference.json` captures all original Go dispatch bindings, including its
84 dangerous method/field pairs and byte hashes for the original scanner,
production inputs and vocabulary. `go-reference-provenance.json` explicitly selects
version1 captured-provenance mode and pins the original Git commit/tree plus the
SHA256 of the unchanged capture. Its own digest is pinned in `reference.rs`.
The Rust check verifies both seals and requires each original dangerous binding
independently. Only historical source paths listed in that exact sealed capture
may be absent; any originals still present must match their captured hashes.
The current vocabulary, production Rust sources, desktop source guards and all
84 original dangerous bindings remain required. This caught a hidden
MCP branch that an aggregate field-count check would have missed. Go is needed
only to recapture this optional reference, never for routine checking:

```sh
go run ./tools/go-capability-reference/main.go --root . > tools/capability-source-check/go-reference.json
go run ./tools/go-capability-reference/main.go --root . --check tools/capability-source-check/go-reference.json
```

Historical verification is an explicit operation, separate from normal portable
checks. Supply a checkout at the exact reference commit (including its original
source bytes); HEAD and tree identity and every original hash are checked:

```sh
cargo run --locked --manifest-path tools/capability-source-check/Cargo.toml -- \
  --root . --verify-go-reference /absolute/path/to/pinned-reference-checkout
```

`--verify-go-reference` does not run the old Go tests or claim their execution.
The optional Go recapture commands above must use that historical checkout as
`--root` after deletion. To change a capture deliberately: verify the old seal,
choose and review the new immutable commit/tree, reproduce capture against that
checkout, review all source/binding differences (never lower the84 floor), update
the provenance capture hash and commit/tree, then update the explicit manifest
seal constant in `reference.rs` after review. Preserve LF bytes. Run both strict
historical verification and normal `--check`, mutation tests and desktop guards.
A missing/corrupt seal or an unknown source path never enables a fallback.
This preparation does not authorize source deletion or satisfy release gates.

This is a bounded source guard, not general Rust type inference or runtime
security verification. Test-only module ancestry is excluded, while unknown
platform cfg branches remain included. Map/recursive transformations are
explicit opaque evidence, with separate reviewed policy; unknown external
input receivers, dynamic untyped indexes and unsupported serde shapes fail.
Known scalar conversions stop JSON-key provenance. The helper depth limit
still fails rather than silently declaring an input inert. Runtime boundary
and behavior tests remain necessary.

Spawn-key closure is checked separately from dangerous-parameter policy.
`contracts/spawn-parameter-keys.json` is the runtime's authoritative spelling
registry: it preserves all46 historical keys plus5 reviewed reservations and
must cover every source-traced `agents.spawn` root. The historical Go vocabulary
is unchanged. The desktop AST guard checks the same registry. Membership only
reserves spelling; it does not authorize a parameter or enable an operation.

Optional full historical Go suite, cross-stack parity, routing harness and
vocabulary comparison commands now share `scripts/hub-reference.py` and require
`WKS_HUB_REFERENCE_ROOT`. See [historical commands](../../scripts/reference/README.md).
Those commands require a clean pinned checkout; routine source checking remains
independent of Git and Go.
