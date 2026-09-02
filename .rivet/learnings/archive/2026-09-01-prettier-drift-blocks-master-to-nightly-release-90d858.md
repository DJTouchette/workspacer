---
title: Prettier drift blocks master-to-nightly release promotion
date: 2026-09-01
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/desktop/src/main/services/hubDaemon.ts
  - .github/workflows/ci.yml
promoted: true
promoted_to: auto-update-release-channel
---

# Prettier drift blocks master-to-nightly release promotion

## Observation
For release candidate a22bc61b, the CI desktop Format check fails solely on apps/desktop/src/main/services/hubDaemon.ts. The file is byte-for-byte unchanged from origin/master, so the failure predates the context health commits.

## Impact
The CI workflow is required for master pushes, and the nightly workflow must not be dispatched until that CI is green.

## Recommendation
Before the next release promotion, obtain approval to fix the unrelated formatting drift (and verify the Windows containment job) or explicitly change the release policy.

## Disposition
Promoted into `.rivet/context/domains/auto-update-release-channel.md` WITH A CORRECTION that matters more than the incident: the nightly is NOT gated on CI. `release.yml`'s `gate` job only decides nightly-vs-not and whether the sha changed; the 08:00Z scheduled run `33510502279` published successfully on 2026-09-01 while master's CI was red. The specific `hubDaemon.ts` drift was fixed in `2d8f7fd0`; what survives is "a Format failure can belong to master, not to your branch — diff the named file first".
