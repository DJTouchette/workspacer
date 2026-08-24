---
title: Version source of truth is apps/desktop/package.json (0.149.0); no v0.150 tag exists
date: 2026-08-23
confidence: high
related_paths:
  - landing/docs.html
  - landing/build.html
  - landing/index.html
  - package.json
  - apps/desktop/package.json
promoted: false
---

# Version source of truth is apps/desktop/package.json (0.149.0); no v0.150 tag exists

## Observation
Latest git tag is v0.149.0 (2026-08-14). The hub-federation feature (commit 1ece854b) landed 2026-08-16, AFTER that tag, and package.json is still at 0.149.0 with no v0.150 tag cut. landing/docs.html, build.html, and index.html all guessed "v0.150 / next release" for federation — a version number that was never confirmed and still isn't real. Replaced with channel-status framing ("Currently nightly-only — hasn't reached a tagged stable release yet") that doesn't name an unconfirmed version.

## Impact
Any future landing-page copy for an in-master-but-unreleased feature should avoid guessing the next version number; state channel availability instead (nightly vs stable) so the line doesn't need editing again when the version is actually cut.

## Recommendation
When federation reaches a stable tagged release, remove the "Currently nightly-only" lines in docs.html and build.html entirely (index.html's line already has no such caveat).
