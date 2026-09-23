---
title: Artifact cleanup startup is delayed and shared between desktop and headless
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/worktreeArtifactCleanupScheduler.ts
  - apps/desktop/src/main/headless/stdio.ts
  - apps/desktop/scripts/cleanup-agent-artifacts.mjs
promoted: false
---

# Artifact cleanup startup is delayed and shared between desktop and headless

## Observation
Automatic artifact cleanup is enabled by canonical agents.artifactCleanup defaults with minAgeHours 1, starts 60 seconds after local desktop startup or the first valid headless request carrying daemonURL, and checks every 15 minutes. Remote-client desktop skips startup. Each pass reads current config; a module-level in-flight guard prevents overlapping scans. Shutdown clears timers and makes subsequent loadState calls fail. CLI npm run cleanup:agents defaults to dry-run and shares the production cleanup core via a disposable bundle; --apply is explicit, config paths match configService, and root/minimum age honor config. Both TS generated defaults and Go embedded defaults include policy and worktreeRoot, with roundtrip tests.

## Impact
Headless and orphaned worktree artifacts can be reclaimed without depending on renderer card closure. Startup/tests do not run deletion at module import. CLI and scheduler use the same safety checks and daemon maintenance negotiation.

## Recommendation
Use docs/agent-artifact-cleanup.md. Keep daemon maintenance fencing and cooldown validation in the shared core, avoid separate ad-hoc deletion scripts, and report skipped reasons rather than assuming a successful invocation deleted files.
