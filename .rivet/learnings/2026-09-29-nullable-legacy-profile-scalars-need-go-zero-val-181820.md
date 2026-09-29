---
title: Nullable legacy profile scalars need Go zero-value decoding
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/profiles.rs
promoted: false
---

# Nullable legacy profile scalars need Go zero-value decoding

## Observation
Go encoding/json leaves string/bool profile fields at zero values when JSON null is present, but Rust Profile deserialization rejected those nulls and made an otherwise usable store unavailable. Profile now uses nullable defaults for id/name/configDir/isDefault/provider/preset/tokenEnvVar, retaining existing nullable list and weight handling. A real file/list/update regression preserves the row and rejects genuinely wrong string/bool types.
