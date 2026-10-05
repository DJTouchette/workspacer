---
title: fs.listEntries hides git-ignored entries unless includeIgnored; editor saves are client-side compare-and-write
date: 2026-10-05
confidence: high
suggested_doc: git-review
related_paths:
  - services/hub-rs/src/services/files.rs
  - apps/native/src/features.rs
promoted: false
---

# fs.listEntries hides git-ignored entries unless includeIgnored; editor saves are client-side compare-and-write

## Observation
fs.listEntries always omits .git and, by default, every git check-ignore'd entry; it now accepts includeIgnored:true (echoed back) so explorers can be honest about 'all files'. Older hubs ignore the flag and keep hiding, detectable by the missing includeIgnored:true echo. fs.write is a plain truncate-write with no precondition, so the native editor's conflict check is fs.read-compare then fs.write then fs.read verify. This is NOT atomic: a write landing between the compare and our write is silently overwritten and the read-back does not detect it (it sees our own text); the read-back only catches a write after ours. Real protection needs a hub-side compare-and-write (expected content/revision) under a file lock. git.status paths are work-tree relative and now carry root.

## Impact
Explorers that claim 'All files' while the hub hides ignored ones mislead users; editors that write blind lose concurrent agent edits.

## Recommendation
Label hidden-ignored trees; join git paths under status.root with links::join_within; treat the editor conflict check as stale-editor protection only, and add a hub-side fs.write precondition if concurrent writers matter.
