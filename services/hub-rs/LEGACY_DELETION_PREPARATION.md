# Legacy deletion preparation — read-only map

2026-09-30. This inventory uses **tracked paths only** (`git ls-files` / tracked-source search), not arbitrary runtime JSON. `services/hub` currently has572 tracked files:532 Go files and40 non-Go files. No deletion, move, gate update or claim that non-deletion gates passed is made here. `CUTOVER_STATUS.md` explicitly keeps those gates separate from source review.

## Prepared live owners

The initial read-only inventory below led to a reversible preparation batch:
13 tracked example files were copied into `plugins/examples`, and the routing
UI fixture moved to the desktop test-fixture owner. The original reference
files remain untouched. Active package, Docker, native, editor/Headroom and
renderer consumers now use the retained copies; installed bundle paths remain
the same. `plugins/examples.provenance.json` records original and destination
hashes, with explicit test-import-depth and README-link exceptions.
`make check-retained-plugin-assets` verifies the recorded bytes. Production and
vendored plugin bytes are unchanged. Final package CI still has to validate the
new source paths; preparing them does not authorize Go removal or certify a gate.

## Retain or redirect before removing the old tree

| Tracked family | Proposed lossless destination / disposition | Exact active consumers to change together |
| --- | --- | --- |
| `examples/**` —13 files across clock-plugin, editor, headroom and transcript-timeline | Move as one unchanged tree to `plugins/examples/`, preserving relative imports, bundled CodeMirror bytes, manifests, tests and READMEs. Keep installed bundle locations `hub/examples`, `examples` and `/usr/local/share/workspacer/examples` stable. Headroom's public Node sidecar is **not** the retired private companion. | `apps/desktop/electron-builder.yml` all3 OS resources; `apps/desktop/src/main/services/hubDaemon.ts::bundledExamplesDir` development branch; `apps/desktop/package.json::test:headroom`; `apps/desktop/src/renderer/tests/editorPluginTree.test.ts`; `apps/native/scripts/windows-payload.mjs` and its fixture in `windows-payload.test.mjs`; `.github/workflows/release.yml` standalone bundle copy; all4 `deploy/fly/{combined,hub,node,rust}/Dockerfile` copies and their `.dockerignore` allowlists; `deploy/fly/rust/build-upgrade.sh` tracked archive path/strip depth; `services/hub-rs/tests/integration_spine.rs` actual editor manifest include. |
| `cmd/hub/{addon-fit.js,icons,manifest.webmanifest,mobile.html,remote.html,sw.js,xterm.css,xterm.js}` | Already has byte-identical tracked twins in `services/hub-rs/assets/web/`:11 files compared directly in this audit. Make Rust copies authoritative; do not generate them from deleted Go paths. | Current Rust web assets are already embedded. Update old operational/source links, not historical result-text fixtures. New snapshot mobile guard already reads the shipping Rust asset. |
| `cmd/hub/sdk.js` | Already byte-identical to `services/hub-rs/src/plugins/sdk.js`, which `plugins/http.rs` embeds. Preserve that existing owner; no extra copy needed. | Documentation referring to the old SDK path; product endpoint `/plugins/sdk.js` remains unchanged. |
| `internal/routing/testdata/preferences-view.json` | Move exact fixture bytes to `contracts/fixtures/routing-preferences-view.json`; this is a shared UI fixture, not executable Go. Record original path/hash in provenance; if made a formal shared corpus, retain proper vocabulary/loader declarations rather than bypassing corpus guards. | Active Vite raw imports: `apps/desktop/src/renderer/src/harness/routingHarness.tsx` and `apps/desktop/src/renderer/tests/components/routingSection.test.tsx`. These would fail immediately after deleting the old path. |
| `cmd/brain/config_defaults.json`, `internal/routing/routing.default.yaml`, `internal/capspec/testdata/param-vocabulary.json` | Existing owners: `services/hub-rs/assets/config-defaults.json`, `services/hub-rs/src/services/routing.default.yaml`, `apps/desktop/tests/fixtures/capability-parameter-vocabulary.json`. Preserve reviewed transformations/provenance; do not blindly assert all three are byte-identical. | `scripts/hub-migration.py::sources` explicitly inventories these3 data files. Config default generation already uses Rust assets. Capability vocabulary is also independently hashed by the Rust source scanner. |
| `internal/limits/testdata/usage-report.json` | Preserve as an exact archived fixture under `contracts/reference-baselines/fixtures/` if its independent sample remains useful; otherwise explicitly record its replacement by current shared usage contracts before deleting it. | No active non-Go-tree reader found in tracked-source search. This is an absence finding limited to the searched tracked tree, not automatic disposal authority. |
| `README.md`, `docs/{plugin-theming,rules-engine-plugin,workflow-events}.md` | Move still-current plugin/event docs to `docs/plugins/` or the existing canonical topic, preserving useful public contracts. Replace obsolete hub bootstrap README with links to `services/hub-rs` operational docs; retain history in Git. | `scripts/check-doc-drift.sh` names `services/hub/README.md`. `landing/build-plugin.{md,html}`, `landing/build.html`, `landing/docs.html` and related docs link examples and legacy MCP source. Update real links; do not rewrite historical quoted bug/result fixtures. |
| `scripts/{fake-claudemon.mjs,federation-harness.sh,jobs-harness.mjs,routing-limit-harness.mjs}` | Do not simply move runnable Go-dependent harnesses and call them portable. Preserve useful fixtures in `scripts/reference/` with explicit pinned-reference requirements, or port their still-required assertions onto current owning Rust targets first. | `Makefile::test-routing-harness` invokes the old routing harness. Federation/jobs/routing harnesses contain actual `go build`/`go run`; fake-claudemon is a helper, not proof of a Rust product dependency. |
| `go.mod`, `go.sum` | Retire with the Go module only after oracle commands and checkout-presence guards are intentionally updated. Preserve exact source checkpoint in provenance. | `apps/desktop/package.json::test:hub-reference`, Make reference targets, and `claudemonRouteContract.test.ts::legacyPresent`. |

