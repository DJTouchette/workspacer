---
title: Retained plugin copy keeps production bytes while separating test relocation
date: 2026-09-30
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - plugins/examples.provenance.json
  - scripts/check-retained-plugin-assets.mjs
  - services/hub-rs/LEGACY_DELETION_PREPARATION.md
promoted: false
---

# Retained plugin copy keeps production bytes while separating test relocation

## Observation
Thirteen tracked Go-tree example assets now have a retained plugins/examples owner while originals remain byte-identical and untouched. Runtime/vendored files copy exactly; Headroom test-only ws import needs one fewer parent and README commands are explicit documented exceptions with both SHA256 values. The TS-only routing view fixture moved by copy to desktop/tests/fixtures, avoiding a fabricated shared-corpus loader. Package destinations stay unchanged; Docker allowlists and git archive strip depth follow the new source depth.

## Recommendation
Run check-retained-plugin-assets plus actual Headroom/editor/native payload owners; keep LF checkout rules for both hashed copies and references. Commit the copied tree before exercising the HEAD-based deployment archive. Reference Go guards and deletion gate remain separate.

## Rust evidence boundary
The later local hub link exhausted scratch space before running tests, so it is not a pass. Exact915960b9 CI36659897298 hub job109712115244 passed integration_spine2 with the original manifest; its copied manifest has identical SHA256 df8e11cc7981286dde1e5239f999ff1735fb7718bd2c4f61ccb302b210fd27a2. The next committed CI/release must validate the new include/package paths.

The staged whitespace check initially reported upstream whitespace in the byte-pinned CodeMirror bundle. Its contents remain unchanged; a file-specific `-whitespace` attribute records the vendored exception. The final diff check and13+1 provenance check pass.
