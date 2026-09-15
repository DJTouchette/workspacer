---
title: First-use intent tests must select a Kanban card after mounting Work
date: 2026-09-15
promoted: false
---

# First-use intent tests must select a Kanban card after mounting Work

## Observation
Full CI includes two intent browser scenarios in firstUse.test.ts in addition to intentCompletion.test.ts. With the Kanban default, initial Work mount and remount after disabling/re-enabling the feature show the board without selecting a workspace. These scenarios must click the saved intent card before selecting Intent/Execution tabs. Ordinary hide/show of the still-mounted Work surface continues preserving the current detail and draft.
