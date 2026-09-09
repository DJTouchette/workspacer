# Isolated Linux release checks

Run each release command through `scripts/release-check.py` when working alongside
a live desktop app. A worktree and separate build caches do not isolate resources:
ordinary child processes inherit the launching app's cgroup. This utility launches
a unique `workspacer-release-check-<uuid>.service` under the user manager's
`app.slice`, alongside application scopes. It changes no app, provider, lifecycle,
global systemd configuration, or permissions.

Requirements: Linux with unified cgroup v2, `/usr/bin/python3` (3.9+), a reachable
user systemd manager, `systemd-run` supporting `--expand-environment=no` (254+),
and delegated memory, CPU and pids controllers. The bootstrap reads its own
`/proc/self/cgroup` and actual cgroup control files **before executing the check**.
A missing controller, unsupported property, wrong cgroup, failed launch or
mismatched limit fails explicitly. There is no execution fallback. The bootstrap
itself is small and runs under the requested service properties even when it
refuses the workload. No root access or configuration edits are attempted.

## Limits and environment

| Setting | Default | Explicit override |
| --- | --- | --- |
| `MemoryHigh` | 1536 MiB | `--memory-high 1536M` |
| `MemoryMax` | 2 GiB | `--memory-max 2G` |
| `MemorySwapMax` | 0 | `--memory-swap-max 0` |
| `CPUQuota` | 100% (one core of aggregate CPU time) | `--cpu-quota 100` |
| `TasksMax` | 128 (threads count too) | `--tasks-max 128` |
| `RuntimeMaxSec` | 3600 seconds | `--timeout 3600` |
| Go soft heap target | `GOMEMLIMIT=1GiB` | `--env GOMEMLIMIT=768MiB` |
| Go execution parallelism | `GOMAXPROCS=2` | `--env GOMAXPROCS=1` |
| Cargo build jobs | `CARGO_BUILD_JOBS=1` | `--env CARGO_BUILD_JOBS=1` |

Memory options accept integer bytes or binary K/M/G suffixes, including zero for
swap. High must be positive and no greater than max; unlimited values are refused.
CPU, tasks and time limits must be positive integers. These conservative defaults
bound a single package check, not an entire concurrently running release gate.
The 1 GiB Go target leaves room inside the 2 GiB cgroup for compiler/linker children,
race instrumentation and non-Go allocations; it is a soft runtime target, not a
total RSS limit. Large packages may still hit the ceiling. Stop and split their
test selection, or use a separate executor with more capacity. Raise limits only
after checking host headroom; the runner does not reserve RAM or protect against
other workloads/global OOM. Do not run multiple release runners concurrently.

`HOME`, `USER`, `LOGNAME`, `SHELL`, `PATH`, `LANG`, `LC_ALL`, `LC_CTYPE`, `TMPDIR`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`,
`XDG_RUNTIME_DIR`, `APPDATA` and `USERPROFILE` retain their caller values when set.
The three build variables above use the documented defaults. Other environment
variables are absent unless explicitly supplied with repeatable `--env NAME`
(copy from caller; unset is an error) or `--env NAME=VALUE`. For example, pass
`--env GOCACHE --env GOMODCACHE` if using caller-selected caches. Pass secrets by
name, never literal values in shell history. User-manager environment is not
merged into the executed command. Cwd defaults to the caller's current directory;
`--cwd PATH` overrides it. No shell parses the command or its arguments.

Environment, cwd and argv are carried in an anonymous input file, not systemd's
command/property metadata. The runner does not log them. Each invocation creates
a private mode-0700 directory under `.workspacer/release-checks/` (relative to the
caller, override with `--log-dir`). `output.log` retains merged stdout/stderr,
kernel cgroup evidence from the bootstrap, and systemd's completion diagnostics;
`result.json` retains the unique unit, exit code, cancellation signal and cleanup
confirmation. Output is written directly to disk without buffering it all in the
launcher. Tail the printed log path for progress. A check can print its own secrets;
review its output before sharing. Stdin for the check is `/dev/null`.

Ordinary command exit codes are returned unchanged. Bootstrap/preflight failures
return 125; argument errors return 2; systemd launch, timeout and OOM failures
return systemd-run's nonzero status (OOM is 1 on systemd 260, not shell-style 137).
The log distinguishes the service result such as `oom-kill` or `timeout`.
SIGINT/SIGTERM cancellation returns 130/143 unless cleanup cannot be confirmed
(125). Cleanup stops only that invocation's exact unique unit; it never signals
live PIDs, process groups, scopes or slices. `KillMode=control-group` cleans up
descendants; the service has a five-second stop timeout and
[`OOMPolicy=kill`](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#OOMPolicy=)
sets `memory.oom.group=1`, verified by the bootstrap. A launcher killed with
SIGKILL cannot perform cleanup; the service's finite runtime limit remains the
backstop. This is resource containment for trusted checks, not a security sandbox:
tests retain filesystem/network access and must not deliberately escape via
another service manager or contact live services.

## Sequential release recipe

From the release worktree containing this runner, execute an explicit reviewed
package/test selection and stop at the first failure. The following is a bounded
Go example, not a declaration that these packages are the whole release gate:

```bash
set -euo pipefail
for package in ./internal/routing ./internal/usageprefs ./cmd/brain; do
  python3 scripts/release-check.py --cwd services/hub -- \
    go test -race -count=1 -p=1 -parallel=1 -timeout=20m "$package"
