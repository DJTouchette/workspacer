---
title: last-exit.json is written by a trap and never consumed, so a hard-killed run is reported as a clean sleep
date: 2026-08-25
confidence: high
promoted: false
---

# last-exit.json is written by a trap and never consumed, so a hard-killed run is reported as a clean sleep

## Observation
deploy/fly/{node,hub}/entrypoint.sh write /data/state/last-exit.json from a TRAP on INT/TERM. Exits that run no trap — host eviction, OOM kill of PID 1, SIGKILL — write nothing, leaving the record from an EARLIER run in place. Nothing can date it: the entrypoint writes a `bootId` field and cmd/brain/lastexit.go's exitRecord struct has fields only for reason/exitCode/at, so bootId is dropped at parse. A node that slept cleanly (signal-TERM written), woke, ran for a day and was then hard-killed reports the stale `signal-TERM` on the next boot, and every client shows a node that went to sleep on purpose. Closed entrypoint-side by renaming the file to last-exit.consumed.json once logged; readExitRecord already returns nil for a missing file, which reads as "no record" rather than "ended cleanly".</observation>
<parameter name="impact">CONSEQUENCE to know about: the rename happens before the brain starts, so brain.info no longer carries an exit reason at all — the record's home is now the boot log (which reaches fly logs via the previous-boot replay). Putting the brain.info half back means moving the consumption into cmd/brain/lastexit.go: read, report, THEN rename. Also note deploy/fly/node/RUNBOOK.md §8 check 12 had to change from `cat /data/state/last-exit.json` to grepping the boot log, because the file is gone by the time you look.</impact>
<parameter name="related_paths">["deploy/fly/*/entrypoint.sh", "services/hub/cmd/brain/lastexit.go"]
