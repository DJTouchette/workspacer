---
title: Rust cutover must retain the legacy hub state directory
date: 2026-09-28
promoted: false
---

# Rust cutover must retain the legacy hub state directory

## Observation
Go defaults layout.json, jobs.json, jobs-history.json, usage-pacing.json, VAPID and subscriptions to os.UserConfigDir()/workspacer-hub, separate from Workspacer config. Rust standard CLI now selects that historical directory for shared hub state, while explicit data-dir wins and custom config-dir stays isolated. On macOS historical hub state is HOME/Library/Application Support/workspacer-hub even though Workspacer CLI config defaults under .config. Seeded real launcher fixtures assert actual layout/pacing/jobs RPC reads as well as unchanged VAPID bytes. The updated CLI9 suite passed in /tmp/workspacer-complete-services-checkpoint.log.
