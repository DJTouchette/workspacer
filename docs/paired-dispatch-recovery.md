# Paired dispatch recovery — incomplete, not rollout-ready

Recovery commit `23e64e83b98f34d93549e28294b272136fdde339` preserves
`091583e75d4f3e81a3988749ad66264c0eb19019` and
`3fc1ef0c0bcd4518389e724f1f4acf548ada8499` atop production `3a16f4bb`.
The five tracked desktop modifications were copied with `git diff --binary HEAD`
and compared byte-for-byte with the recovery commit. Only the two named new source
files, `dispatchTargets.ts` and `remoteDispatchRegistry.ts`, were copied. The old
worktree was not modified; generated skills, caches and credentials were excluded.

## Additional patch

Dispatch admission events can only originate inside the router and can only be
consumed by the authenticated local host. Paired operators cannot publish dispatch
results; a local host or a provider granted `agents.spawn` can. Socket publishers
cannot assert a dispatch envelope's `hub` stamp. The event registry, renderer
consent rules and shared fixture classify all four dispatch topics consistently.

Read `services/hub/internal/bus/remotedispatch_events_test.go`, the changes in
`bus.go`, and `eventplane_test.go` first. The new fixtures cover authenticated
operator and federation publication forgery, asserted hub stamps, and provider
publication grants. They have **not run** and are not end-to-end acceptance proof.

## Confirmed unfinished seams

- `remoteDispatchRegistry.start` has no production caller. The registry cannot
  resolve a manager or load durable state in the recovered route.
- `dispatchTargets.ts` discovers `peers.json` entries with a new `dispatch` flag;
  existing secure pairing does not enable a desktop-local execution target.
  The manager tool, target UI, Inspector and default backend route are unfinished.
- `fleetWorkflowRuntime.ts` still owns local-only admission; returned remote
  updates bypass local task/request/workflow outcome and evidence recording.
- `rpc.go` labels every forwarding error a definite spawn failure. A timeout can
  instead mean uncertain admission. Do not retry it as a fresh dispatch.
- `remoteDispatchRegistry.ts` acknowledges before best-effort wake delivery,
  swallows persistence errors, and evicts open records before terminal records.
  Peer replay is process-local and retains only the final update. These do not
  establish the required durable reconnect/delivery contract.
- `dispatchreadiness.go` treats credential-file/account metadata presence as
  authenticated readiness. This is not proof of a usable login. Remote isolated
  worktree admission, rollback and lease/reservation behavior remain unverified.

These are implementation gaps, not accepted changes to the user's contract.
Keep the manager local; retain local request/task IDs, workflow pins and evidence;
send only supported execution metadata to the peer. Never copy local full-access
grants or credentials into worker arguments. Unknown/offline is not complete, and
no admission uncertainty permits blind spawn retries or local fallback.

## Verification and continuation

Local activity was limited to source/context reads, git, recovery byte comparison
and `gofmt`. `git diff --check` passed. Witness selection identified the bus,
capspec and dependent brain/MCP suites. No local build, test, typecheck, dependency
installation, browser, runtime mutation or child agent was used.

GitHub's existing `.github/workflows/ci.yml` is active and supports
`workflow_dispatch`. No branch was pushed and no hosted run was started: the task
also explicitly prohibits pushing without authority, and publication approval
was requested but had not arrived when this recovery note was written.

After publication authority is resolved, finish the seams above and run the full
relevant hosted CI on the exact feature-branch commit. Add the required real
local-MCP → paired host → remote brain → local task/wake fixture, including old
peer negotiation, uncertain admission, reconnect/dedup/adoption, initial message
delivery, limited credentials, remote cwd isolation and failed-worktree cleanup.
The current new tests cover only the event authority boundary.

After independent review and green hosted checks, coordinate upgrades of both
endpoints. A peer at `3a16f4bb` must remain honestly unsupported until upgraded.
Only with separate live-validation authority, discover a real remote repository
and current provider readiness, then run one benign Claude worker from a manager
that remains on the desktop. Verify one initial message, authenticated progress/
block/final wakes and local task/request outcome updates. No deploy, restart,
pairing/config change or real remote spawn has been performed here.
