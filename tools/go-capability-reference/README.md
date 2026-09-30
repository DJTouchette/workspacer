# Original Go capability binding reference

A standalone standard-library-only AST oracle copied from the original
`services/hub/cmd/brain/capspec_params_test.go` scanner. It does not compile,
start, or depend on the Go hub. The original scanner hash is pinned in this
tool so edits require deliberate re-extraction and review.

Run from the current repository root, using the separately pinned historical
checkout described in [scripts/reference/README.md](../../scripts/reference/README.md):

```sh
export WKS_HUB_REFERENCE_ROOT=/absolute/path/to/pinned-reference-checkout
python3 scripts/hub-reference.py verify
go run ./tools/go-capability-reference/main.go --root "$WKS_HUB_REFERENCE_ROOT" --check tools/capability-source-check/go-reference.json
# Review a prospective recapture without overwriting the sealed current asset:
go run ./tools/go-capability-reference/main.go --root "$WKS_HUB_REFERENCE_ROOT" > /tmp/go-capability-reference.json
```

The reference preserves the original algorithm, including its depth-three
receiver-helper bound and flattened nested tags. It captures 84 dangerous
(method, parameter) bindings, not an assertion of exhaustive Go semantics.
Every non-test brain source input, the original scanner, and the independent
parameter vocabulary carry SHA-256 provenance. Rust's routine source guard
checks sealed captured provenance and individual live binding closure without
invoking Go or requiring deleted source paths. See the source-checker's README
for strict historical verification and deliberate resealing; changing a capture
is not accomplished by simply redirecting over the checked-in JSON.
