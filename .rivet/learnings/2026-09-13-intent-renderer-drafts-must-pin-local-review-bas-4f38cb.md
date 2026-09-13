---
title: Intent renderer drafts must pin local review bases and survive nested form unmounts
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/components/IntentArtifacts.tsx
  - apps/desktop/src/renderer/src/components/IntentControls.tsx
  - apps/desktop/src/renderer/src/components/IntentSteering.tsx
promoted: false
---

# Intent renderer drafts must pin local review bases and survive nested form unmounts

## Observation
Artifact chooser previously erased unsaved annotations; drafts now key on workspace/artifact/digest. Assessment forms unmount when Direction/Execution hides, so state is lifted into controlled per-record maps. Alternative-selection and assessment drafts now retain the prior selection ID/assessment count from first edit: refreshing records must not silently advance their compare-and-swap base. Mutations use component epoch guards so a late save cannot clear a later workspace draft, and artifact retries reuse IDs after ambiguous responses.
