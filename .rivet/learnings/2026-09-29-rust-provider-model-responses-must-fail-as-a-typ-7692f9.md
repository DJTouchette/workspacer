---
title: Rust provider model responses must fail as a typed batch
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/models.rs
promoted: false
---

# Rust provider model responses must fail as a typed batch

## Observation
Legacy providersListModels decodes into typed id/label/default fields and returns an empty list if any row has an invalid field type. The Rust map previously coerced malformed fields independently into blank/default values. services/models.rs now validates the whole response before projecting rows and updating routing; omitted/null fields retain Go zero values. provider_request also centralizes tested provider/cwd admission and URL query encoding.
