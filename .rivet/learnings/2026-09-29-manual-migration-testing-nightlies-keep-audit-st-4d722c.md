---
title: Manual migration testing nightlies keep audit state separate from publication
date: 2026-09-29
promoted: false
---

# Manual migration testing nightlies keep audit state separate from publication

## Observation
An explicitly requested nightly can now use workflow_dispatch nightly=true migration_preview=true on the default branch while migration.json remains incomplete. check-nightly-preview.py requires latest successful push ci.yml for the exact SHA before build and again before publication. Scheduled/default nightlies still require migration ready. Release notes label the experimental migration status; the existing rolling nightly feed is updated, never the stable channel.
