---
title: Brief tools share workspaceRoots confinement in the desktop provider
date: 2026-09-01
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/lib/pathConfinement.ts
promoted: false
---

# Brief tools share workspaceRoots confinement in the desktop provider

## Observation
The desktop hub capability paths for brief.append, brief.check, and brief.archive all call assertPathAllowed with the shared workspaceRoots() output before touching the brief files. A platform-specific fix in that root/normalization layer therefore covers all three tools without expanding arbitrary filesystem access.

## Impact
Fixing an individual brief service would leave the other two tools and other shared callers inconsistent.

## Recommendation
Patch and test the shared path/root authorization layer; retain each tool's second containment check on derived .workspacer paths.
