# Codex driver cleanup — slice B review handoff

Date: 2026-09-06. Branch: `wks/workspacer-fix-codex-generation-ownershi`.
Cleanup baseline: `9b12360880b7386314fc5eaddd81ab826c3ba14f`.
Repair implementation commit: `09c5e791f54fd6767dd3d49ff3e19517660ca213`.
The baseline was merged into this isolated branch, preserving landed cwd fix
`ab85e61a`. This handoff accompanies the repair for fresh focused concurrency
review before a manager's local merge. No push, primary-branch write, service
restart, or live-session operation was performed.

## Review entry points and behavior

Read these repair files in order:

1. `services/claudemon/src/daemon/spawn.rs::handle_managed`: Codex claims its
   generation and publishes its row/metadata/transport/resume history in one
   synchronous transaction, then passes that exact token to `spawn_session`.
2. `services/claudemon/src/session/store.rs`: `claim_generation_with`,
   `with_generation`, `deregister_managed_with`, `record_output_owned`, and
   the deterministic generation fixtures.
3. `services/claudemon/src/providers/codex.rs`: `spawn_session`, `run_session`,
   `run_rollout_fallback`, `finish_driver`, and registration race fixtures.
4. `services/claudemon/src/providers/mod.rs::spawn_attach_pty_owned` and
   `services/claudemon/src/providers/codex_rollout.rs::apply_for_generation`:
   delayed Codex output/update ownership. The legacy PTY entry points remain
   available for their existing callers.

The accompanying files are this handoff and
`.rivet/learnings/2026-09-06-codex-ownership-must-cover-publication-and-regis-fb72be.md`.

- Publication, RPC input/decision/model/interrupt/yolo installation, hybrid PTY
  installation, fallback PTY/input/transport/notice installation, and shared
  teardown all serialize on the exclusive generation entry. Conversation
  removal is inside teardown, so an old exit cannot erase newly seeded history.
  Codex event application, PTY output and fallback rollout updates recheck
  ownership at their synchronous mutation boundary.
- Lock order is generation then synchronous registries/conversation. Do not
  nest generation operations: separate ids may share a DashMap shard. No
  generation guard crosses an await. PTY output waits on its buffer first,
  then acquires generation for publication; registration/teardown never wait
  on the buffer mutex. Owned process cleanup runs outside these transactions.
- Shared-helper caller verification: OpenCode and Pi still use
  `spawn_attach_pty` with the original output path; the ordinary PTY spawn
  route still uses `spawn_tailer` without a managed token. Claude-stream,
  Copilot, OpenCode and Pi retain their existing generation claim points and
  use the unchanged `deregister_managed` signature. Their lifecycles were not
  rewritten. The complete provider/store suites cover these callers.

- Lifecycle tracing uses a fixed reason/outcome vocabulary, session id,
  generation, direct app-server PID, Unix PGID, and the reaped exit status.
  There is one `appserver_started` record, one app-server terminal/start-failed
  record, and one `driver_terminal` record per normal attempt/lifetime.
  Fallback preserves the failed app-server record before recording its own
  eventual driver exit. No prompt, protocol body, argv, token count, credential,
  or raw error is added to these records. Evidence is constant-sized per
  driver; emission is bounded independently of turns/messages. Records use the
  existing tracing sink and its retention policy, not a new historical database.
- Every existing driver exit arm names its reason. Writer failure now wakes the
  owner as well; its task is held in a JoinSet so driver cancellation aborts it.
  Startup readiness is capped at 10 seconds and websocket connection at 5
  seconds, with direct-child exit observed during either wait. No reconnect
  behavior or idle timeout was added.
- Unix app-server spawn creates a dedicated process group through Tokio's
  `process_group(0)`. `waitid(WNOWAIT)` observes the direct child's death without
  reaping it. That unreaped child reserves the PID used as the PGID until the
  one SIGKILL group signal is sent. Teardown then waits at most 2 seconds to
  reap the direct child and 2 seconds to observe group disappearance. After
  reap, probes use signal zero only; repeated cleanup returns its stored
  outcome and never sends another numeric group signal.
- Process ownership is the captured child handle/group, not the current store
  generation: a stale driver still cleans its own processes. Generation
  transactions prevent supersession between a check and shared registration
  or teardown. Teardown emits no duplicate SessionEnd for an already stopped
  row and no SessionEnd for a superseded driver. The original Linux process
  tree fixtures and lifecycle reason/correlation/privacy vocabulary are unchanged.
- A residual group, failed signal, failed wait, or timeout is a cleanup failure,
  logged with a distinct outcome and returned as an error. A failed app-server
  cleanup also prevents entering fallback. SessionEnd still means the driver
  ended; it is not a claim that cleanup succeeded. UI closure rules are unchanged.
- `turn/completed` remains Idle; subagent events remain Subagent updates. The
  real fake-server loop verifies they leave the app-server running and produce
  no root SessionEnd.

## Runtime proof and limits

The baseline fixture ran **before** replacing direct-child teardown:
`start_kill` plus direct-child `wait` left its known forked descendant alive.
The fixture then explicitly killed and reaped that descendant. This is proof
of a local cleanup gap, **not** reproduction or attribution of the historic
orphan incident. Its original exit reason and descendant detachment remain
unknown.

The hardened Linux fixtures prove:

- child and same-group descendant terminate and are reaped, including when the
  direct child exits first;
- repeated cleanup causes no second signal, and separate successor and healthy
  idle processes survive old-generation cleanup;
