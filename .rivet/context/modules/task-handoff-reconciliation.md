# Retained parallel work reconciliation (2026-09-11)

Compared recovery `a4386dd8fd677b881d79ae5b689e2145541a705c` with retained
`18cd41693875374fbee5e4c877daf4b5614660a5` and its saved working edits, read-only.
No whole-branch cherry-pick or source worktree mutation.

Already represented: desktop local-continuation callers (banked in 6781ac73),
source/output selection validation, prepared-code checks, case-component
collision checks, typed receipt declaration and harness union fix. Recovery
also has stronger deterministic local admission, canonical source-root checks,
immutable result-publication retries and resumable cleanup intent. Preserve
these rather than replacing them with the parallel versions.

Useful unique work to preserve selectively:
- Store overlap completion after partial writes; atomic materialization with
  digest verification; staging-file identity/link checks.
- Native directory identities, allocation-bound receipts, and the untracked
  Windows/POSIX privacy drafts. The Windows draft checks real DACLs; it needs
  creation-time ACLs, reparse protection and native CI before acceptance.
- Context-aware admission outside the remote journal mutex.
- Accurate prepared execution cwd/branch projection and artifact link checks.
- Source/predecessor selection, source pinning, selected-untracked report
  handling and report-path template projection need porting without weakening
  repository-root/ownership checks or frozen-result immutability.
- Saved native/owner-scoped tests and Task Inspector browser coverage provide
  additional witnesses; the old integration metadata must not overwrite the
  working request/workflow fixture on the recovery branch.

Remaining acceptance work: ordinary Windows source and returned custody with
native ACL/file identity checks; Windows-origin/Linux-execution/Windows-return
hosted fixture; remote scout evidence into Windows local implementation;
persisted storage reservations and bounded quarantine admission for a 10 GB
worker. Blanket Windows refusal is withdrawn as an accepted scope. Full Git GC
may remain manual only with bounded retained storage and truthful status.

## Applied on this branch

Ported the saved Store overlap/atomic materialization/link checks and owner-
scoped bus tests. Reworked the privacy draft to install Windows ACLs atomically,
verify actual file ACLs, support native extended paths, and use write-through
receipt replacement. Native allocation IDs now bind receipt consumption,
returned-custody reuse and cleanup. Admission carries context and does not hold
its journal mutex during filesystem verification. Prepared cwd/branch projection,
selected-untracked report handling and source pinning were incorporated while
preserving recovery's deterministic claims and immutable result retry behavior.
Case-component validation was already stronger here and was kept.

The wider saved source-selection/report-template/UI drafts were not wholesale
imported: they overlap current workflow validation and include behavior beyond
this minimum Windows contract. They remain preserved in the old worktree and
commit for deliberate follow-up. This branch supports the configured canonical
source checkpoint and same-owned-task returned custody for local continuation;
it does not claim arbitrary predecessor-worktree exports or automatic Git GC.

The minimum Windows/source-return-local-implementation path now has native
hosted fixtures. Storage admission/retention is bounded and exposed explicitly;
final run URLs and exact tested SHA are reported in the worker result, rather
than embedding self-invalidating final commit claims in this document.
