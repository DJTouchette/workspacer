---
title: Fleet result wakes must carry refreshed workflow instructions
date: 2026-09-29
promoted: false
---

# Fleet result wakes must carry refreshed workflow instructions

## Observation
Go desktophost commitWorkerResult supplies instructions to fleetmsg after validated result persistence. The Rust wake formatter and producer originally omitted this block even though workflow_runtime::instructions existed. Manager finish wakes now read committed task history, restrict bound workflow steps to the current recipient owner, and recheck live role/worker activity/parent after the disk await. Ordinary parents do not receive task/workflow instructions. A persisted-history delivery regression covers manager, ordinary parent and moved task owner. Witness has no mapping for these new Rust paths; run the wakes and fleet_messages Cargo integration targets explicitly.
