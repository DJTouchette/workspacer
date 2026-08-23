---
title: Hardening pass: profile bypass, oneshot stdin, and the tail-spacer scroll bug
date: 2026-07-30
promoted: true
---

# Hardening pass: profile bypass, oneshot stdin, and the tail-spacer scroll bug

## Observation
Three findings worth remembering. (1) SECURITY: hubCapabilities agents.spawn clamped skipPermissions/permissionMode on the REQUEST but passed profileId through — a bus caller can create a profile whose extraArgs carry --dangerously-skip-permissions (claude.profiles.add is itself a capability) and spawn with it. The Go brain already scrubbed (profiles.go scrubBypassArgs) so only the desktop path was open; agents.spawn is registered with registerCapability, NOT the catalog-delegating cat(), so the TS handler is the one that runs by default. Fixed by porting scrubBypassArgs into claudeProfiles.ts and adding scrubProfileBypass to BOTH spawn helpers — the boundary decides, the helper enforces, so a future spawn entry point can't forget. (2) claudemon /oneshot put the prompt in argv; with claudeBaseArgv() returning ['cmd.exe','/c','claude'] on non-npm Windows installs, agent-written text lands on a cmd.exe command line. Prompt now goes on stdin (claude --print reads it), which kills the class regardless of argv[0]. Both /oneshot and heartbeat also piped stderr without draining it: a child that fills the 64KiB pipe blocks in write(2), never exits, and the call burns its whole timeout — /oneshot now drains both pipes with tokio::join!, heartbeat uses Stdio::null() like the codex path. (3) The tail spacer made 'am I at the bottom?' and 'should streaming drag the view?' share one answer: distanceFromContentEnd discounts the spacer, so a ~600px spacer bought the user 600px of free scrolling that still counted as stuck — scroll up mid-reply and the next chunk snapped you back with no scroll-to-bottom button. Stickiness now uses the RAW distance; only the button uses distanceFromContentEnd.

## Disposition
Folded three ways: profile-bypass scrub -> domains/agent-spawn.md; /oneshot stdin + pipe-drain -> domains/claudemon-http-api.md; tail-spacer raw-distance stickiness -> domains/chat-tool-rendering.md.
