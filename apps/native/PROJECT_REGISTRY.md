# Native project registry consistency

Registry reads and mutations share `Backend::project_write`, the existing
process-local FIFO transaction lock keyed by normalized hub URL. Each read or
verified write allocates a `revision` under that lock. This is local projection
metadata, never a config field or an external compare-and-swap token. The UI
compares revisions, while request numbers continue to identify user receipts.
A later-started refresh cannot sample a pending write's pre-save map. A delayed
older snapshot cannot replace a newer acknowledged snapshot. The controller
retains the newest successful registry separately from loading request slots;
all windows sharing that controller receive the same projection.

Each accepted `TouchProject` owns a separate future queued on the same hub lock.
Touches are neither superseded nor coalesced. They perform one read/patch/save/
verify round, with no spawn side effect or automatic retry. `lastOpened` is the
maximum of the incoming timestamp and all equivalent entries' stored timestamps.
The last 128 completed touch receipts retain request number, project, timestamp,
and error. This bounds receipt history, not pending work; pending touches are
never evicted by that limit. Existing connection-epoch handling and shutdown
lifetime still apply; this is not a durable launch journal.

Imported map keys are compared using `same_dir`: trailing separators and slash
variants normalize, Windows-shaped drive/UNC paths compare without ASCII case,
and POSIX case remains significant. It does not resolve symlinks, dot segments,
or filesystem aliases. Pin/touch updates the identity field on every matching
entry, preserving each key and its separate metadata. List rows union protection,
pins, and maximum recency, retaining the first nonempty identity metadata in
stable map order. Conflicting imported labels are not merged or overwritten.
Removal refuses if *any* alias is configured/malformed or scripts/widgets protect
it. Successful removal requires absence from every equivalent map key and both
legacy arrays; a skipped or half-applied save is an error.

The lock covers cooperating native backends in this process. External processes
can still race between config.get and wholesale config.save. This repair does
not change hub APIs, add external CAS, or claim cross-process atomicity. A
separate controller reads its own snapshots; windows of one host share a single
controller. Connection failures retain the existing inspect-before-retry policy.

## Verification

Run the complete suite serially, plus formatting, strict lint and the locked build:

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml --features ui-tests -- --test-threads=1
cargo fmt --manifest-path apps/native/Cargo.toml --check
cargo clippy --locked --manifest-path apps/native/Cargo.toml --all-targets --features ui-tests -- -D warnings
cargo build --locked --manifest-path apps/native/Cargo.toml
```

`registry_refresh_waits_for_pin_unpin_and_remove_transactions` holds the save
reply, starts a newer-number refresh, and proves no refresh RPC escapes the
barrier. Restoring the old Projects read branch makes this test fail.
`project_snapshot_revisions_beat_request_order_in_every_window` delivers inverted
request/revision order, late old reads, unpin, touch, removal and a refusal; it
also clicks the selected star to verify the next action is Unpin. Restoring the
old sync_projects method makes its first pin assertion fail. Protocol tests
exercise distinct/same-project touch bursts, refusal attribution and pin/remove
races; `projects_hub` verifies the changes on a real isolated hub's config file.

For an independent runtime check, run `python3
apps/native/scripts/project_registry_smoke.py --help`. Supply the native binary,
Xvfb/Openbox/xdotool paths, Openbox config, lavapipe ICD, required library/data
paths, an unused private display number and a **new** output directory. The
command starts and stops only its own processes, uses a fresh HOME and explicit
environment without inherited WKS overrides, and serves a provider-free fake
hub. It holds pin/unpin writes while Change requests a refresh, then captures
star/receipt state and verifies the subsequent star requests Unpin. It records
commit/binary identity, bus traffic, saved config and cleanup evidence. Screenshots
require human inspection. No real agent is launched. This is Linux X11/lavapipe
evidence, not Windows, macOS, native GPU or screen-reader validation.
