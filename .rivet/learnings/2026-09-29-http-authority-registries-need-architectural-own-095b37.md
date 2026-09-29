---
title: HTTP authority registries need architectural ownership rather than a fake behavior replay
date: 2026-09-29
author: codex
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/tests/corpus_ownership.rs
  - contracts/http-route-registry.json
promoted: false
---

# HTTP authority registries need architectural ownership rather than a fake behavior replay

## Observation
http-route-registry.json is consumed by a TypeScript source guard that compares real Rust routers, guard chains and dynamic operation closure. It is not a golden behavior corpus with two implementation replays. The ownership guard now recognizes exactly this named architectural registry and requires its seven production Rust sources, actual six-check TypeScript guard, active Vitest runner and unchanged vocabulary loader. This validation always runs even though the Rust ownership test itself mentions the filename, so that incidental mention cannot become a fake second behavior loader.

## Impact
Adding a dummy Rust JSON consumer would satisfy the language counter while providing no independent route-authority evidence.

## Recommendation
Keep the architectural classification narrow and mutation-tested. Ordinary data corpora still require their independent language loaders; source registries must retain actual-source discovery, vocabulary and authority mutation checks.
