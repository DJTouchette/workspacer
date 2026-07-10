# work{spacer} landing page

Three self-contained pages with all styles inline, so you can open any one
straight in a browser with no build step.

- `index.html` is the marketing page.
- `docs.html` is the detailed user docs (sidebar nav, one section per topic).
- `build.html` is the build & internals page (architecture, providers, plugins, MCP).

## Look

The everforest brand palette: forest ground (`#1e2326`) with the sapling accent
(`#a7c080`), amber and a soft cyan for highlights, light scanlines, and a
file-tree style feature list. Monospace throughout. The `work{spacer}` wordmark
and the `{ ▮ }` mark show up in the nav and footer. Speaks to the CLI crowd.

## Logos

The marketing page has a "works with your agents" section with the real Claude,
Codex (OpenAI), and OpenCode marks inlined as SVG. The Claude mark keeps its
brand clay color; the other two tint with `currentColor`.

## Screenshots

The framed boxes labelled things like `[ hero screenshot ]` and
`[ review / diff pane ]` are placeholders. Drop a real capture into each slot
when you have one. The hero shot is the big terminal window near the top, and
there are smaller slots in the feature cards and the split sections.

## Copy

Written in a relaxed voice with no em dashes. The feature claims map to things
that ship today: the agent providers (Claude Code, Codex, OpenCode, and Pi in
beta) with their two integration tiers, ambient awareness and the Triage Inbox,
the GUI agent pane, the review pane, the pane types, UI modes (fleet/focus),
the remote clients (the `/m` mobile PWA with push, `/remote`, and the full app
at `/app`), plugins, the MCP facade, and the desktop + claudemon + hub split.

## Docs

`docs.html` is generated from per-section drafts that were each grounded against
the real source, then swept for voice (no em dashes) and rendered to a single
static page. Sections: overview, install, architecture, agents and providers,
the agent pane, pane types, layout and navigation, attention, remote, extending
(plugins / mcp / tui), and configuration.
