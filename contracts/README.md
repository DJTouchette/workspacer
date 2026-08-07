# Cross-language contracts

Golden fixtures that pin behaviour reimplemented in more than one language, so the
copies cannot silently drift. Each file is loaded by a test in **every** language
that owns a copy of the logic; the test asserts identical output for identical
input. If you change one implementation, the fixture forces you to change (or
consciously extend) the others.

| Fixture | Owners | Guards |
|---|---|---|
| `model-pricing-cases.json` | `apps/desktop/.../modelUsage.ts` (TS) · `services/claudemon/.../pricing.rs` (Rust) | Per-model USD rates. Longest-prefix match must price every listed id identically. |
| `deepmerge-cases.json` | `apps/desktop/.../configService.ts` (TS) · `services/hub/cmd/brain/config.go` (Go) | `deepMerge(target, source)` — the config.yaml overlay algorithm both writers use. |
| `session-schema.json` | `apps/desktop/.../shared/sessionSchema.ts` (TS) · `services/hub/cmd/brain/stores.go` (Go) | Saved-session format version. Both writers stamp it; a reader refuses a file stamped higher than its own rather than treating an unparseable shape as empty. |
| `config-lock.json` | `apps/desktop/.../lib/configLock.ts` (TS) · `services/hub/cmd/brain/configlock.go` (Go) | The advisory lock guarding config.yaml. `staleMs` and the lock filename must match — a side that expires locks sooner steals one the other still holds. |
| `host-trusted-config-cases.json` | `apps/desktop/.../lib/hostTrustedConfig.ts` (TS) · `services/hub/cmd/brain/config.go` (Go) | Config sections a bus caller may never write. Both `config.save` entry points must strip the same list — `updates.channel` reaches the updater feed URL. |
| `path-containment-cases.json` | `apps/desktop/.../lib/pathConfinement.ts` (TS) · `services/hub/cmd/brain/fsguard.go` (Go) · `services/hub/internal/bus/policy.go` (Go) | Canonicalize-then-contain for every caller-supplied filesystem path: per-component symlink resolution (so `link/..` is not collapsed textually), the credential/config-dir second gate, and — via the `methods` block, cross-checked against `capspec.PathParam` — that every path-bearing capability actually calls the guard. `services/claudemon` is deliberately **not** an owner: `transcript.rs`'s `is_within` is documented as purely lexical, confines one hook-supplied transcript path, and exposes no `fs.*` capability, so it would fail the symlink cases by design. |

Rates are USD per **million** tokens. Add a case whenever a new model id or merge
edge case ships; the cheapest place to catch drift is here, before it becomes a
mispriced session or a clobbered config.
