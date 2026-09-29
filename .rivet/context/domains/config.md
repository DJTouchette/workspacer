---
title: Configuration (two writer implementations)
tags: [config, settings, defaults, concurrency, mtime-gate, config-lock, wholesale]
related_paths:
  - "apps/desktop/src/main/services/configService.ts"
  - "services/hub/cmd/brain/config.go"
  - "services/hub/cmd/brain/config_defaults.json"
  - "apps/desktop/src/main/services/configDefaults.generated.ts"
  - "apps/desktop/scripts/gen-config-defaults.mjs"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Configuration: writers, merge semantics, and state loss

## Sources and ownership

`config.yaml` has two active writer implementations: TypeScript
`apps/desktop/src/main/services/configService.ts` inside Electron and Rust
`services/hub-rs/src/services/config.rs` inside the standalone or embedded backend.
The private Node companion has been removed. Go
`services/hub/cmd/brain/config.go` remains a reference during the retirement audit;
it is no longer the default backend. “Two writers” does not mean exactly two
running processes.

`services/hub-rs/assets/config-defaults.json` is the defaults source. The desktop
generator emits main/renderer default modules from it; do not independently edit
those outputs. Default keybindings also have a preset representation whose
contract tests must remain aligned. Types do not validate arbitrary YAML.
`ProjectIdentity` is shared through `main/shared/ipcTypes.ts`, with renderer
re-exports rather than another independent project-field definition.

Both writers merge defaults with user configuration and perform explicit
migrations/normalization. Unknown keys are not indiscriminately pruned; retired
keys have a named removal policy. Model/context and manager preferences have
presence-aware normalization, including explicit null context preferences.
A generic deep-merge change must not erase that meaning.

## Read caching and save transaction

Cheap reads compare an mtime/size stamp and reload on inequality. Saves do not
trust that read optimization: they re-read unconditionally under the shared
exclusive lockfile, merge the partial, re-check the stamp and atomically publish.
The bounded stamp retry handles many nonparticipating editor writes; it is not
a filesystem compare-and-swap atomic with the final rename. An outside writer
can still race after the last check.

`contracts/config-lock.json` pins lock filename and the ten-second stale threshold.
TS waits up to 250 ms synchronously; Rust's wait budget is two seconds. The lock
serializes cooperating writers across refresh→merge→write; mtime checking alone
would leave the read/modify/write race open. A timeout never means write anyway.
Read-time migrations and seed helpers have their own paths, so inspect those too
when changing publication rather than assuming every write calls public save.

The Rust writer additionally checks against replacing existing non-default content
with a bare defaults-shaped file (`refuseWipeWithDefaults`). This guard does not
make every arbitrary partial safe or eliminate the wholesale-map rules below.

## Merge and replacement semantics

Ordinary nested partials deep-merge. Null/undefined normally means no replacement,
except where a presence-aware field contract defines explicit null meaning.
Exactly these user maps are replaced wholesale when supplied:

- `ui.customThemes`
- `claude.budgets`
- `projects`

Send the complete desired remaining map, not only the changed entry. `{}` means
empty the map; null, arrays, strings and other non-object values are refused.
The shared TS list is `main/shared/configWholesale.ts`; Rust and the facade are
pinned to it through `contracts/wholesale-config-paths.json` including value cases.
A malformed map must never be coerced into a successful empty replacement.

Renderer `ConfigContext.save` uses `minimalConfigPatch` so stale unchanged
siblings are not resent. That diff must not recurse into wholesale maps: trimming
one to the changed entry would delete the others. A new wholesale path needs
updates to the fixture, validators, facade schema, patch logic and tests.

The ordinary bus `config.save` path strips host-trusted sections/subpaths before
merging, including the currently declared update/script/launcher-sensitive
settings. Consult the concrete lists and their shared contract before adding a
key. The owner `desktop.saveConfig` service is a separate surface; do not infer
identical filtering merely because both end at configuration storage.

## Failure outcomes

Distinguish malformed existing data from a transient write problem:

| Condition | Current behavior |
| --- | --- |
| Existing file unreadable, unparseable, empty or not a configuration map | Protect it with persistBlocked; use fallback data in memory |
| File disappears after it was loaded | Retain the known in-memory value and block persistence |
| Save while persist-blocked | A merged value may be returned in memory, without writing the protected file |
| Lock timeout, write failure or exhausted stamp retries | Log and retain/return the prior configuration; do not latch persistBlocked |
| Invalid selection or wholesale-map request | Refuse through the relevant validation error path |
| Missing file on first load | Seed defaults; the Rust loader also diagnoses suspected loss of an established install |

A returned object is therefore not always proof a requested change was saved.
`ConfigProvider` warns on a rejected promise, but a host returning the prior value
is a different outcome. Do not claim all persistence failures throw or that every
successful RPC adopted the caller’s partial.

Malformed YAML backup behavior preserves recoverable bytes in a timestamped
.broken file; a backup failure does not authorize overwriting the original.
Correcting the file and reloading can clear persist-blocking. Lock/write failure
must remain retryable rather than becoming a permanent load-failure latch.

## Create-once state and identity

The Rust `state_loss` and TS `stateLoss` helpers look for other state beside a
missing file. Empty pre-created directories are not evidence of previous use;
files, including zero-byte files, and nonempty directories are evidence. This is
a diagnostic heuristic, not a definitive filesystem history.

Different loaders deliberately respond differently:

- Missing host pairing token amid prior state: `workspacer serve` refuses unless
  supplied a credential or explicitly allowed a new identity. The desktop warns.
- Missing/unreadable VAPID key with subscriptions: push generates a new key,
  warns and drops now-invalid subscriptions. Failure constructing push disables
  push, not the entire hub.
- Missing config at the Rust backend's first read amid prior state: warn and seed
  defaults without persistence-blocking, since other state may predate config.
  Do not confuse this with an existing unreadable file or mid-run disappearance.

A regenerated credential is a new identity; it cannot preserve old pairings.
Read the token/push-specific guide before applying config recovery behavior to a
credential file. Preserve the established diagnostic behavior when changing
either writer.

## Renderer and generated-state maintenance

TS configuration changes are emitted and directory watching survives atomic inode
replacement. Renderer updates and minimal patches reduce stale UI write-back;
they are not a second cross-process lock. High-frequency controls such as sidebar
width update local state while dragging and persist at defined boundaries rather
than synchronously writing YAML on every pointer movement.

Generated defaults, keybinding presets, project identity and manager selections
have distinct shared contracts. Run the relevant loaders rather than assuming
one successful config-service test covers every mirror. From `apps/desktop`,
`npx vitest run src/main/services/configService.test.ts` exercises the TS writer;
from the repository root, `cargo test --manifest-path services/hub-rs/Cargo.toml
--test config` exercises its Rust counterpart, and the library's
`services::config` tests cover write failure and concurrent-save seams.
Source-changing work also needs generator drift/type checks and
renderer patch/save tests.
