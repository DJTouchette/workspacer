# work{spacer} landing page

Static pages with no build step. Page-specific styles are inline; shared
enquiry styling and behavior live in `contact.css` and `contact.js`.

- `index.html` is the marketing page. It leads with the native app
  (`apps/native`); the Electron app is described as deprecated.
- `enterprise.html` is the professional-use page with a guided pilot offer.
- `docs.html` is the user docs for the native app and the hub (sidebar nav, one
  section per topic). `docs-electron.html` is the previous docs page, kept
  unchanged under a deprecation banner as the Electron app's legacy docs.
- `build.html` is the build & internals page (building from source, the Rust
  hub and session engine, providers, plugins, MCP, federation, and a legacy
  section for the Electron app).
- `build-client.html` is the "build a client" page (the session engine's HTTP
  and SSE API).
- `build-plugin.html` is the "build a plugin" page (linked from the nav).
  `build-plugin.md` is its plain-text twin for LLMs and is **generated**: edit
  the HTML, then run `make landing-plugin-md`. The generator refuses markup it
  cannot convert instead of dropping it.

Every page states the same position: the native app is the product, and the
Electron app is deprecated (it still runs, its installers stay on the releases
page, new work goes native). If that changes, it changes on all of them.

## Analytics

`analytics.js` is the PostHog snippet, loaded from the `<head>` of the landing
pages. The project key and
config live in a single file rather than copies that drift. A new page
MUST add `<script src="analytics.js"></script>` before `</head>` or it will be
invisible in PostHog. The key in there is a PostHog *project* API key, public
by design.

## Look

Everforest ground (`#1b2023`) with the sapling accent (`#a7c080`). `index.html`
and `enterprise.html` share one `:root` token block; the docs and build pages
carry the same palette under their own variable names. If you change a colour,
change it on every page.
Three rules carry the design, and each one was a deliberate fix:

- **Sans for prose, mono for code.** The pages used to be monospace throughout,
  which read as a README and made inline `<code>` invisible. Body is now Inter;
  `--font-mono` is reserved for the wordmark, eyebrows, `.k` keys, `<code>`,
  `<pre>`, and terminal bodies. Both faces are *loaded* from Google Fonts —
  before, the pages named JetBrains Mono without ever fetching it, so nearly
  everyone saw a fallback.
- **Green is an accent, not a body colour.** It used to be on every heading,
  link, bullet and prompt, which made it wallpaper. The text ramp
  (`--ink` → `--ink-dim` → `--ink-faint`) carries the page; green marks one
  phrase per heading, the primary button, and active state.
- **Big and light, not small and bold.** Display type is weight 500 at
  `-0.035em`. Headings that were 27–36px/700 are now 31–72px/500–600.

There are no scanlines any more — a fixed 0.3-opacity overlay dimmed every
screenshot on the page. The hero has a masked grid instead, confined to itself.

## Hero

Two columns, left-aligned: copy on the left, `.pane-demo` on the right. Keep it
that way. An earlier hero had converged on the standard shape for this
category (centered pill, centered headline, two CTAs, a ring of floating
provider logos, one big screenshot below), which read as a copy of a much
better-known page.

- The headline is the three things the page is about, in one line: a *small
  app* (footprint, simplicity) where your agents can *call each other*
  (cross-provider spawn). An older headline sold the Fleet Manager and the
  brief; those belong to the Electron app, which the page now calls deprecated.
- `.pane-demo` is **markup, not a screenshot**, and it is an illustration of
  one spawn rather than a copy of the app's chrome (the native app has no tab
  strip). The three panels tell one story: you ask Claude for something, it
  spawns a Codex child and ends its turn, and the child's result comes back as
  a wake. The third tab quotes `apps/desktop/assets/skills/spawn-agent/SKILL.md`
  **verbatim**. If the skill's wording changes, change the quotes.
- **The tabs work.** It is a real ARIA tablist with roving tabindex and arrow
  keys, driven by an IIFE in the page script. All three panels stay mounted in
  one grid cell and inactive ones take `.off` (`visibility: hidden`), so the
  pane never changes height when you click, except under 560px, where `.off`
  becomes `display: none`.
- The approval card in the mock shows the app's own three buttons (Allow once,
  Deny, Deny with note…) and **no key chips**, because the native approval card
  has none. Don't invent one to fill a gap. The `spawn` line uses real
  `spawn_agent` parameter names (`provider`, `worktree`, `trackTask`).
- The background is **not a square grid**. It's an editor's ground: horizontal
  line rhythm plus one accent column ruler standing in the gutter at 48%,
  dropped below 980px where there is no gutter. `.shot-band` and `.proof` need
  `position: relative` or the absolutely-positioned `.hero-grid` paints its
  rules over them, screenshot included.
- The big screenshot lives in `.shot-band` below the fold, sharing a
  `--bg-elev` plinth with the proof band so the two read as one block.
