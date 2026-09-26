---
title: Stable Windows release metadata can name an installer asset that does not exist
date: 2026-09-26
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - .github/workflows/release.yml
  - apps/desktop/electron-builder.yml
promoted: false
---

# Stable Windows release metadata can name an installer asset that does not exist

## Observation
While publishing v0.169.0, the stable latest.yml named Workspacer-Setup-0.169.0.exe but GitHub stored Workspacer.Setup.0.169.0.exe and its dotted blockmap. The installed electron-updater GitHubProvider.resolveFiles only replaces spaces with dashes; it does not reconcile dotted GitHub asset names. Verified against v0.168.0: the dashed download URL returned HTTP 404 and the dotted URL returned HTTP 302. This contradicts the context doc claim that the GitHub provider handles that rename.

## Impact
Corrected the v0.169.0 draft latest.yml url and path to the existing dotted installer filename before publishing; checksum and size were unchanged. Nightly already has explicitly dashed filenames and matched its metadata.

## Recommendation
Validate metadata against actual GitHub asset names before publishing every release. A future workflow change should enforce matching stable asset names or normalize the uploaded metadata while retaining any landing-page download compatibility. Re-running the old tag workflow can overwrite the corrected metadata.
