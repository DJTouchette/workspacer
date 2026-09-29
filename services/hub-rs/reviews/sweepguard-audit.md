# Sweepguard infrastructure and host-case audit

All three source files were read, including the negative assertions. The exact
review plan is `sweepguard.json`; it distinguishes retained behavior from the
retired Go process-wide gate registry and does not certify final platform cutover.

| Retained source | SHA-256 |
| --- | --- |
| `services/hub/internal/sweepguard/gate_test.go` | `dc42a1d4d9e8299941905b61a8aeea661742c9667922f688cd60b2ad14cc65e6` |
| `services/hub/internal/sweepguard/sweepguard.go` | `dba00b4cea75f501ad278a84417e69df42d1e829579f0de0b283d40a3cd0008e` |
| `services/hub/internal/sweepguard/sweepguard_test.go` | `f2a788beae6152f607d259e6c3b014e87868dd886876ecf38ae346df0a4ab642` |

## Existing and completed replacements

`tests/support/sweepguard.rs` implements executed and enumerated counts,
allow/accept/ok/pass and deny/refuse/reject/fail vocabulary, unknown verdicts,
trimmed skip reasons, deterministic diagnostics, independent allow/deny floors,
deny-only floors, corpus shrink detection and strict executed-case floors.
`tests/sweepguard.rs` tests empty, half-empty, skipped and shrunken populations,
including mutations of the actual path and filename corpora. A population of
79 with 77 skips can satisfy enumeration without satisfying an execution floor.
Real consumers include files, stores, profiles, models, briefs and library tests.

The library selected-directory loader's injected symlink failure executes five
of seven cases, records both skipped cases, and fails its required three allow
and four deny executions. It does not report a successful corpus on a host
without symlink privilege. Files and stores require actual symlink creation.

`tests/support/repo.rs` replaces ambiguous ancestor/cwd discovery for corpus
ownership and vocabulary guards with the manifest's known repository root.
Both `Makefile` and `services/hub-rs/Cargo.toml` must be files. No retained Go
marker is required. Missing checkout, partial markers and missing input are
all hard errors; there is intentionally no vendored-checkout skip result.
`tests/repo_reader.rs` checks missing/partial/directory markers, real corpus
reads, moved inputs and changed bytes between runtime reads. This also avoids
Go's external-input cached-test result issue: Cargo executes these reads on
each test invocation. Compile-time fixture consumers fail compilation when
their required `include_str!` input disappears.

The audit also found and removed two unnecessary Unix-only test gates:
`derived_symlinks_cannot_escape_semantic_library_roots` now uses Windows file
symlinks, and `diff_refuses_symlink_escape_and_disables_external_diff_program`
uses Windows directory symlinks. Setup errors fail both tests. Native Windows
CI is still required to establish platform execution; Linux results alone do
not prove those branches.

## Gate replacement and caller accounting

The original `GateCounter` and `RunGates` enforced more than a nonempty fixture:
distinct test-name counting; repeat names count once; exact declared group
size rejects added and missing tests; the latest Ran/Skip verdict wins in both
orders; one starved group fails an otherwise successful unfiltered suite;
filtered runs and existing nonzero suite results retain their original result.
The direct tests also assert skipped-name/reason diagnostics. The migration
does not add an unused clone. Rust host-test setup errors fail rather than skip;
test function names are unique, and Cargo owns filtered-run and exit behavior.
Required executable case names are independently guarded against deletion,
ignore attributes, unsupported platform cfg and disabled ancestor modules.
Adding a new executing test no longer requires changing a Go registry count.

Transitive Go AST call inspection resolves exactly the declared 17 brain
symlink tests and 10 Git tests. Their executable replacements are grouped below;
the inventory in `sweepSourceGuard.ts` names the concrete Rust test functions.

