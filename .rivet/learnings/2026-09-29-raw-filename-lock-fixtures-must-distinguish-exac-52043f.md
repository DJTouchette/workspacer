---
title: Raw filename lock fixtures must distinguish exact-byte support from APFS EILSEQ refusal
date: 2026-09-29
confidence: high
suggested_doc: config
related_paths:
  - services/hub-rs/src/services/config.rs
promoted: false
---

# Raw filename lock fixtures must distinguish exact-byte support from APFS EILSEQ refusal

## Observation
macOS preview36637635273 job109642057069 failed only config lock raw-byte fixture because APFS returned EILSEQ92 for invalid UTF-8 names. Production correctly preserved bytes and refused; the fixture incorrectly required success on every Unix filesystem. Revised test always proves ordinary lock success, probes exact raw filename creation, requires matching EILSEQ only when that probe rejects, and forbids lossy replacement-character filenames and leftovers. Supporting filesystems must still prove exact-byte acquisition and release.

## Validation
The focused Linux owner test passed (one test, zero ignored) in /tmp/workspacer-configlock-apfs-fixture.log. This exercises the supporting-filesystem path; the rejecting-filesystem branch requires the next macOS CI run. Production code was unchanged.
