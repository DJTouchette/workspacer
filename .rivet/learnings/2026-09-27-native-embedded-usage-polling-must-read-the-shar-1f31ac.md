---
title: Native embedded usage polling must read the shared launch setting explicitly
date: 2026-09-27
promoted: false
---

# Native embedded usage polling must read the shared launch setting explicitly

## Observation
External-claudemon serve does not launch a daemon child, so its config.yaml usage.pollOnBoot-to-environment propagation no longer configures the embedded runtime. Native reads the same shared config root and passes an explicit Embedded Options value without mutating global environment. Explicit WORKSPACER_USAGE_POLL_ON_BOOT takes precedence, preserving isolated tests. Missing, malformed, and nonboolean settings preserve daemon defaults. Library lifecycle tests explicitly disable idle usage polling.
