# Adversarial hardening: method and findings

A record of the hardening pass that runs from `ea34740` to `cf2cf7e` (sixteen
commits), written so somebody else — human or agent — can pick it up without
re-deriving how it works or re-finding what it found.

Two things live here: **the method**, which is reusable and is most of the value,
and **the findings**, which are mostly closed but whose *shapes* keep recurring.

---

## 1. The problem this addresses

This is a polyglot monorepo where the same concept is deliberately reimplemented
per stack. One containment predicate ships three times (Go brain, Go bus,
TypeScript desktop). `config.yaml` has two writers. The filename slug has three
copies. `~/.claude/projects`' path encoder has three. The config-dir rule has four.

Under the default `DELEGATE_CATALOG_TO_BRAIN`, the Go brain answers most bus
capabilities — so **the copy people read is not the copy that runs**. A guard
added in the natural place (the desktop TypeScript, where the UI is) lands on a
dead path, and nothing says so.

That is the disease. Everything below is diagnosis and treatment for it.

---

## 2. The method

### 2.1 The loop

Each round is: **hunt (read-only) → close (edits + regression tests) → verify
(adversarial)**. The three roles are separate agents on purpose — a hunter that
can edit starts fixing instead of finding, and a closer that grades itself passes
itself.

Rounds are one workflow invocation each, so results bank before the next starts.
An interruption costs one round, not a pass. This matters: two rounds were lost
to environment failures before that structure was adopted.

### 2.2 Narrow lenses beat broad ones, by a lot

Measured, not assumed. Four broad lenses over the codebase found **8** findings.
Eight narrow lenses over the *same* codebase found **41**. A clean sweep result
says as much about the sweep as the code.

Lenses that earned their place:

| Lens | What it does |
|---|---|
| **Deliberate breakage** | Mutate each implementation on purpose; a mutation that SURVIVES is a gap. Consistently the highest-yield lens — it beat the attacker lens in every round it ran. |
| **Differential execution** | Push identical input through every copy of a twin and diff the verdicts. Proof must be an execution, not a reading. |
| **Attacker** | Given a concrete prize (read `remote-token`, overwrite `config.yaml`), work backwards. |
| **Coverage critic** | Diff the registry against the corpus, both directions. Find what nothing asserts. |
| **Composition** | See §2.4. |

### 2.3 The standard: a guard that kills no mutant does not exist

This is the single most important rule here, and it was learned the hard way —
**more than half of all findings were guards that existed and asserted nothing.**

Every fix must be mutation-verified: break the thing, watch the *named* test fail,
restore. Not "tests pass" — tests passed the entire time.

Things found passing while asserting nothing:

- a sweep whose eight subtests all skipped, inside a green suite
- a corpus where a one-character typo in a `${TOKEN}` silently defanged a deny case
  in all three loaders, making all 64 deny cases individually unfalsifiable
- `go test` printing `ok (cached)` on a tree where `-count=1` FAILED
- coverage floors set at 26% of their true value
- a delivery test that derived its expectation **from the row it was checking**
- a guard whose evidence was `strings.Contains(reason, "no params")` — satisfied by
  writing that phrase
- a "closure record" satisfied by naming any symbol that existed anywhere

### 2.4 Composition: the highest-value lens

Every guard in the system asks *"can this ONE call escape?"* None asked *"can
these TWO calls escape together?"* That produced a critical in **four consecutive
rounds**.

Two generalised shapes — hunt these directly rather than waiting to stumble on one:

- **write-then-interpret** — X writes bytes to a location Y later reads as
  *config, code, argv or policy* rather than as data. Both calls confined, no path
  escapes any root. The boundary crossed is **data becomes instruction**.
- **widen-then-use** — X mutates state (a grant, a root set, a permission mode, an
  approval gate, a session) that Y's guard *consults*. X may write it, Y may trust
  it, neither is wrong alone.

**Method that works:** build an inventory of every capability with four columns —
`reads` / `writes` / `executes` / `changesState` — then work *backwards from
sinks*. For each capability that executes something, ask what decides its argv and
who can write there. For each that returns bytes, ask what decides its location and
who can move it. That is a finite search over a table.

**Can the composition record be derived mechanically? No — measured.** 0.95%
precision over the inventory, hits carried by English filler, only 6% of `executes`
cells naming an object another capability could write. A forcing function
(every new capability must be *considered* for composition, with a checkable
record) is the honest answer. Don't re-litigate this without new evidence.

### 2.5 Planes

There are **three** planes, and each needed closing separately:

| Plane | Verb | Closed in |
|---|---|---|
| Call | `call` | rounds 1–6 |
| Event | `subscribe` / `publish` | round 7 |
| HTTP | routes | round 8 |

The event plane and the HTTP plane each **defaulted open** and had never been
looked at. Closing one plane while a sibling plane carries the same bytes is the
central failure mode — see `plugin.loaded` in §3.

