---
title: Saved timestamps need field-specific Date normalization instead of empty coercion
date: 2026-09-29
suggested_doc: renderer-backend-seam
related_paths:
  - services/hub-rs/src/services/stores/yaml_strings.rs
  - apps/desktop/src/main/lib/providerParity.ts
promoted: false
---

# Saved timestamps need field-specific Date normalization instead of empty coercion

## Observation
Actual old Rust saves contain unquoted ISO timestamps that js-yaml represents as Date. Coercing those to empty would regress persisted dates and restore ordering. The chosen correction keeps LOAD and existing bytes unchanged, normalizes valid LIST dates in Rust and desktop timestamp-specific readers, and quotes new root timestamps. The Rust scalar grammar mirrors installed js-yaml including Date.UTC year0..99, rollover, offsets and fractions; broad chrono parsing incorrectly accepted JS strings.

## Impact
The actual Stores producer and desktop CLI agree on two real saved documents and68 timestamp projections. Original Go yaml.v3 also decodes the new quoted saves with identical contents. Ordinary scalar/title rules remain strict; the old Go odd-date-last assertion is explicitly superseded for upgrade preservation.

## Recommendation
Keep the real producer-to-js-yaml CI seam, typed timestamp matrix and old-save nonmutation tests. Preserve frozen JSON case bytes and record the intentional timestamp correction in migration review.
