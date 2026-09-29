---
title: Skill removal must resolve the selected directory before inspecting its markdown
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/library.rs
  - services/hub-rs/tests/library.rs
promoted: false
---

# Skill removal must resolve the selected directory before inspecting its markdown

## Observation
Rust library.remove reused the save destination, canonicalized SKILL.md, then recursively removed its parent. A selected skill with SKILL.md pointing at a second in-library skill deleted the second directory. The exact new integration test failed against the cached pre-fix library. A second regression proved nonobject resultSchema was silently accepted and could overwrite a stored dispatch schema.

## Recommendation
Derive and guard the selected skill directory directly for recursive deletion; validate schema shape before any write. Preserve file-alias handling for save separately.

The final owning integration also found that mutable serde_json indexing inside redaction recreated an absent MCP config as env:null/headers:null after validation rejected it. Redaction and placeholder restoration now use get_mut on existing objects only; all20 library integrations pass.