If a fourth plane is found, assume it defaults open until shown otherwise.

### 2.6 Verification is not optional, and verifiers find real things

Every verifier in this effort found real decoration **inside the fixes meant to
remove it**. Twice a critical was re-opened by the patch that closed it.

Make the verifier's schema demand it. A field named `stillDecoration` or
`guardsThatDoNot`, described as *"guards deletable with every suite still green"*,
gets honest answers. So does telling the agent plainly that an honest "this one is
still theatre" is worth more than a clean bill of health.

Also require `weakenedAnyCase` on every closer, and require it be verified
**programmatically** (diff every fixture's case-name → expect map against
`git show HEAD:<path>`), not asserted. Closers otherwise weaken a case to go green
and describe it as a fix.

### 2.7 Operational notes that cost real time

- **`/tmp` is a per-user-quota tmpfs.** A full one does not present as a test
  failure — it presents as the *shell* being unable to `exec`, so every command
  returns nonzero with no output. This killed a 27-finding closer outright. The
  vitest suite leaked ~92 mkdtemp dirs per run; the claudemon suite leaked a loose
  sqlite file per `test_state()` (1034 had accumulated). Both fixed; keep them
  fixed.
- **Don't clone the repo per agent.** Eight agents × `git archive` is 200MB in
  `/tmp`. Prefer in-place mutate-and-restore.
- **Watchdogs need a realistic idle threshold.** A mutation sweep runs 65 mutants
  inside one command and writes nothing for 10+ minutes. Five minutes false-alarms;
  20 is right. Watch `/tmp` usage too.
- Workflow scripts are plain JS: a backtick inside a prompt template literal breaks
  the parse. This cost three retries.

---

## 3. What was found

~250 findings across sixteen commits, 14 criticals. Roughly half are genuine
security bugs and half are plain correctness bugs. Full detail is in the commit
messages, which are written to be read — each names the defect, why it survived,
and what now catches it. Below are the ones whose *shape* recurs.

### 3.1 The criticals, by shape

**Derived paths.** A guarded parameter, then an unguarded operation on a path
*derived* from it. `library.list` guarded `cwd` correctly, then `os.ReadFile`'d
every derived `<dir>/*.md` — so a symlink you were allowed to write returned
`remote-token`, which promotes a connection to trusted, which unlocks
`/plugins/install`. `fs.read` of the identical symlink was refused, which is what
proved containment was real and specifically bypassed. Same shape in
`library.remove`, `layouts.list`, `sessions.list`, `library.save`.

**Data becomes instruction.** `fs.write` a `.git/config` with `core.fsmonitor`,
then `fs.listEntries` — git executes it. `fs.write` a `.claude/settings.json`, then
`agents.spawn` runs its hooks. `fs.write` a `.opencode/plugin/*.js`, then
`providers.listModels` executes it. `fs.write` a ripgrep `.ignore`, then
`search.project` returns `.git/config`. In every case both calls were correctly
confined.

**The dangerous value is not a path.** `config.save` writes
`agents.binaries.<provider>` — argv[0] of every spawned agent. `terminal.shell` is
argv[0] of the next terminal the *local user* opens. `terminals.create` took an
unvalidated `shell`. `agents.spawn`'s `mcpItemIds` names library items that become
MCP server commands. No path analysis finds any of these.

**Same bytes, different plane.** `plugin.loaded` (the full manifest: install argv,
server command, every declared fs scope) was classified `TopicHostOnly` and refused
to every scoped tier — while unguarded `GET /plugins` handed the identical payload
to an anonymous caller. `plugin.settings.changed`'s recorded justification was
*"the equivalent READ — /plugins/settings — is guard()ed to the host token"*; that
was true, and `/plugins/ui/` walked around it by inlining the same values into an
anonymously readable HTML document.

**Guards that filter too late.** `broker.Publish` matched and *enqueued*
`pty.bytes.<id>` into a refused connection's channel; only the writer goroutine
dropped it. Enqueue armed the overflow bookkeeping, so `pty.desync` then leaked the
sessionId of a stream the connection had just been refused. **A guard that runs
after the queue leaks existence through the queue.**

**Lexical where it should be canonical.** This one is *remarkably* durable — it
appears in the first commit of the series and again in the sixteenth.
`filepath.Abs`/`path.resolve` collapse `link/..` textually *before* reading any
symlink, so the guard validated one file and the handler opened another.
`sessionService` used `path.resolve` + `startsWith`. `ValidateUIDir` checked the
`ui` *string* for `..` and never resolved it — **a symlink is not a string**.
A string test reads like a path test and passes every case an author thinks to
write.

### 3.2 The correctness half

Worth remembering these would have reached users sooner than the security ones:

