---
title: Nightly publication needs typed JSON and independent readback
date: 2026-09-30
suggested_doc: auto-update-release-channel
related_paths:
  - scripts/finalize-nightly.py
  - .github/workflows/release.yml
promoted: false
---

# Nightly publication needs typed JSON and independent readback

## Observation
Release36679959476 returned success after its form-field PATCH, but release399798075 still had draft:true, an untagged URL and public nightly downloads returned404. An explicit JSON request containing boolean draft:false published the already verified16 assets at fee9d40f; canonical native/Electron URLs then returned200.

## Impact
A successful PATCH command or published_at timestamp alone does not establish a usable public nightly.

## Recommendation
Use finalize-nightly.py to validate uploaded names/sizes and exact target, send typed JSON, independently read back non-draft state and resolve the actual tag. Keep its fake-gh regression tests in CI before release mutation.