- a deliberately unreaped descendant yields `GroupStillPresent`, including
  through the actual websocket driver, which returns an error;
- actual fake websocket close, malformed websocket frame, all four managed
  channel closures, startup child exit, missing binary, readiness timeout and
  connection refusal produce their expected reasons and cleanup outcomes;
- a direct-child exit while its descendant holds the websocket open produces
  `ChildExited`, preserving exit status 7, then cleans the group;
- a failing sink reports writer failure without requiring a read event;
- one SessionEnd per owning lifetime, zero for a superseded lifetime, and an
  already superseded startup cleans its own child without replacing channels;
- deterministic barriers after ownership checking and during successor row
  publication exclude overlapping registration/teardown; conversation removal
  excludes successor history publication;
- per-driver hooks supersede between the former startup/fallback precheck and
  registration, in both headless and hybrid modes: successor channel delivery,
  yolo identity, PTY identity/liveness, wrapper, row, transport and history
  survive, with no stale SessionEnd or degradation notice;
- output superseded while awaiting its buffer cannot publish stale bytes;
  rollout application stops on supersession/end; an owning fallback still
  installs its terminal transport and emits exactly one SessionEnd.

Linux descendant fixtures run in dedicated test subprocesses which alone set
`PR_SET_CHILD_SUBREAPER`; they reap only fixture PIDs. The daemon does **not**
become a subreaper. No process enumeration or selection by cwd, provider name,
or idle state is used.

**Containment limits:** GroupGone means the created Unix group disappeared.
A descendant which deliberately changes group/session can escape this boundary;
this slice neither detects nor claims to contain it. A cgroup or equivalent
stronger containment design is pending. Descendant reaping in production belongs
to the OS/adopting parent; a lingering zombie correctly produces a warning.
Cancellation/panic uses best-effort Drop signaling and reports unverified
cleanup; asynchronous reap cannot be guaranteed during runtime shutdown.
The hybrid/fallback PTY and throwaway model-list process keep their existing
process helpers; this change's verified boundary is the managed app-server.

**Platform limits:** Runtime verification is Linux only. Other Unix targets use
the same POSIX group/waitid code but were not cross-compiled or executed here.
A Windows cross-check with system Rust 1.95 failed because that sysroot lacks
Windows std/core. Retrying with the already installed Rustup 1.94.1 Windows
MSVC target reached native dependencies but failed because MSVC `lib.exe` is
absent (see ignored `target/windows-check-194.log`). No Windows build or runtime
success is claimed, and no toolchain was installed. The non-Unix owned-child
source branches are unchanged by this repair.
Windows/non-Unix retain direct-child kill/reap and explicitly report
`DirectChildOnly`, never GroupGone. Existing daemon-wide Windows job-object
confinement is unchanged; per-generation Windows jobs require separate design
and runtime validation. No daemon-wide job is terminated for a session.

## Checks and reproduction

Environment: Linux 7.0.9-arch2-1 x86_64, Rust/Cargo 1.95.0, Python 3.14.5.
Builds use the ignored worktree `target/`, outside `/tmp`.

- Focused `cargo test --lib generation_`: 9 passed.
- Full `cargo test`: 857 library tests passed, 4 ignored; 8 integration tests
  passed; 1 doc test ignored. This includes provider, store and daemon tests.
- `cargo test --lib providers::`: 254 passed, 3 ignored, including the unchanged
  Linux owned-process fixture and actual fake websocket idle/root-child cases.
- `cargo test --lib session::store::tests`: 97 passed.
- Tests use the isolated HOME files/PID/network namespaces shown below. Logs
  are in ignored `target/{race,provider,store,full}-tests*.log`.
- Rivet `witness select` reviewed: its cross-stack co-change suggestions are
  broader than this Rust ownership change; full claudemon checks were selected.
- `cargo fmt --check` and `git diff --check`: passed.
- Strict `cargo clippy --all-targets -- -D warnings` hits two **pre-existing**
  Rust 1.95 `collapsible_match` findings: `codex.rs`'s `"error"` translation
  arm and `copilot.rs`'s `"session.todos_changed"` arm. Neither was changed.
  `cargo clippy --all-targets -- -D warnings -A clippy::collapsible_match`
  passed; no lint suppression was added to source.
- The real authenticated Codex probe remains ignored. Websocket EOF, writer
  task panic, child-wait system-call failure, and hybrid discovery/fallback
  exit arms are instrumented but not each exercised by an end-to-end fixture.

To run all tests without exposing live home files, processes or network
services, from this worktree root (Bubblewrap required):

```sh
mkdir -p target/test-home
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="bwrap --die-with-parent --unshare-pid --unshare-net --unshare-ipc --ro-bind / / --bind $PWD/target/test-home $HOME --bind $PWD $PWD --tmpfs /tmp --dev /dev --proc /proc --"
cargo test --manifest-path services/claudemon/Cargo.toml
```

The PID/network namespace permits loopback fake servers but no live daemon
access. Home is mounted from an ignored fixture directory without changing
HOME. Focused commands append `--lib direct_child_cleanup_gap_fixture`,
`--lib owned_group_cleanup_fixture`, `--lib fake_appserver`, or
`--lib superseded_startup`, `--lib generation_`, `--lib providers::`, or
`--lib session::store::tests` to the same cargo invocation. The fixture scripts
are embedded in the existing provider test module; generated scripts and logs
remain ignored in `target/`.