- The diff in the mock uses green for the addition and *dimmed* for the
  removal. No red: the page has one loud colour, and a `--red` token would have
  to be added to every page's `:root` block to stay in sync.

## Logos

The page inlines the real Claude, Codex (OpenAI), OpenCode and GitHub Copilot
marks as SVG in the harness grid. The Claude mark keeps its brand clay color;
the others tint with `currentColor`. Pi is not in the grid: it is not offered,
and the caveat line under the grid says why.

Each tag says what that provider can do **from the native app**: Claude Code
and Codex can be started from New Agent and spawned as children; OpenCode and
Copilot only arrive as children (`apps/native/src/navigation.rs` has two
`Provider` variants; `spawn_agent` accepts four). Keep the tags honest if
either list changes.

## Screenshots

Every `shots/native-*.webp` is a capture of the real `wks-native` window
showing the fixture hub (`native-harness serve`, the same fixture `--demo` runs
in-process). No agent, credential or model is involved. Restage all of them
with one command:

```sh
make landing-shots        # builds the two binaries, then runs landing/capture-shots.sh
```

It needs Linux with `Xvfb`, `openbox`, `xdotool`, `cwebp` and a software Vulkan
driver (Mesa lavapipe). Captures are taken at 2x; the `width`/`height`
attributes in `index.html` are the logical window size, so keep the two in
step. The script's header explains the two fixtures and why the scroll counts
are tied to the window height.

Three sizes, for three jobs: 1280×760 for the two full-width frames, 900×600
for the feature rows, 820×550 for the three-up gallery. A capture of a whole
window shown at a third of the page is a grey smudge, which is why the
"simple" section is rows (`.feat`) rather than a three-up of cards, and why the
gallery uses the smallest window the content still fits in.

The script also takes the captures the docs and enterprise pages use
(History, Projects, Settings, the Light theme). It drives the window with real
key presses, and a key chord occasionally lands before a view is ready, so
**look at what it produced** before committing: a capture that shows the chat
where a view should be is a miss, and running it again fixes it.

`mobile-fleet.webp` and `mobile-chat.webp` are the phone client, staged from an
earlier fabricated fleet. `sidebar-recent.webp` is the one Electron capture
left, used by `docs-electron.html`.

Every frame is flat, with an optional `.frame-cap` caption. Fake terminal
chrome (traffic lights) around a GUI capture is a category error.

## Copy

Written in a relaxed voice, and deliberately short: the page is a poster, not
a datasheet. Three ideas, in this order, and everything else is support:

1. **Small** (`#small`): one binary, no webview, bounded memory by design.
2. **Cross-provider spawn** (`#spawn`): the `spawn-agent` skill.
3. **Simple** (`#simple`): a list and a conversation.

Then remote (`#remote`), the honest list (`#status`), open source (`#open`)
and how to get it (`#get`). Exhaustive feature lists belong in the docs.

Claims map to things the native app ships today, and the sources are:

- **Footprint.** The size bars and the proof band compare *download size on
  Linux x64*: the native figure is `make package-native-linux` on this tree,
  the Electron figure is the AppImage attached to the latest release. Both are
  **hand-written** and go stale; re-measure when you touch the page. Last
  measured at 0.169.0: 33.6 MB against 168.9 MB (the bar is 20% wide), with a
  107 MB stripped binary inside the archive, which the caption admits. There are
  deliberately no memory or CPU numbers: the only ones recorded
  (`apps/native/VALIDATION.md`) were taken on a software renderer. The
  "bounded" tile quotes `apps/native/README.md` → Performance boundaries.
- **Spawn.** `apps/desktop/assets/skills/spawn-agent/SKILL.md`, the
  `spawn_agent` tool in `services/hub-rs/assets/mcp-tools.json`, and
  `.rivet/context/domains/agent-spawn.md` (per-session skill delivery, nothing
  written into the project, wakes route to the direct parent).
- **Simple, remote.** `apps/native/README.md`: New Agent, approvals and
  questions, Changes, the terminal, History, Jobs, Settings → Remote.
- **What it isn't.** The native app has no Fleet Manager launcher, board,
  tiled panes, embedded browser or plugin panes, and in `--local` mode quitting
  stops the agents it owns. The page says so. Don't soften it; a page that
  overclaims the boring parts doesn't get believed about the interesting ones.

The Electron app is described as **deprecated** (still runs, installers stay on
the releases page, new work goes native), and the honest list links its legacy
docs.

Push notifications are deliberately not promised.

## Enterprise enquiries

`enterprise.html` offers a scoped, paid setup and onboarding pilot with an
inline form (`#pilot-form`). The other pages offer an enquiry dialog through
`data-contact-open` links; their ordinary href points to `enterprise.html#pilot`
if JavaScript or the dialog API is unavailable. The enterprise footer also
offers the general enquiry dialog.

