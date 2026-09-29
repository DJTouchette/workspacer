---
title: Rust native preview packaging owns isolated backend and installer identity
date: 2026-09-28
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/scripts/windows-payload.mjs
  - .github/workflows/rust-native-preview.yml
promoted: false
---

# Rust native preview packaging owns isolated backend and installer identity

## Observation
Standard native packaging bundles four Go executables plus private Node and desktop-host.cjs. The explicit Rust preview instead stages wks-native and workspacer-rust plus CRT/assets, uses a distinct NSIS product/AppUserModelId, and maps feature-gated --local to the in-process Rust backend with isolated data. Preview CI is a separate PR/manual artifact workflow; it does not publish rolling nightly.

## Recommendation
Keep packaging tests that remove every legacy build input before staging Rust. Run Windows installer smoke with Node absent from PATH and the feature-matched Rust native harness; this proves owned backend behavior but is not visual GUI validation. Keep production release mode unchanged until parity gates pass.
