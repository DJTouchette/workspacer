# Optional historical hub oracle

Ordinary Rust builds, desktop tests, and portable source guards do not invoke
this wrapper or require Git/Go history. These commands deliberately execute the
old Go implementation only when requested. They are not the shipping backend.

Set `WKS_HUB_REFERENCE_ROOT` to a separate, clean repository checkout at the exact
commit in `tools/capability-source-check/go-reference-provenance.json`:

```sh
export WKS_HUB_REFERENCE_ROOT=/absolute/path/to/pinned-reference-checkout
python3 scripts/hub-reference.py verify
make test-hub-reference
make test-hub-parity
make test-routing-harness
make hub-vocabulary
# Optional export to stdout for review; no fixture is overwritten by the tool:
python3 scripts/hub-reference.py vocabulary-export > /tmp/historical-vocabulary.json
```

The wrapper validates the manifest/capture seals, pinned Git HEAD/tree, clean
tracked and nonignored untracked state, original captured source hashes, and
presence of all tracked inputs (including fixtures) before starting any Go, Cargo, npm or Node command.
A missing variable, wrong checkout, modified source, or missing fixture fails;
there is no fallback to `services/hub` in the current working tree.

`test-hub-parity` retains the full current Rust suite, the explicit ignored
Go-versus-Rust compatibility test, all six historical Go fixture invocations,
the four independent desktop fixture tests, and the exact vocabulary comparison.
The Go oracle executable lives in a temporary directory outside the historical
checkout and is removed even on failure. Go uses `-mod=readonly`; the routing
harness also receives that setting and requires every routing assertion to run.
Dependencies can still use normal external Go caches/network access when these
optional commands are explicitly invoked.

`make hub-vocabulary` now compares historical output with the retained asset; it
does not regenerate that asset. Registry evolution belongs to reviewed portable
contracts and current installed-capability tests. Export is a separate explicit
stdout operation. Likewise, this wrapper neither deletes Go sources nor records
release gates, and successful wrapper unit tests do not claim Go oracle execution.

Source-only validation (no Git/Go processes or original Go checkout required):

```sh
python3 -B -m unittest discover -s scripts -p 'test_hub_reference.py' -v
```

For an intentionally reviewed new reference checkpoint, follow the resealing
procedure in `tools/capability-source-check/README.md`; update the wrapper's
matching seal only as part of that same review.
