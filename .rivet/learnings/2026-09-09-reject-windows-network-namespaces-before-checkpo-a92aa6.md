---
title: Reject Windows network namespaces before checkpoint filesystem inspection
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacementArtifact.ts
promoted: false
---

# Reject Windows network namespaces before checkpoint filesystem inspection

## Observation
Win32 path.isAbsolute and root length accept UNC and device namespaces. The fef68807 validator lstat-ed these untrusted candidate volumes before allowed-root identity comparison. Mocked fs regression spies reproduce remote-I/O eligibility without SMB. Separately, handoff.json had lstat followed by a pathname read: replacing it with identical bytes and a new inode passed. The proposal now opens once, fstats BigInt identity and size before descriptor reads, rechecks returned byte length, and closes in finally.

## Impact
Rejecting a pointer after filesystem inspection is too late to prevent network access; receipt hashes alone do not bind a proposal to the inspected file identity.

## Recommendation
Keep drive-qualified syntax gating ahead of all candidate filesystem calls. Preserve per-component identity/link checks and document that component walks remain non-atomic.
