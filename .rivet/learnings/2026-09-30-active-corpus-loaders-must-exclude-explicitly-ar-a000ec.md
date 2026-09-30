---
title: Active corpus loaders must exclude explicitly archived Go identities
date: 2026-09-30
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/tests/corpus_vocabulary.rs
  - contracts/retired/block-loader-provenance.json
promoted: false
---

# Active corpus loaders must exclude explicitly archived Go identities

## Observation
Post-deletion whole hub CI exposed12 Go paths still listed as active block loaders across3 portable corpora. Each already has retained Rust/TS consumers. Only those12 declarations moved into a sealed historical archive with pinned originalsource hashes/casecounts and required live replacement declarations. Active loader resolution still fails missing sources; corpus cases/values unchanged. Existing /contracts/**/*.json LF rule protects the archive seal on Windows. Direct exact corpus test target passed using cached serde_json/sha2/regex after Cargo auto-bin linker resource failure; normal Cargo proof remains CI-owned.
