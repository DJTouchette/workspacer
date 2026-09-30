# Provider scope and registration review

The shipped Electron path starts `serve --hub-only` with a borrowed claudemon
and sets `DELEGATE_CATALOG_TO_BRAIN=false`. Electron therefore registers both
its file-backed catalog and live/native methods. Rust maps that flag to
`control_plane_only`, which skips native `install_config`; its internal
log-only notification fallback cannot take the desktop notification slot on
this path. The notification handler in Electron records an in-app item and,
when enabled/supported, creates the OS notification.

The optional outbound worker is a separate composition. `provider_relay` filters
its actual installed handlers through explicit Catalog/Full registration sets.
Catalog excludes notifications, live sessions and process controls even when
the local private hub has more handlers. An engine-less local `brain.info`
label alone is not evidence of what the remote worker exports.

The portable `brain-capabilities.json` contract pins the declared scope sets.
`assets/provider-scope-overlaps.json` preserves all 55 legacy declared overlaps
without introducing a runtime dependency on retained Go files. The scope test
checks both missing overlap declarations and stale declarations, as well as
human-facing `ADOPTED-DEGRADED` markers. Those markers retain compatibility
warnings for adopted older hubs; they do not prove that a current Rust service
has the old Go implementation's limitations. Current service behavior has its
own owner tests (including persistent Rust analytics).

Unknown Rust scope strings are rejected by the typed configuration boundary;
they do not inherit the Go helper's fallback to the wider Full set. Default
scope selection remains explicit at the CLI boundary.

The legacy registration recovery assertion requires reclaiming a method after
a *different* stale provider is evicted, without reconnecting the already-live
worker. Evicting the worker itself only proves reconnect behavior. The new real
socket fixture holds `brain.info` on a predecessor, observes partial ack and a
working sibling method, then checks recovery on that sibling's exact connection.
It also checks that absent local handlers and private handlers are not exposed.

Validation and exact source hashes are recorded in the associated review plan
once the before/fix socket proof and scope checks complete. This document is
not itself a passing test receipt.

The Rust relay keeps the Go five-second retry cadence while grants are missing.
Each retry offers the same installed scope-filtered methods; acknowledgements
replace the accepted set after intersecting it with that offer. New conversation
and snapshot grants initialize their read feeds once, while duplicate replies do
not reseed snapshots. The timer lives inside the socket session and exits with
its cancellation. No mutation is replayed to repair registration.

The bus lifecycle deliberately adopts the modern Rust protocol: sensitive Bearer
credentials replace URL query credentials, authenticated caller-context version 1
is required, and Full workers use a separate upstream caller credential. The
bounded websocket limit is 64 MiB, dialing is bounded at ten seconds, and reconnect
backoff is one to thirty seconds (reset after a healthy thirty seconds). These
replace Go's 8 MiB read limit, five-second dial and one-to-sixteen-second retry
sequence. Power-stop close code 4001 pauses until explicit resume. Per-caller
cancellation/identity routing and correlated asynchronous replies are covered by
the existing relay owner tests; they are stronger protocol semantics than the
retained Go client, not claims of byte-identical transport behavior.
