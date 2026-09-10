---
title: Manager handoff checkpoint errors conflate Windows path spelling and digest failures
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacementArtifact.ts
promoted: false
---

# Manager handoff checkpoint errors conflate Windows path spelling and digest failures

## Observation
The ac818c86 validator reproduces Checkpoint brief pointer or content hash is invalid with valid SHA-256 values when a lower-case Windows drive differs from realpath output or a forward-slash launch root differs from path.dirname. This occurs before receipt hash validation and successor spawn; the successor ID is allocated at start. Later validateSuccessor calls check session readiness and provenance, not checkpoint files. The remote artifact was unavailable, so its exact failed predicate remains unproven.

## Impact
A legitimate checkpoint can fail solely on Windows path spelling. Malformed and well-formed but incorrect hashes reach the same old message, so a screenshot cannot prove model-generated metadata or stale bytes.

## Recommendation
Compare only interchangeable Windows drive letters and separators without case-folding components or resolving dot segments. Preserve root, realpath, basename and link checks; hash exact buffers and distinguish field/category errors. Run native artifact tests in the existing Windows CI job; Linux Win32 fixtures do not establish native Windows execution.