## Hard source-byte blockers versus already-portable checks

1. **Capability source scanner currently requires old Go bytes.** `tools/capability-source-check/src/reference.rs` reads every `go-reference.json::sources` entry plus `cmd/brain/capspec_params_test.go` and compares SHA256. Missing files fail. Its84 original dangerous bindings are independently checked against current production owners and must stay. Prepare a versioned captured-provenance mode tied to a reviewed Git commit/tree and immutable source digests; validate live Rust/TS owners and the binding floor without requiring deleted checkout paths. Keep explicit historical verification available through a pinned Git worktree/source archive. Do not convert missing arbitrary source files into success or remove the binding assertions. Update `tools/capability-source-check/tests/policy.rs` provenance mutation controls with the same change.
2. **Renderer registration guard still reads Go main.** `apps/desktop/src/renderer/tests/backend/backendParity.test.ts` reads `services/hub/cmd/hub/main.go` and extracts `RegisterLocal` calls. Replace this oracle with the reviewed `contracts/backend-capabilities.json`/Rust capability inventory, retaining positive population floors and a real installed-method comparison. A path-only metadata replacement is insufficient.
3. **Migration ledger is historical evidence to retain.** `services/hub-rs/migration.json`, `reviews/*.json` and their original SHA256 values must survive removal. `scripts/hub-migration.py` already tolerates removed sources that were recorded ported/retired and validates replacement/test files outside the old tree. It does not prove gate execution or physical deletion. Preserve that distinction and test the final completed-ledger/deleted-tree case; do not drop records or mark gates merely to make `ready` pass.
4. **Six reference baselines already support deletion.** `scripts/capture-contract-baselines.py --check` verifies fixture bytes, case floors, Rust loaders, reference commit and hashes in `contracts/reference-baselines/manifest.json`; original source bytes are checked only when present. `--capture` deliberately needs tracked original bytes and is not a test execution. Preserve the manifest and captured corpus bytes; no new source copy is required for routine checks. Rust `tests/corpus_ownership.rs` has the corresponding independent guards.
5. **MCP generation is already portable.** `scripts/mcp-catalog.py` consumes `services/hub-rs/assets/{mcp-tools,mcp-help,mcp-rust-presentation,mcp-wire-contract,mcp-wire-overrides}.json`. It validates original digest metadata but does not read old Go sources. `tools/go-mcp-wire-reference` and `tools/go-capability-reference` are optional standard-library source capture tools; their explicit capture commands need a pinned historical checkout after deletion, not a runtime Go fallback.
6. **Current Rust asset generation must remain live.** `Makefile::check-hub-rust-assets` uses portable contracts and existing public TS owners; `generate-rust-brain-capabilities.cjs` reads `contracts/backend-capabilities.json` and `desktop-service-methods.json`. Keep these generators and public Electron services. Their Node execution is build tooling/public product ownership, not the removed private Node companion.
7. **Daemon route inventory intentionally distinguishes reference sources.** `apps/desktop/src/main/services/claudemonRouteContract.test.ts` already filters old Go callers only when `services/hub/go.mod` is absent, while scanning active callers against `contracts/claudemon-routes.json`. Preserve historical entries in `apps/desktop/tests/support/claudemonCallers.json`, active caller population/refusal checks and the live scanner. Review this explicit retirement mechanism rather than broadening it into a general missing-file skip.

