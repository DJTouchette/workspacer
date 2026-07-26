# work{spacer} landing page

Four self-contained pages with all styles inline, so you can open any one
straight in a browser with no build step.

- `index.html` is the marketing page.
- `docs.html` is the detailed user docs (sidebar nav, one section per topic).
- `build.html` is the build & internals page (architecture, providers, plugins, MCP).
- `build-plugin.html` is the "build a plugin" page (linked from the nav).

## Analytics

`analytics.js` is the PostHog snippet, loaded from the `<head>` of all four
pages. It is the one exception to "everything inline" — the project key and
config live in a single file rather than four copies that drift. A new page
MUST add `<script src="analytics.js"></script>` before `</head>` or it will be
invisible in PostHog. The key in there is a PostHog *project* API key, public
by design.

## Look

Everforest ground (`#1b2023`) with the sapling accent (`#a7c080`). All four
pages share one `:root` token block — if you change it, change it in all four.
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

## Logos

The marketing page inlines the real Claude, Codex (OpenAI), and OpenCode marks
as SVG, in the harness grid and again as the floating glass tiles in the hero.
The Claude mark keeps its brand clay color; the other two tint with
`currentColor`. Pi has no mark and uses a mono `π`.

## Screenshots

`.frame` holds real captures from `shots/` (webp, staged from live sessions).
To refresh one, restage the shot and drop the new capture into the matching
`shots/` file. The old fake terminal chrome (mac traffic lights) is gone — it
was a category error around captures of an Electron GUI. The hero shot gets a
shadow and a 2.5° `rotateX` that eases flat on hover; everything else is a flat
frame with an optional `.frame-cap` caption.

Cards pin their shot to the bottom with `margin-top: auto`, so a row of cards
with uneven copy still lines its screenshots up. Keep that if you add a card.

## Copy

Written in a relaxed voice, and deliberately short — the page is a poster, not
a datasheet. Six sections: the babysitting problem, the harnesses, panes,
supervise + remote, architecture, and open source. Exhaustive feature lists
belong in `docs.html`, not here; the previous version crammed nine features
into single sentences.

Feature claims map to things that ship today: the agent providers (Claude Code,
Codex, OpenCode, and Pi in beta), ambient awareness and the Triage Inbox, the
GUI agent pane, the review pane, the pane types, UI modes (fleet/focus), the
remote clients (the `/m` mobile PWA, `/remote`, and the full app at `/app`),
plugins, the MCP facade, and the desktop + claudemon + hub split. Push
notifications are deliberately not promised until they're reliable.

The proof band under the hero (harnesses / themes / plugins / licence) and the
version strings in the footer and the open-source tile are **hand-written** and
go stale. Check them against `apps/desktop/package.json`, `themes.ts`, and the
plugin catalog when you touch the page.

## Download buttons and the star pill

Three buttons (`dl-btn-nav`, `dl-btn-hero`, `dl-btn`) share one UA-detection
pass and one GitHub Releases fetch; `#dl-alts` gets every platform's link so a
wrong detection strands nobody. All of it degrades to the releases page.

The nav pill shows the live star count, but only above a `FLOOR` (10) — below
that the number reads as a liability rather than proof, and the plain "GitHub"
label is the better pill. It also stays "GitHub" if the API is rate-limited.

## Docs

`docs.html` is generated from per-section drafts that were each grounded against
the real source, then swept for voice and rendered to a single static page.
Sections: overview, getting started, running agents, the agent pane, pane types,
layout and navigation, attention and notifications, remote and multi-client,
extending (plugins / mcp / tui), and configuration. Architecture internals moved
to `build.html`.