All forms POST to `https://formspree.io/f/xbgjzbna`. `contact.js` initializes
the pinned `@formspree/ajax` 1.1.5 CDN library for inline feedback, a disabled
submit button while sending, and draft preservation on errors. A basic HTML
POST remains available if the SDK cannot load. No backend or API key is needed.
The shared dialog markup lives in `contact.js`; keep its fields in sync with
the enterprise HTML form. Styles are scoped by `.wks-contact`.

Name, email and message are required; company is optional. The hidden
`subject`, `source` and `_gotcha` fields supply context and the honeypot.
Formspree's dashboard controls the notification recipient; it should be
`thetouchstonedev@gmail.com`. The direct email link remains a fallback.
Test browser responses with intercepted requests rather than creating live
enquiries. Frontend checks do not confirm inbox delivery or dashboard settings.

Every HTML page includes `contact.css`, then deferred `contact.js` and the
pinned SDK in that order. Keep these assets alongside the pages when deploying.
Reference: [Formspree's vanilla JS guide](https://help.formspree.io/articles/building-your-form/submit-forms-with-javascript-ajax/).

The hero's pilot link goes to the separate lower `#pilot` section. Keep that
destination below the product tour: placing it beside the hero made the link
produce no visible movement on tall desktop windows. The screenshot tour uses
native captures (delegation, Changes, History), with keyboard-accessible tabs
and full-size image links. Without JavaScript, all three views remain visible.

The page describes what the native app does for professional use, grounded in
`apps/native/README.md` and the `spawn-agent` skill. The Fleet Manager and
configured workflows get one plain paragraph, and it is worded carefully for a
reason: the workflow runtime is in the Rust hub (`services/hub-rs`:
`workflows.rs`, `workflow_runtime.rs`, the `manager` flag in `spawn_plan.rs`),
so it runs wherever the hub does, but the only launcher for a manager and the
only workflow editor are in the Electron app. The native app, the phone client,
the TUI and the CLI cannot start one. Don't shorten that to "requires Electron"
or to "native does it"; neither is true. Each developer has their own
installation; this page does not promise shared team administration. Local
coordination does not mean model requests stay on the machine. The workflow
guide is `docs-electron.html#fleet-workflows`.

The pilot offer's own wording ("a workflow matched to a real task") predates
this and was left alone; what the pilot installs is the owner's call.

## Download buttons and the star pill

Three buttons (`dl-btn-nav`, `dl-btn-hero`, `dl-btn`) share one UA-detection
pass and one GitHub Releases fetch. They only ever pick a **native** package:
every asset pattern requires `native` in the name, so an Electron installer can
never match. When the latest release has a native package for the visitor's OS
the buttons become a direct download; when it has none (macOS always, any OS
if the release predates native packaging) the nav and hero buttons point at
`#get`, whose rows say how to build it, and the CTA button stays the repo link.
`#dl-alts` lists the native links that exist and then the releases page under
the label "deprecated electron installers".

The three `#get` rows are hand-written statements about what
`.github/workflows/release.yml` and `apps/native/scripts/package-*.{sh,mjs}`
produce. As of this writing the release workflow attaches the Windows
installer only; the Linux archive is a CI artifact and is **not** attached to
releases, so the Linux row tells people to build it. Update the row (and the
asset pattern already waiting for it in the script) when that changes.

The nav pill shows the live star count, but only above a `FLOOR` (10). Below
that the number reads as a liability rather than proof, and the plain "GitHub"
label is the better pill. It also stays "GitHub" if the API is rate-limited.

## Docs

`docs.html` documents the native app first (overview, getting started, running
agents, agents that spawn agents, the conversation, views, keys, attention,
remote, settings), then the hub-level topics that apply to any client (jobs,
limit-aware routing, MCP and plugins, configuration). It is written from
`apps/native/README.md` and the shipped skills; when the README changes, this
page is what goes stale.

Two things are load-bearing:

- **The jobs section is pinned by a test.**
  `services/hub-rs/src/services/jobs/docs_tests.rs` reads this file, runs every
  `<pre data-job-example>` block through the real validator, and checks that
  the spec's field and kind names are still documented. After touching the
  section, run `cargo test --locked --lib docs_tests` in `services/hub-rs`.
- **Old anchors still resolve.** Sections that moved to `docs-electron.html`
  (`#fleet-manager`, `#fleet-workflows`, `#briefs`, `#agent-pane`, `#panes`,
  `#layout-and-navigation`, `#launch-integrations`) are short stubs under
  `#legacy` that link to the same anchor on the legacy page, because the app,
  plugin READMEs and old links point at them. Don't delete a stub without
  checking who links to it.

`docs-electron.html` is frozen: it is not updated, and where it says "the
desktop app" it means the Electron app. Architecture internals live in
`build.html`.
