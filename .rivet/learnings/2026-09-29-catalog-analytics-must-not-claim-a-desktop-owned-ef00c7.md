---
title: Catalog analytics must not claim a desktop-owned capability without an engine
date: 2026-09-29
suggested_doc: usage-accounting
related_paths:
  - services/hub-rs/src/services/mod.rs
  - services/hub-rs/src/services/workflow_artifacts/mod.rs
promoted: false
---

# Catalog analytics must not claim a desktop-owned capability without an engine

## Observation
Rust install_config reports catalog when engine is absent, but previously opened headless-analytics.sqlite and installed analytics.summary/recent unconditionally with home_dir. This could shadow an actual desktop provider with an unavailable-source handler. Analytics installation now requires the owned engine while catalog pricing stays installed. The workflow merge remains content-only; cached overlays additionally fence cwd/generation identity and actual closure handling prevents late artifact resurrection.

## Impact
Capability presence is an ownership decision, not proof a data source exists. Returning an error instead of zeros is insufficient when an unavailable local owner prevents the real provider from answering.

## Recommendation
Keep real catalog external-provider forwarding and full-engine-stop error tests, plus live-state overlay/no-resurrection controls. Treat cache identity fencing as stronger than the old unversioned Go overlay map.
