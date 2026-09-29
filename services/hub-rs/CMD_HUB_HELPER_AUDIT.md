# Remaining cmd/hub helper audit

Read-only source checkpoint, 2026-09-29. The current backlog has ten files:
launch integration, main, and routing/usage adapters plus their tests. Host-only
HTTP helper/test rows are already recorded; there are no separate pending
shutdown or health helper files. Their Rust owners remain plugins/http,
server/policy, Core health and the owned backend shutdown.

## Launch integration

`launchintegration.go` authorizes the old provider callback before and after a
20-second plugin call, validates the enabled selected manifest, version1,
plugin-owned declared method, agent support and bounded context shape.
Rust `plugins/launch.rs` and `agent_spawn.rs` replace the callback with an opaque
Core `LaunchPermit`, bound to the live owner operation, session, plugin and
runtime generation. Preparation checks the permit around facade setup and the
hook, checks the manifest again afterward, consumes the permit once, validates
patch shape and refuses host-owned model/facade/identity overrides.

Existing unit proof covers the context allowlist, grant revocation after a slow
hook, forged JSON selection, generation cleanup, unsafe patch fields and argv /
environment overlays. Runtime launch-permit tests cover caller disconnection,
wrong method/plugin/provider and expired/cancelled pending operations. No new
launch implementation gap was found in this bounded comparison.

One actual consumer was stale: `deploy/fly/combined/verify-web-capabilities.py`
required the retired `plugins.prepareLaunch` method. The verifier now requires
public `agents.spawn` and `plugins.manifests` plus authenticated
`health.launchReady === true`. Five local positive/negative tests include the
Python-assembled remote program with mocked SSH/network; they do not call a live
machine. The image still deliberately includes Node22 for providers/plugins,
verified by its generated Dockerfile and verify-image.sh. No runtime dependency
or public LaunchPermit endpoint is added.

## Routing ceiling and audit adapter

`routingceiling.go` injects current matrix verdicts into Go bus admission and
projects the resulting spawn receipt to the decision log. Rust RoutingService,
SpawnAudit, owned coordinator and source Origin now cover the actual dispatch
paths, including canonical/default cwd, independent peer policy, safe concrete
model tuples, exact-model refusal, fresh-session refusal and exactly-once
routing-phase audit. See bus-ceiling/freshness/core review plans for specific
assertions rather than inferring parity from aggregate test counts.

The fixed audit reason names routing.yaml without copying arbitrary payload or
error text. Rust records effective provider/model/effort at the spawn level;
legacy callback structs and duplicated nested tuple fields are not a new public
RPC contract. The legacy unresolvable-cwd log sentence is represented by an
omitted canonical cwd plus the default ceiling key in the structured receipt.
No production code was changed by this helper audit. These notes do not certify
the remaining large routing sampler/preferences/usage adapters or main entry
point, and do not add migration records without parent review.