- `localeCompare` throws on a non-string `createdAt`; the catch-all returns `[]` —
  **every layout and every saved session vanishes** on one provider while the other
  lists them fine.
- `resolveSessionFilename` silently overwrote a *different* session's file on the
  second collision.
- Go's `strings.ToLower` folds `U+0130` to `i`; JS's `toLowerCase` yields `i` +
  `U+0307`, which becomes `-`. So the brain wrote `aib.yaml` where the desktop wrote
  `ai-b.yaml` — an item saved under delegation is invisible to the other provider
  and undeletable, because delete re-slugs a name that was never written.
- `search.project` clipped at 300 UTF-16 units on one side and 300 code points on
  the other: half the text, plus a lone surrogate on the wire.

---

## 4. What exists now

| Artifact | What it holds |
|---|---|
| `contracts/*.json` (11 fixtures) | Golden cases for logic implemented more than once. `path-containment-cases.json` alone is 137 cases across 14 blocks. Each is loaded by a test in **every** owning language. |
| `internal/capspec` | Classification registry: every capability, and every dangerous *parameter*, is path-scoped / excused-with-a-checked-reason / refused. ~6k lines. |
| `internal/capspec/composition.go` | The composition record. Exemptions carry a **Bearing** — machine-checked proof, via a real call chain, that the named guard reaches one of the pair's halves. |
| `internal/capspec/eventtopics.go` | Event-topic registry. Default **closed**; a new topic fails until classified. |
| `internal/capspec/httproutes.go` | HTTP route registry. Same forcing function. |
| `internal/sweepguard` (+ `tests/support/sweepTally.ts`) | A sweep must prove it ran: non-zero executed cases, denies counted separately from allows. |
| `internal/extinput` | Pins out-of-module files into the `go test` cache key. See the warning below. |

### 4.1 Traps in the machinery itself

- **`extinput` works by a subtlety.** `search.InDir` is a purely *lexical* prefix
  test, and `computeTestInputsID` only cleans a *relative* recorded name. So an
  absolute path that still carries its `..` passes the string test while the kernel
  resolves the real file. `Path()` is **string-concatenated, not `filepath.Join`'d**,
  because Join would clean away the one thing making it work. Do not "tidy" this.
  The obvious alternatives were both measured and both fail: the env-var-name trick
  is a no-op, and a plain `os.ReadFile` of an out-of-module absolute path is not
  re-hashed either.
- **`-count=1` is a second belt** in the Makefile and both CI Go jobs, with a test
  that parses both files so it cannot quietly fall out.
- **Registration has FIVE doors**: desktop `cat(...)`, desktop
  `registerCapability(...)`, the brain's registry, `cmd/hub`'s `RegisterLocal` /
  `RegisterLocalIdent`, and HTTP `AddRoute`. Completeness checks have missed the
  third and fourth. Enumerate all five.

---

## 5. Known open

None is a live escape; each is one lie away from being one. All are disclosed in
the commit that shipped them.

1. `TestLoopbackConfinedRoutesActuallyHaveTheirConfinement` regex-matches the layer
   name rather than exercising the confinement — and was written *specifically*
   because its first version failed to fail.
2. The HTTP route sweep scans a hardcoded three-file list on the Go side, which is
   the exact failure its author wrote it to prevent on the Rust side.
3. `handlerRefuses401` accepts a **comment** containing `StatusUnauthorized` as
   proof of a credential check.
4. The cross-site-origin test is named for every route and iterates six of 35.
5. The event-topic answer key is hand-written; a coordinated three-place edit still
   passes. A third independent check derived from where the code publishes covers
   8 of 9 host-only rows.
6. `BearsOnGrantedRoots` is the weakest Bearing kind and could not be made
   method-specific.
7. Three inert composition claims remain prose (`claude.signal`,
   `claude.handoffBrief`, `claude.handoffAgentBrief`) — pinned by name, and each
   must say *NOTHING HERE IS MACHINE-CHECKED* in its own reason.
8. Windows branches are untested (no Windows CI job). Case-insensitive filesystems
   are a recorded known gap.

Items 1–4 are one species: **a guard whose evidence is textual rather than
behavioural.** That is the same species as §2.3's list, and it is the most reliable
place to look next.

---

## 6. If you pick this up

- Read the commit messages from `ea34740` forward. They are the primary record;
  this file is the index.
- Run a round the way §2.1 describes. Do not let one agent hunt, fix and grade.
- Demand execution as evidence. Inspection-only findings are wrong often enough
  that the rule pays for itself.
- Expect your own fix to contain decoration. Every previous one did.
- The counts do not converge. Round tallies were 34, 41, 26, 36, 27, 32 — the
  falling stretches were noise, and the count went *up* on the cleanest instruments
  we had. Do not read a low round as "nearly done"; read severity and whether the
  previous round's fixes held.