## Named command changes to schedule with deletion

- `make test-hub-parity` currently builds `cmd/hub-reference`, executes Go fixture adapters and compares the Go vocabulary export. After final reference receipts, split/rename the historical oracle command to require a pinned reference checkout; retain portable Rust and independent TS fixture replay as ordinary CI. The ignored `tests/compatibility.rs::shared_contracts_go_reference` must remain explicit about requiring `WKS_GO_HUB_REFERENCE`, not silently pass without it.
- `make hub-vocabulary` currently executes the Go exporter. Preserve `assets/hub-vocabulary.json` provenance and make future vocabulary maintenance flow through a reviewed portable owner plus actual installed-capability tests.
- `make test-hub-reference`, desktop `test:hub-reference`, `test-routing-harness`, reference CLI READMEs and `mise.toml`'s Go-module explanation need explicit retirement/retargeting. Current `test-hub` and `test-hub-rust` already run Rust and should not change.
- Legacy binary cleanup entries in Makefile/.gitignore and historical comments can be cleaned last. A string such as `services/hub/cmd/hub/main.go:392` in a linkifier fixture or recorded structured result is intentional test data, not an active file dependency.

## Lossless review sequence, after the non-deletion gates

1. Record a final immutable reference commit and verify all captured fixture/source hashes while the original tree still exists.
2. Move only the tracked retained assets above and update every concrete consumer in the same change. Compare tracked source/destination byte hashes, including bundled JS and plugin tests; never copy arbitrary runtime state.
3. Complete portable source-guard/oracle transitions with refusal/mutation tests and retain original digests, case floors and live cross-language consumers.
4. Validate package copy paths, Docker allowlists/archive strip depth, SDK/web serving, editor/headroom consumers, generators, corpus guards and actual capability inventory against the changed paths.
5. Only then perform the separately authorized Go deletion, verify no active build/runtime/source-reader dependency remains, record deletion evidence and run completion checks. No source removal or certification is performed by this preparation document.

## Reversible retained-asset copy checkpoint

The13 tracked examples have now been copied to `plugins/examples/`; the originals
remain untouched. Active package/desktop/native/container consumers use the new
owner with unchanged installed destinations. `plugins/examples.provenance.json`
records exact original/copy hashes and the two approved Headroom test-import and
README-only relocation exceptions. Production/vendored bytes are identical.

The routing view sample was copied to the **TypeScript-only** owner
`apps/desktop/tests/fixtures/routing-preferences-view.json`, rather than promoting
it to a shared contract without a Rust loader. Its two executing imports changed;
the original fixture remains. `make check-retained-plugin-assets` verifies both
copies/provenance, and the existing asset-check target includes it. This checkpoint
performs no Go deletion or reference-byte guard retirement.