done
```

Do not replace that selection with concurrent `go test -race ./...`, cargo and
desktop checks. If brain exceeds the ceiling, select reviewed `-run` groups and
retain evidence of their combined coverage; do not count an OOM or timeout as a
pass. `cmd/brain/hostgate_test.go` keeps its package-wide config-home sandbox;
`internal/routing/effort_test.go` can intentionally read the real routing file
read-only. This runner changes neither policy.

After the Go checks finish, run other explicit checks one at a time, for example:

```bash
python3 scripts/release-check.py --cwd services/claudemon -- \
  cargo clippy -j 1 --all-targets -- -D warnings
python3 scripts/release-check.py --cwd apps/desktop -- npm run typecheck
```

On a memory-constrained host, full Rust debug symbols can make the linker spend
minutes reclaiming memory at `MemoryHigh`. Keep the cap and use per-command
`--env CARGO_PROFILE_DEV_DEBUG=0 --env CARGO_PROFILE_TEST_DEBUG=0` for local
`cargo test -j 1 -- --test-threads=1` and Clippy checks if that happens. Cargo
rebuilds the affected artifacts and reports an unoptimized profile without debug
symbols; assertions and test selection are unchanged. Record the override and
retain the logs. Hosted CI still validates its default profile and platform
builds. Cancel only the unique check launcher before starting a replacement.

Keep unit-test temporary directories on their normal temporary filesystem.
Setting `TMPDIR` beneath the checkout changes Git discovery and home-based file
guard expectations. Private dependency/build caches can live in `.workspacer/`,
which contract-loader discovery excludes so cached source copies cannot count
as shipping loaders or inflate the guard's memory use.
Likewise, select an E2E scratch root outside live state, such as a unique directory
under `/tmp` via `--env WKS_E2E_SCRATCH=...`. The fixture deliberately refuses
every descendant of the real `~/.workspacer`, including allocated worktrees
beneath it. Do not weaken that guard to permit a checkout-local scratch home.

Tests that boot services must supply their own scratch state and ephemeral ports.
Preserve the desktop E2E fixture contracts in `tests/e2e/fixtures/scratchState.ts`
and `appHub.ts`: scratch HOME/XDG, live-port refusal and no live-state descriptors.
Pass explicit scratch variables with `--env` when a particular fixture requires
them. The generic runner never invents a new HOME, config identity or provider
account. Do not route app restart, port-sweep or live-session commands through it.

## Runner validation only

These commands do not run Go, Rust, desktop builds or test suites:

```bash
python3 -B -m unittest discover -s scripts -p 'test_release_check.py' -v
python3 -B scripts/smoke_release_check.py
# Optional: finite 96 MiB allocation, only after verifying a separate 64 MiB cap:
python3 -B scripts/smoke_release_check.py --memory-ceiling
```

Fake-systemd tests cover fail-closed behavior, enforced properties, literal argv,
environment selection, exit status and cancellation racing service startup. The
opt-in real smoke verifies cwd/argv/environment, exit 37, success and cancellation.
Its optional memory test verifies a 64 MiB ceiling, zero swap, group OOM policy,
CPU/tasks controls and at least 512 MiB available host RAM before allocating a
finite 96 MiB. High equals max only for this tiny disposable test: a lower high
can throttle reclaim until the runtime timeout before reaching the OOM ceiling.
It requires an actual `oom-kill` completion diagnostic, not just any failure.
An unsupported environment fails the smoke instead of reporting a skipped pass.
Logs remain under `.workspacer/release-check-smoke/` for independent inspection.
