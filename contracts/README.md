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

Rates are USD per **million** tokens. Add a case whenever a new model id or merge
edge case ships; the cheapest place to catch drift is here, before it becomes a
mispriced session or a clobbered config.
