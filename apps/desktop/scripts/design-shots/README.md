# design-shots — full-app screenshot rig

One command that renders the **real** desktop renderer (the web build of it,
`dist/web`) in headless Chromium against the **real** Rust hub, with a
fabricated, fully populated world, and captures every major screen at
1600×1000 @2x (the phone PWA at 430×932 @2x).

```sh
cd apps/desktop
npm run shoot:design -- --out /tmp/shots                     # baseline
npm run shoot:design -- --out /tmp/shots-v2 --setting ui.designV2=true   # same shots, flag on
```

Use it to capture a baseline before a redesign and the identical frames after
it, with the redesign switched on behind a config flag.

## What it needs

- `node_modules` in `apps/desktop` **and** `apps/desktop/src/renderer`.
  Electron is not needed, so `npm install --ignore-scripts` is fine in both (it
  skips the `electron-rebuild` postinstall).
- `cargo` (first run builds `services/hub-rs`'s `hub_contract_fixture` example,
  ~2 min cold; cached afterwards). Set `WKS_TEST_HUB_FIXTURE_BINARY=/abs/path`
  to reuse a binary you already built.
- A Chromium Playwright can launch. The rig tries the repo's `playwright` first,
  then any global install on `NODE_PATH`; it never runs `playwright install`.

`dist/web` is rebuilt (`npm run build:renderer:web`, ~10 s) only when a
renderer source file is newer than it. `--skip-build` / `SHOT_SKIP_BUILD=1`
skips that check; `--rebuild` forces it.

## Options

| Flag / env | Meaning |
|---|---|
| `--out DIR` / `SHOT_OUT` | where PNGs go (default `apps/desktop/design-shots-out/`, gitignored) |
| `--state DIR` / `SHOT_STATE` | scratch root for the hub's state (default `<out>/.rig-state`); a fresh subdir per run, deleted after |
| `--setting key.path=value` (repeatable) / `SHOT_SETTINGS="a.b=1,c.d=true"` | deep-merged into the config document the app loads, **before** first render. Values are parsed as JSON when possible (`true`, `3`, `"x"`, `{...}`), else kept as strings |
| `--local key=value` (repeatable) | seed a `localStorage` key before the page loads |
| `--only REGEX` | only shots whose id matches, e.g. `--only '^(04\|09\|2)'` |
| `--scale N` | deviceScaleFactor (default 2; 1 is handy while iterating) |
| `--serve` | boot the stage and print the `/app` and `/m` URLs instead of shooting — open them in any browser to explore the fake world |
| `--list` | print the shot ids + descriptions |

The run prints one line per shot, writes `manifest.json` next to the PNGs,
saves a `*.FAILED.png` for a shot that threw, lists any bus methods the fake
world did not model, and exits non-zero if any shot failed.

## How it works

| File | Role |
|---|---|
| `stage.mjs` | Builds/reuses the hub + web bundle. Boots `hub_contract_fixture --mode browser` on a kernel-chosen port with `HOME`/`XDG_*` and every path flag (`--tokens-file`, `--layout-file`, `--jobs-file`, …) inside the scratch dir, serving `dist/web` at `/app/`. Attaches a fake **capability provider** over `/bus` with the host token. This is `tests/e2e/fixtures/appHub.ts` as plain `.mjs`. No claudemon, no Electron, no fixed ports — it cannot touch a live workspacer. |
| `world.mjs` | The invented world: five agents in different states (streaming with a live Bash call, plan, subagents and a workflow; blocked on a permission prompt; asking a question; a finished Codex agent with a diff; an idle OpenCode agent), git status/diffs/log, usage report, analytics, session history, Fleet Manager dispatch history, jobs, layouts, library, routing policy, a terminal screen. Everything is `/home/dev/work/…`; no real people, repos or tokens. |
| `shots.mjs` | The shot list: per shot an optional world tweak, viewport, entry URL, and the UI steps (palette commands, shortcuts, clicks). |
| `shoot-all.mjs` | CLI. For each shot: reset the hub's layout + snapshots + config to the shot's world, open a fresh browser context (clean `localStorage`), wait until the hero agent's card has painted, run the steps, settle, screenshot. |

Layout (`layout.get/set`) and jobs (`jobs.*`) are **hub-local**, so they are
seeded as files the hub reads (`layout.json`, `jobs.json`,
`jobs-history.json`) and the layout is reset between shots with `layout.set`.
Everything else is answered by the fake provider. The contract-fixture hub
exits by itself after 10 minutes; the rig recycles it after 7.

