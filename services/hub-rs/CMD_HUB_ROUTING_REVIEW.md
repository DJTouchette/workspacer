# Hub routing adapters: source review

The reviewed source rows are `cmd/hub/routing.go`, `routing_test.go`,
`routingselect.go`, `routingpreferences.go`, and `routingpreferences_test.go`.
Exact source hashes and behavioral mappings are in `reviews/cmd-hub-routing.json`.
The final frozen-source library checkpoint passed 391 tests with no failures
or ignored tests (`/tmp/workspacer-watcher-barrier-full-lib.log`).

The Rust owner is `services/routing/sampler.rs`, installed for both standalone
and embedded backends. It obtains observations through the owned daemon client
or the explicitly configured external daemon. Callers cannot choose that URL
or supply their own usage observations.

The shared producer now has a ten-second fetch deadline independent of each
three-second caller wait. It publishes its result before notifying waiters;
cancelling the final waiter does not lose a late observation. Failed refreshes
invalidate the completed cache. Decisions and previews require a new reading;
Overview may reuse an observation for sixty seconds. The shared library
checkpoint at `/tmp/workspacer-shutdown-final-lib.log` passed 382 tests,
including cancellation, late publication, caller timeout, concurrent reads,
failure invalidation, report projection and the typed catalog decoder.

The old hub mirror's thirty-second prefetch and fifteen-minute idle tail are
retired. Actual quota polling belongs to claudemon's `account_usage::spawn_poller`,
started by `daemon/mod.rs`: live polling is sixty seconds, idle polling fifteen
minutes, and failures back off. The desktop `useUsageReport.ts` hook shares one
sixty-second report fetch while subscribers exist and stops when the last one
leaves. This preserves public observation-age bounds; it does not claim an
identical background schedule or duplicate the daemon's polling.

Preview is a narrow public projection: no decision ID, private ceiling key,
matrix, account, ticket, capacity or demand. Its strict request shape follows
the Go adapter. The shipped desktop request interface only sends role,
profile, provider and cwd. The actual-bus preview tests passed in the final
checkpoint.

Catalog HTTP failures and malformed answers remain unknown. A valid answered
empty provider list is unavailable; a positive answer is cached for ten
minutes. Catalog refresh is demand-driven by `routing.select`, with a
five-second minimum gap. The old service's deferred boot check and five-minute
retry timer are replaced by this demand-driven owner; preview and report do
not boot provider CLIs. The pending-catalog sentence marks the first-probe and
unanswered-provider windows. Unknown remains fail-open for routing, and is
never asserted to be measured-empty or available. Go's private
`Catalog.Models` error string
distinguished HTTP 502 from an unreachable daemon. Its production consumers
(`ValidateAgainstCatalog` and `answerCounting`) discard that text and retain
only the answered/unknown distinction; the exact string assertion lives in a
private adapter test. Rust retains the public state distinction without adding
an error-detail field to the public catalog.

The final checkpoint verifies:

- Actual HTTP catalog production: empty, malformed, positive TTL and failed
  probe controls, with no invented available state.
- Operator preview and owner preference save/select/preview/reopen behavior;
  view reads and non-host mutations remain refused.
- Positive catalogs exclude unsupported model/effort fallover candidates;
  unknown evidence retains the original candidate. Go used deferred
  `Matrix.Issues`; Rust evaluates the same evidence from the cached catalog.
- The actual decision discloses an unanswered catalog, known-empty candidates
  give a safe reason, and the pending predicate clears after positive answers.

The first extended checkpoint ran 389 tests: 387 passed and two preview
fixtures failed because they incorrectly used view tokens for operator-scoped
methods. Production authorization correctly refused them. Fixtures now use a
scoped operator for positive reads and explicitly assert view refusals; this
correction passed in the final checkpoint. No scope vocabulary changed.

The second checkpoint passed 388 of 390 tests. Its remaining preference
fixture compared an omitted private effort to the preview DTO’s required empty
string; the assertion now compares their identical text selection. A separate
watcher timing failure was corrected with a committed-scan barrier by its
owner. The subsequent 391-test pass covers the final combined source.
