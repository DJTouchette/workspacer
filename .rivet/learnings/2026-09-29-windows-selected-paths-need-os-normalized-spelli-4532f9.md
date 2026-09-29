---
title: Windows selected paths need OS-normalized spellings after link traversal
date: 2026-09-29
promoted: false
---

# Windows selected paths need OS-normalized spellings after link traversal

## Observation
Windows CI found HTML card diffs rejecting a file in its own Git worktree: Git and native temp/file APIs can use different DOS short names, case, or verbatim prefixes. The shared selected-path canonicalizer now performs its link-before-parent walk first, then resolves the existing ancestor through the OS and appends only missing tail names. The Windows probe compiles a regression comparing forward-slash/case-varied Git-style paths with native canonical paths; actual Windows rerun remains required.
