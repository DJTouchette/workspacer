# Adversarial hardening workflow

`adversarial-round.js` is the reusable, parameterized form of the sixteen-commit
hardening pass documented in `docs/adversarial-hardening.md`. That pass was run as ~10
one-off scripts (`antidrift-*`) with every path, commit hash, and test command hardcoded —
so none could be re-run or pointed at another surface. This is the same method with all of
that lifted into `args`.

## One round = one invocation

Deliberate. Each round banks its results before the next starts, so an interruption costs
one round, not a whole pass (see §2.1 of the doc). Run a round, read the return value, feed
its findings into the next round's `knownGaps`, repeat. Do **not** wrap this in an internal
loop — that reintroduces the failure the split was built to avoid.

The round is `Map -> Hunt (N parallel lenses) -> Close -> Verify`. Hunter, closer, and
verifier are separate agents on purpose: a hunter that can edit starts fixing instead of
finding; a closer that grades itself passes itself.

## Invoke

```
Workflow({ name: 'adversarial-round', args: { ...see below... } })
```

`args` fields:

| field | required | what |
|---|---|---|
| `lenses` | ✅ | `[{ key, task }]` — the narrow hunters. This is the yield lever (see below). |
| `testCommands` | ✅ | the exact suite commands, pasted into closer + verifier |
| `round` | | integer label |
| `baseline` | | what "green" means right now (commit + counts) |
| `surface` | | the plane under attack this round, in prose |
| `threatModel` | | the prize an attacker gets — grounds severity |
| `mapPrompt` | | how to enumerate the surface into an inventory |
| `knownGaps` | | `["title", ...]` do-not-re-report, **passed by value** (the originals used `/tmp` files that silently degraded) |
| `isolation` | | `true` runs each hunter in its own git worktree (only if hunters mutate in parallel — they shouldn't; hunters are read-only) |

## The lens catalogue (the part worth keeping)

Measured, not assumed: 4 broad lenses found 8 findings; 8 narrow lenses over the *same* code
found 41. Narrow wins by a lot. These earned their place across the rounds — copy the ones
that fit your surface into `args.lenses`:

- **Deliberate breakage / mutation** — mutate every branch of the implementation on purpose;
  a mutation that SURVIVES the suite is a gap. Highest-yield lens, every round it ran.
- **Differential execution** — push identical input through every copy of a twin and diff the
  verdicts by EXECUTING both, not reading them.
- **Attacker** — given a concrete prize, work backwards to a call chain that reaches it.
- **Coverage critic** — diff the registry against the corpus both directions; find what nothing
  asserts (self-skipping tests, empty fixture blocks, unread fixture fields).
- **Composition** — the highest-value lens. Every guard asks "can this ONE call escape?"; none
  asks "can these TWO escape together?". Hunt two shapes directly: *write-then-interpret* (X
  writes bytes Y later reads as config/code/argv/policy) and *widen-then-use* (X mutates state
  Y's guard consults). Produced a critical in four consecutive rounds.
- **Plane sweep** — the same bytes cross call / event / HTTP planes. Closing one while a sibling
  carries the same payload is the central failure mode. Assume any newly found plane defaults open.

## What the schema forces (and why)

The original family had three divergent close schemas and two hunt schemas. This reconciles
them into one set whose fields are anti-cheat by design:

- Hunters must fill `evidence` (a command **and its output**) and `triedAndClean` (a clean
  surface still shows its work).
- Closers must fill `mutationProof` (broke it → named test red → restored) and
  `weakenedAnyCase` (verified programmatically against `git show HEAD:<path>`, not asserted).
- The verifier is independent and must fill `stillDecoration` (guards deletable with suites
  still green), `catchesOriginalDefect` (re-introduce the bug, watch it go red), and
  `newGuardFails` (register a dummy entry, confirm a forcing function rejects it).

## Example — a path-containment round in this repo

```js
Workflow({ name: 'adversarial-round', args: {
  round: 9,
  baseline: 'green at HEAD; go test -count=1 clean, apps/desktop + renderer vitest green.',
  surface: 'The path-containment predicate ships in three copies: services/hub/cmd/brain/fsguard.go (the copy that actually answers under DELEGATE_CATALOG_TO_BRAIN), services/hub/internal/bus/policy.go, apps/desktop/src/main/lib/pathConfinement.ts.',
  threatModel: 'PRIZE: read ~/.config/workspacer/remote-token (promotes a bus connection to TRUSTED -> /plugins/install -> arbitrary commands) or overwrite config.yaml (feeds the electron-updater URL).',
  mapPrompt: 'Inventory every bus capability and HTTP route touching the filesystem; for each, the guard it passes through and whether a derived path escapes it.',
  testCommands: 'cd services/hub && go build ./... && go vet ./... && go test ./... 2>&1 | tail -12 ; cd apps/desktop && npx prettier --check "src/**/*.ts" && npm run typecheck && npx vitest run 2>&1 | tail -6 ; cd apps/desktop/src/renderer && npx vitest run 2>&1 | tail -6',
  knownGaps: [],  // fill from the prior round's return value
  lenses: [
    { key: 'mut-brain', task: 'Mutate every branch of fsguard.go (canonicalizePath, canonicalRoot, isWithin, pathWithinRootsCanonical, pathIsSecretCanonical, assertPathAllowed) and the derived-path guards in library.go/stores.go. Record KILLED/SURVIVED per mutation against the corpus loader then the whole package. Every SURVIVED is a gap.' },
    { key: 'mut-bus', task: 'Same, for internal/bus/policy.go and the conn.authorize wiring in bus.go plus internal/plugin manager expandScope.' },
    { key: 'mut-ts', task: 'Same, for pathConfinement.ts and the derived-path guards in libraryService.ts/sessionService.ts.' },
    { key: 'mut-loaders', task: 'Attack the TEST code, not the impl: can a loader silently skip a case, run zero cases for a group, swallow a sandbox-setup failure, or pass on an empty fixture block? Delete/neuter an assertion and check something fails.' },
    { key: 'derived-go', task: 'Derived-path sweep, Go: a guarded parameter then an UNGUARDED Join/ReadFile/Remove/Write/spawn-cwd on a path derived from it, across every cmd/brain handler.' },
    { key: 'derived-ts', task: 'Same sweep in apps/desktop/src/main services.' },
    { key: 'twins', task: 'Any logic reimplemented per stack that decides a path/filename/id/root-set/ordering. Build a differential harness; prove divergence by EXECUTING both copies on identical input.' },
    { key: 'compose', task: 'Two individually-confined capabilities that together escape. Hunt write-then-interpret and widen-then-use directly. Any caller string reaching exec/spawn/fetch/URL is in scope even with no path involved.' },
  ],
}})
```
