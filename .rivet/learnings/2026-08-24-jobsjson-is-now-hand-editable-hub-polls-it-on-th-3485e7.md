---
title: jobs.json is now hand-editable: hub polls it on the 30s tick and re-reads before every write
date: 2026-08-24
confidence: high
suggested_doc: hub-jobs
promoted: false
---

# jobs.json is now hand-editable: hub polls it on the 30s tick and re-reads before every write

## Observation
internal/jobs Service now re-reads its spec file whenever the contents change. reloadIfChangedLocked() runs at the top of the locked section of tick(), List, Upsert, Propose, Remove and RunNow. Change detection is a sha256 of the file bytes (specHash/haveSpecHash), not mtime, and the same hash suppresses the service's own write echo. New() is just the first reload, so boot and hot-reload share one code path. RunScheduler's interval is now the Service.tickEvery field (default defaultTickEvery = 30s), which is what lets a test drive the real scheduler goroutine.</observation>
<parameter name="impact">Two things that used to be true are no longer true: an external edit to jobs.json was invisible until restart, and the next hub write silently clobbered it. Anything that assumed s.jobs is a boot-time snapshot is wrong now. Also: the ONLY bookkeeping write to the spec file is the `once` trigger's self-disarm; interval reschedules touch nextAt only.</impact>
<parameter name="recommendation">Do not add an mtime comparison as an optimisation in front of the hash — the hash exists because a same-size write inside one mtime tick is missable and this repo already shipped one millisecond-collision bug. If you add a new jobs.* RPC that mutates, call reloadIfChangedLocked() first or it will clobber hand edits. Bad-parse and missing-file both keep the last good schedule; only `{"jobs": []}` clears it.</recommendation>
<parameter name="related_paths">["services/hub/internal/jobs/*.go", "services/hub/scripts/jobs-harness.mjs"]