| Old symlink caller(s), abbreviated after `Test` | Actual Rust assertion owner |
| --- | --- |
| ARejectedSessionSlotDoesNotFallBackOntoAnotherSessionsFile | stores collision-at-alias test preserves original session and outside bytes |
| EveryLibraryListWalkerGuardsTheFileItOpens; LibraryListDoesNotReadOutsideTheProjectItNamed | new four-walker test verifies escaped bytes absent **and ordinary item present**; derived alias test refuses saves |
| GitRunsInTheCanonicalCwdTheGuardReturned | actual Git canonical-cwd subprocess test |
| GlobalLibraryItemsSurviveASymlinkedConfigDir | new real global library save/list through symlinked configuration root |
| LayoutWriteAndDeleteRefuseAnEntryThatResolvesOutOfTheStore; SessionWriteAndDeleteRefuseAnEntryThatResolvesOutOfTheStore; StoreGuardsRefuseAnEntryTheyCannotResolve; StoreListersFollowASymlinkThatStaysInsideTheStore | stores boundary test covers both stores, escape and cycle, unchanged targets/links, quarantine refusal and in-store listing |
| LibraryDerivedPathsStayInsideTheSelectedLibrary; LibraryItemDirsRefuseEveryShapeOfTheComparison; LibraryListDoesNotLaunderHomeFilesThroughAnItemDirectory; LibraryWritesAndDeletesStayInTheProjectTheCallerNamed | actual seven-case selected-directory corpus plus public library redirection and four-walker controls; source retains shared canonical destination guard for save/remove |
| LibrarySaveWritesThroughTheGuardsAnswer | new project/Claude alias save tests check link survives and resolved target bytes change |
| ListEntriesResolvesASymlinkedDirectoryAsADirectory | new actual file/directory alias listing classification |
| StoreEntryPathReadLegReturnsTheResolvedFile; StoreWriteAndDeleteLegsUseTheGuardsAnswer | new both-store read/save/delete test checks canonical returned path, alias preservation, target mutation and target deletion |

| Old Git caller(s), abbreviated after `Test` | Actual Rust assertion owner |
| --- | --- |
| GitOutsideAWorkTreeFailsWithItsOwnMessage; GitReadsWorkInsideTheAllowedRoot; GitRunsInTheCanonicalCwdTheGuardReturned; HeadlessGitReviewActions | git selected-repository reads, canonical-cwd, malformed/outside repo and stage/unstage/commit/local-push tests |
| HeadlessFileWatchFreezesIdentityAfterMissingFileAppears; HeadlessFileWatchLeaseRenewalExpiryAndMissingFile; HeadlessFileWatchReplacementAndReferences | filewatch reference/replacement/missing-file, lease and symlink-swap tests; actual owned polling integration |
| ListEntriesDoesNotExecuteGitConfigCommands; ListEntriesHidesGitAndIgnored; ListEntriesHidesGitignoredNamesTheDesktopHides | files actual Git filtering, Git actual child fixed-prefix test and real Command prefix/diff-family twin assertions |

The bus's two registrations exclusively served the already-retired anonymous
`conn{caps}` authorization harness (ledger `internal/bus/hostgate_test.go`). They
are not silently counted as Rust filesystem-grant tests. Active canonical path
and link-budget tests remain independently executed.

## Guard mutations and validation

Rust Tally now checks its own lifetime: dropping an unread tally or unobserved
floor fails, and a later count invalidates an earlier floor. A delayed stale
result cannot certify newer cases. The negative battery also preserves an
existing panic without double-panic abort. Returned helper tallies still work.
Floor exposes only `unwrap`/`unwrap_err` assertions: boolean projections were
removed because discarding their result would otherwise hide a failed floor.
The exact owning `tests/sweepguard.rs` was recompiled directly with the cached
serde_json dependency after that correction: all five tests passed. The command
and log are recorded in the review JSON; consumers already used `unwrap`.

The TypeScript AST guard preserves all four source rules and differentiates
sibling counter symbols. Mutants remove the actual spawnCwd fixture floor,
add an unobserved source file, move counts into registration, introduce
catch-return/unaccounted capability gates, use comments as decoys, or hide a
required Rust test. The original mixed population measured 21 Go declarations
plus 29 TS declarations under a combined floor of 30; the complete unchanged
TS population is now independently pinned at 29, with host-reference floor 15.

Linux validation: reader 13; full library 362 at the bus checkpoint; owning
integration targets 55, zero ignored; TypeScript registry/policy 12 plus sweep
guard 4; source parameter guard zero errors. The final filewatch edit only
enables its existing symlink test on Windows and awaits native execution.
Windows CI at c0f40add passed existing selected-directory cases (including
simulated privilege failure), canonical Git cwd, and store alias boundaries:
https://github.com/DJTouchette/workspacer/actions/runs/36628039492/job/109610780781.
Newly added/enabled native branches still require latest-head CI.