One-time notices that would cover a populated screen are pre-dismissed
(`onboardingDismissed`, `ui.commandLayerAnnounced`, and the
`wks:first-task-guidance-dismissed` localStorage key); the first-run shot turns
them back on.

### Adding a shot

Append to `SHOTS` in `shots.mjs`. Keep ids stable (`NN-slug`) so a redesign run
lines up file-for-file with the baseline. Prefer `palette(page, 'Title')` (the
command palette can open almost every pane and dialog) or a default keybinding
over CSS selectors; a redesign will rename classes but not commands.

## Gating a redesign behind a setting

### How settings flow

- **Source of truth for defaults:** `services/hub-rs/assets/config-defaults.json`.
  `npm run gen:config-defaults` regenerates two TS copies from it
  (`src/main/services/configDefaults.generated.ts`,
  `src/renderer/src/hooks/configDefaults.generated.ts`); drift tests
  (`configService.test.ts`, hub-rs `config.rs` tests) fail if they diverge.
- **Persistence:** one shared `config.yaml`
  (`~/.config/workspacer/config.yaml`). Two writers with the same semantics:
  Electron main `src/main/services/configService.ts` and the hub's
  `services/hub-rs/src/services/config.rs` (used by `/app` web clients and
  headless). Both load as `deepMerge(defaults, file)` and save as
  `deepMerge(current, partial)`; **neither prunes unknown keys** under `ui`, so a
  new `ui.*` key round-trips with no Rust or main-process change.
- **Renderer:** `ConfigContext` (`src/renderer/src/contexts/ConfigContext.tsx`)
  loads it once via `window.electronAPI.getConfig()` (IPC in Electron,
  `config.get` over the hub bus on the web). Components read it with
  `const { config, save } = useConfig()` (`hooks/useConfig.ts`). Writes go
  through `save({ ui: { ...config.ui, key: value } })`.
- **In this rig** the provider answers `config.get` itself, so `--setting`
  lands in exactly the object `useConfig()` returns, before the first render.

### Recommended: `ui.designV2` (boolean, absent = off)

1. Type it — add to `UIConfig` in `src/renderer/src/hooks/useConfig.ts`:
   ```ts
   /** Opt-in redesign (design pass). Absent/false = current UI. */
   designV2?: boolean;
   ```
   For symmetry also add `designV2?: boolean;` to `Config['ui']` in
   `src/main/services/configService.ts` and `AppConfig['ui']` in
   `src/main/shared/ipcTypes.ts` (types only; not required for it to persist).
2. **Leave it out of `config-defaults.json`** while it is experimental — the
   same pattern as `ui.commandLayerAnnounced`: absent reads as off, nobody's
   file gains a key until they flip it, and no generated file or drift test
   changes. Read it as `config.ui.designV2 === true`. (If it later becomes a
   shipped default, add `"designV2": false` to the JSON and run
   `npm run gen:config-defaults`.)
3. Read it in **one** place near the root so CSS and components can both gate
   on it — e.g. in `App.tsx` next to the existing `ui.mode` handling:
   ```ts
   const designV2 = config.ui.designV2 === true;
   useEffect(() => {
     document.documentElement.dataset.design = designV2 ? 'v2' : '';
   }, [designV2]);
   ```
   Then style with `:root[data-design='v2'] …` and branch components on
   `useConfig().config.ui.designV2`.
4. Optional user toggle: a row in
   `src/renderer/src/components/settings/AppearanceSection.tsx`, copying the
   `showComposerSend` toggle (`saveWithFeedback({ ui: { ...config.ui, designV2: v } })`).
5. Capture:
   ```sh
   npm run shoot:design -- --out /tmp/shots-v2 --setting ui.designV2=true
   ```

Any other key works the same way (`--setting ui.theme=light`,
`--setting ui.mode=focus`, …), and `--local` covers flags that live in
`localStorage` instead of config.

## Known limits

- Analytics moved to a plugin, so the palette's "Analytics" shows an install
  hint; usage is covered by the Overview shots instead.
- The spawn dialog is full-screen by design (opaque backdrop).
- The web build reports `platform: 'web'`, so Electron-only chrome (native
  title bar overlay, in-app updater) is not in frame.
