---
title: Native Windows release compilation overlaps Electron and transfers verified CRT inputs
date: 2026-09-29
confidence: high
related_paths:
  - .github/workflows/release.yml
promoted: false
---

# Native Windows release compilation overlaps Electron and transfers verified CRT inputs

## Observation
release.yml now starts native-windows-build directly from gate alongside the Electron/backend matrix. Native package waits both and reuses workspacer-rust.exe from the Windows matrix, GUI+harness plus DLLs from native compiler runner. Inputs include exact source SHA, gate-derived version, platform, run ID and file SHA256; native receipt retains rustc, image and CRT source. Package verifies all before restoration and never rediscovers CRT. Internal input artifacts avoid workspacer-* so publish glob cannot leak raw executables. Each compilation job has one rust-cache with cache-bin:false and distinct release shared-key.

## Impact
Avoids serial native compilation and competing Cargo-bin cache cleanup while preserving candidate/version and runtime provenance.

## Recommendation
Validate the split through a nonpublishing workflow before claiming measured speedup; Windows package smoke and final publication dependencies remain required.
