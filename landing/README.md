# work{spacer} landing page

Static pages with no build step. Page-specific styles are inline; shared
enquiry styling and behavior live in `contact.css` and `contact.js`.

- `index.html` is the marketing page.
- `enterprise.html` is the professional-use page with a guided pilot offer.
- `docs.html` is the detailed user docs (sidebar nav, one section per topic).
- `build.html` is the build & internals page (architecture, providers, plugins, MCP).
- `build-plugin.html` is the "build a plugin" page (linked from the nav).

## Analytics

`analytics.js` is the PostHog snippet, loaded from the `<head>` of the landing
pages. The project key and
config live in a single file rather than copies that drift. A new page
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

## Hero

Two columns, left-aligned: copy on the left, `.pane-demo` on the right. Keep it
that way. The previous hero had converged on the standard shape for this
category — centered pill, centered headline, two CTAs, a ring of floating
provider logos, one big screenshot below — which read as a straight copy of a
much better-known page with the same headline noun. Every one of those elements
was individually fine and collectively a clone.

- `.pane-demo` is **markup, not a screenshot**: the app's tab strip, a turn's
  work log, an approval card waiting on a keypress, and the session status bar.
  It costs no image, stays sharp at any width, and re-states the headline claim
  instead of illustrating it. If you change pane chrome in the app, change it
  here too — a stale mock is worse than no mock.
- **The tabs work.** `chat` / `review` / `terminal` is a real ARIA tablist with
  roving tabindex and arrow keys, driven by the last IIFE in the page script.
  All three panels stay mounted in one grid cell and inactive ones take
  `.off` (`visibility: hidden`), so the pane never changes height when you
  click — except under 560px, where `.off` becomes `display: none` because a
  fixed height would leave a hole under the two short tabs. The panels tell one
  story on purpose: the agent edits `bus.go`, asks to run that package's tests,
  and leaves the diff staged.
- The background is **not a square grid** — that's the other shape every tool
  page has. It's an editor's ground: horizontal line rhythm plus one accent
  column ruler standing in the gutter at 48%, dropped below 980px where there
  is no gutter. `.shot-band` and `.proof` need `position: relative` or the
  absolutely-positioned `.hero-grid` paints its rules over them, screenshot
  included.
- The big screenshot lives in `.shot-band` below the fold instead, sharing a
  `--bg-elev` plinth with the proof band so the two read as one block.
- Headline is the IDE claim ("An IDE where the agents do the typing"), not
  "control plane" — that phrase is in the hub's own vocabulary (`build.html`,
  where it's accurate) and shouldn't double as the product's tagline.
- The diff in the mock uses green for the addition and *dimmed* for the
  removal. No red: the page has one loud colour, and a `--red` token would have
  to be added to all four `:root` blocks to stay in sync.
- Every `kbd` in the hero is a **real default** — `y`/`n` are
  `fleet-approve-yes`/`fleet-approve-no` in
  `apps/desktop/src/renderer/src/hooks/configDefaults.ts`, and Escape is the
  interrupt. The review panel has no key chips because the review pane has no
  bindings; only the Fleet Deck and the Inbox do. Don't invent one to fill a
  gap. Same rule for the paths and commands in there: `internal/bus/*.go` are
  real files and the hub is its own Go module, so the test command is the one
  you'd actually run from `services/hub`, not from the repo root.

## Logos

The marketing page inlines the real Claude, Codex (OpenAI), and OpenCode marks
as SVG in the harness grid. The Claude mark keeps its brand clay color; the
other two tint with `currentColor`. Pi has no mark and uses a mono `π`.

They used to appear a second time as floating glass tiles in the hero. That
went away with the hero rework below — a ring of provider logos around a
centered headline is the single most copied shape in this category, and we had
it verbatim.

## Screenshots

`.frame` holds real captures from `shots/` (webp, staged from live sessions).
To refresh one, restage the shot and drop the new capture into the matching
`shots/` file. The old fake terminal chrome (mac traffic lights) is gone — it
was a category error around captures of an Electron GUI. Every frame is flat
now, with an optional `.frame-cap` caption; the tilted hero shot went away with
the hero rework.

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
the existing example-workspace captures, with keyboard-accessible tabs and
full-size image links. Without JavaScript, all three views remain visible.

Claims are grounded in `docs/features/fleet-workflows.md`,
`apps/desktop/src/renderer/src/lib/fleetManager.ts`, and the workflow runtime/settings under
`apps/desktop/src/`. Configured workflows require the desktop host. Each
developer has their own installation; this page does not promise shared
team administration. Local coordination does not mean model requests stay on
the machine. The public workflow guide is `docs.html#fleet-workflows`.

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
