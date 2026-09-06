# Codex driver cleanup — slice B review handoff

Date: 2026-09-06. Branch: `wks/workspacer-owned-codex-driver-cleanup`.
This is a local isolated change for independent review and subsequent local
merge by the manager. No push, merge, service restart, or live-agent cleanup
was performed.

## Review entry points and behavior

Read `services/claudemon/src/providers/codex.rs` (`DriverEvidence`,
`OwnedAppServer`, `start_appserver`, `run_session`, and the local fixtures),
then `services/claudemon/src/session/store.rs::deregister_managed`.

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
  generation: a stale driver must still clean its own processes. Shared-store
  teardown remains generation guarded. Startup/fallback checks prevent an
  already superseded attempt from registering replacement channels.
  `deregister_managed` holds the generation read guard through its mutations
  and emits no duplicate SessionEnd for a row already stopped.
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
  already superseded startup cleans its own child without replacing channels.

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
Windows/non-Unix retain direct-child kill/reap and explicitly report
`DirectChildOnly`, never GroupGone. Existing daemon-wide Windows job-object
confinement is unchanged; per-generation Windows jobs require separate design
and runtime validation. No daemon-wide job is terminated for a session.

## Checks and reproduction

Environment: Linux 7.0.9-arch2-1 x86_64, Rust/Cargo 1.95.0, Python 3.14.5.
Builds use the ignored worktree `target/`, outside `/tmp`.

- Full `cargo test`: 849 library tests passed, 4 ignored; 8 integration tests
  passed; 1 doc test ignored. This includes provider, store and daemon tests.
- Final provider suite (including Codex rollout): 89 passed, 2 ignored, after
  adding the superseded-startup assertion.
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
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="bwrap --die-with-parent --unshare-pid --unshare-net --unshare-ipc --ro-bind / / --bind $PWD/target/test-home /home/djtouchette --bind $PWD $PWD --tmpfs /tmp --dev /dev --proc /proc --"
cargo test --manifest-path services/claudemon/Cargo.toml
```

The PID/network namespace permits loopback fake servers but no live daemon
access. Home is mounted from an ignored fixture directory without changing
HOME. Focused commands append `--lib direct_child_cleanup_gap_fixture`,
`--lib owned_group_cleanup_fixture`, `--lib fake_appserver`, or
`--lib superseded_startup` to the same cargo invocation. The fixture scripts
are embedded in the existing provider test module; generated scripts and logs
remain ignored in `target/`.
