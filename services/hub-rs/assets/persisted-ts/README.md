# Current retained TypeScript writer captures

These three files are exact bytes copied from real TypeScript store writes by
`apps/desktop/src/main/services/persistedWriterCapture.test.ts`. They are not
hand-transcribed schemas, Rust serializer output, or files recovered from an old
user installation. The manifest records current writer/capture-source hashes and
the artifact hashes. Capture provenance includes actual random UUIDs and capture
times. The review's temporary Git repository/worktree paths are retained exactly,
and those directories were removed before the capture completed. No normalizer
has edited the artifacts to fit a Rust reader.

- `dispatch-history.json`: two actual accepted dispatches, one finished with
  observed token/cost metrics and validated result/evidence ID. The other remains
  starting in the stored file; Rust must project it stale/not-live after reopen.
  No requests existed, so the TS writer omitted the optional requests field.
- `fleet-review.json`: actual base/result Git commits in an isolated worktree,
  captured through `reviewAllocation` and `FleetReviewStore.capture`. The retained
  file diff is readable after the Git repository is removed. Optional readers
  are absent until an authorized owner-adoption mutation.
- `manager-replacements.json`: actual `rememberLaunch`, `rememberChild` and
  journal `edit` writes. It contains an activating committed operation with a
  sending delivery, retained launch/worker/completion metadata, and an optional
  pending child. Rust-only pendingSignatures is absent.

The Rust owning test target is `services/hub-rs/tests/persisted_ts_imports.rs`.
It reads these files through TaskStore, ReviewStore and ReplacementState, checks
read-time byte preservation and authority controls, then rewrites/reopens through
normal store APIs. It also checks the manifest/source provenance. The target is
part of ordinary hub integration CI; adding the file is not a passing execution
receipt. Initial preparation ran TS capture/typecheck/provenance tests only;
Rust execution is intentionally deferred to branch CI because of local disk limits.

```sh
# Source-only verification, also wired into desktop CI:
node scripts/capture-persisted-ts.mjs --check
node --test scripts/capture-persisted-ts.test.mjs
# Explicit recapture through current real TS writers, requires desktop deps + Git:
node scripts/capture-persisted-ts.mjs --write
# Owning Rust proof (run in CI or with sufficient build resources):
cargo test --locked --manifest-path services/hub-rs/Cargo.toml --test persisted_ts_imports
```

Recapture creates new real IDs/times/temporary paths, so byte-identical replay is
not promised. Review the new outputs and source hashes rather than stripping
those values. Routine `--check` verifies the recorded capture without generation,
Node package loading, Git or providers. Existing historical schemas, arbitrary
unknown root-envelope fields, and live process ownership are outside this fixture
claim. These captures do not justify changing production acceptance semantics.
