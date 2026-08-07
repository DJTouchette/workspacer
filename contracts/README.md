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
| `filename-slug-cases.json` | `apps/desktop/.../lib/fileUtils.ts` (TS) · `services/hub/cmd/brain/slug.go` (Go) | The on-disk NAME for `library.save` / `layouts.save` / `sessions.save`, which either writer may produce. Case folding is not the same operation in the two languages — `strings.ToLower` is a simple per-rune fold, `String.toLowerCase` is full Unicode special casing and can lengthen a string — so `İ` slugged to `aib` in Go and `ai-b` in TS and the item became invisible to one provider and undeletable by both. Both now fold ASCII only. |
| `path-containment-cases.json` | `apps/desktop/.../lib/pathConfinement.ts` (TS) · `services/hub/cmd/brain/fsguard.go` (Go) · `services/hub/internal/bus/policy.go` (Go) | Canonicalize-then-contain for every caller-supplied filesystem path: per-component symlink resolution (so `link/..` is not collapsed textually), the credential/config-dir second gate, and — via the `methods` block, cross-checked against `capspec.PathParam` — that every path-bearing capability actually calls the guard. Each `allow` case also carries `resolvesTo`, the path the guard must RETURN: the corpus used to pin only the verdict, and a copy that returned a textually-cleaned path instead of the resolved one passed every case in all three languages while handing back a file outside the root. The `sessionFilenames` block is a fourth copy with its own shape (`sessions.*` take a bare basename, not an absolute path), loaded by `sessionService.test.ts` and `cmd/brain/stores_test.go`. The `projectDirNames` block is a fifth (`claude.sessionsForDir` encodes a cwd into ONE `~/.claude/projects` component — `..` used to survive the encoding and climb out of it), loaded by `claudeSessionList.test.ts` and `cmd/brain/providerparity_test.go`. `methods` carries two normative columns: `rootSet` (which allow-list confines the caller's field) and `derivedRootSet` (the narrower list confining paths COMPOSED from it — `library.*` only). `services/claudemon` is deliberately **not** an owner: `transcript.rs`'s `is_within` is documented as purely lexical, confines one hook-supplied transcript path, and exposes no `fs.*` capability, so it would fail the symlink cases by design. |

Rates are USD per **million** tokens. Add a case whenever a new model id or merge
edge case ships; the cheapest place to catch drift is here, before it becomes a
mispriced session or a clobbered config.
