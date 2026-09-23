# Agent worktree artifact cleanup

Workspacer automatically reclaims eligible generated artifacts from stopped agent
worktrees. It starts checking one minute after local desktop/headless startup,
then every 15 minutes. The default minimum age is one hour. Idle agents are still
live and are excluded. Remote-client desktop mode does not clean local files.
The age gate applies to the allocation, associated stopped sessions, and the
newest write within each artifact directory. Fresh output is left alone.

The cleanup preserves source edits, branches and worktrees. It removes only
recognized ignored build/dependency artifacts that pass the core's worktree,
path and daemon safety checks; it does not traverse dependency links into another
checkout. A compatible running daemon must support the worktree maintenance
protocol before any deletion can occur. Old daemons or unavailable session state
cause cleanup to skip rather than infer that no agents are running.

Candidates are limited to managed linked worktrees under the configured root.
New allocations carry directory identity in their Git administration directory;
older allocations require a `wks/` branch. Primary checkouts, arbitrary temp
directories, locked worktrees and unknown directories are excluded. Artifact
names alone are insufficient: `node_modules`/JS outputs require `package.json`,
`bin`/`obj` require a corresponding .NET project (`bin` also supports Go), and
`target` requires Cargo or Maven metadata. Python/Gradle/CMake outputs have
equivalent recognizers. Git must confirm there are no tracked files inside and
the directory is ignored. Nested Git checkouts are preserved.

Shared dependency references are checked across Git's registered worktrees,
including the primary checkout, other configured roots, and links nested in
dependency/hidden directories. Links are inspected, never traversed. An incomplete
or over-budget reference scan skips cleanup. Byte estimates exclude symlink
targets and multiply-linked files; actual filesystem free space can differ.

Cleanup and daemon admission share an exclusive per-worktree maintenance file.
Worktree creation also locks its source while linking dependencies. A launch
during cleanup is refused with a retry message. Locks never expire by age: if a
process is killed while holding one, inspect its recorded PID and token and
confirm all owners are stopped before manually removing the abandoned
`.workspacer-maintenance.lock` from that worktree's Git administration directory.

Configure this in `config.yaml` (both desktop and headless use the same fields):

```yaml
agents:
  worktreeRoot: "" # default: ~/.workspacer/worktrees
  artifactCleanup:
    enabled: true
    minAgeHours: 1
```

Set `enabled: false` to disable automatic cleanup. Configuration is read each
cycle; no restart is needed. Use an absolute worktreeRoot if customizing it.

From `apps/desktop`, preview cleanup without deleting anything:

```sh
npm run cleanup:agents
```

Apply the eligible removals explicitly:

```sh
npm run cleanup:agents -- --apply
```

Both commands honor the configured worktree root and minimum age. Override with
`--root /absolute/path`, `--min-age-hours 24`, or `--daemon-url http://127.0.0.1:7891`.
The default daemon port also honors `WORKSPACER_PORT_OFFSET`. Disabling automatic
cleanup does not disable an explicit CLI apply.

The CLI prints JSON containing artifact paths and planned/removed bytes, skipped
worktree reasons and errors. Automatic runs write `[worktree-artifact-cleanup]`
reports to stderr/main logs when there are candidates, skips or errors. Review
skip reasons to distinguish protected live worktrees from unavailable daemon
state. No git branches or uncommitted source changes are removed.

Tests: `npm run test:cleanup-agents` covers CLI argument/config handling; the
scheduler and cleanup core have focused Vitest tests using disposable fixtures.
