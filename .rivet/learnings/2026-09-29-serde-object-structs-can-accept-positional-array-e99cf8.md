---
title: Serde object structs can accept positional arrays in provider responses
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/models.rs
promoted: false
---

# Serde object structs can accept positional arrays in provider responses

## Observation
The provider typed-response regression exposed serde_json accepting [] as a defaulted struct and model row arrays as positional structs. Go encoding/json rejects those shapes. provider_rows now checks envelope and every model row are object/null before typed deserialization; null envelope/rows still retain Go zero defaults. Regression rejects both [] and [id,label,true] model rows. Preserve upstream routing metadata only after this validation.
