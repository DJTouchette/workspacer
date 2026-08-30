---
title: Delegation moved the writers to Go but left every persistence guard in the TS twin
date: 2026-07-31
promoted: true
---

# Delegation moved the writers to Go but left every persistence guard in the TS twin

## Observation
Fixed 2026-07-31. DELEGATE_CATALOG_TO_BRAIN defaults ON (brainDelegation.ts:18), so services/hub/cmd/brain is the live writer for config, layouts, saved sessions, profiles and library. Every safety property around those writes had been written in the desktop TS and never moved: fsync (atomicWriteFile.ts), the saved-session identity check (sessionService.ts refuses to clobber a file it cannot identify; stores.go saveSavedSession has no such check), and persistBlocked/.broken-* backup which exist for config.yaml only. One guard went the other way and that is what hid the pattern: dropHostTrusted lives only in Go, so the TS bus handler answering config.save when delegation is OFF had no drop at all. Three fixes: (1) brain writeFileAtomic now calls tmp.Sync() before Close/Rename - the TS twin's comment already CLAIMED the Go side did this; without it a crash leaves a correctly-named EMPTY file, worse than the half-write the helper prevents because the old copy is gone. (2) migrateSessionData (App.tsx) now returns a `recognised` flag: null/undefined data is the legitimate empty case, any other unmatched shape is a file this build cannot read. useSessionLifecycle holds restoreFailedRef (a REF not state - saveCurrentSession is called from timers and beforeunload closures that captured an older render) and blocks every save path for the run, posting to the notification center. Before this, a nightly->stable rollback or a transient EACCES made the 1s debounced autosave write agents:[] over the user's whole layout with no backup. Fresh install still saves - empty IS the truth there. (3) contracts/host-trusted-config-cases.json now pins the host-trusted section list plus drop semantics, asserted by BOTH hostTrustedConfig.test.ts and TestHostTrustedContractCases; emptying either side's list fails that side. Durability is deliberately NOT fixture-able - it is a syscall property, not a data mapping. Open follow-up from the same review: stores.go still lacks the identity check, session/layout files still have no schemaVersion and no .broken-* backup, and listSessions/listSavedSessions silently skip unparseable files so a broken default.yaml just vanishes from the list.

## Disposition
Folded into .rivet/context/domains/config.md (delegation guards note, incl. open follow-ups).
