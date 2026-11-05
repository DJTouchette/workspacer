Lucide icons (ISC license, https://lucide.dev) that the bundled gpui-component
set lacks, copied from the `lucide-react` 0.460 icon nodes the desktop uses for
the same affordance (response-card actions in `HtmlResponseCard.tsx`). They
stroke with `currentColor`, so the native client tints them with `text_color`.

`reply.svg` and `megaphone.svg` (fleet wake cards' Reply action and progress
icon, as `FleetMessageCard.tsx`) come from the official Lucide repository's
`icons/` directory.

`snowflake.svg` (a session whose prompt cache has expired, in the sidebar, the
context gauge and the composer's cold-cache note) comes from the same `icons/`
directory.

`git-pull-request.svg` (the title bar's pull request button and chip, and the
Pull request page) comes from the same `icons/` directory.

`file-text.svg`, `pencil.svg`, `sparkles.svg` and `wrench.svg` (the v2
transcript tool event rows: read, edit, skill and other tools) come from the
same `icons/` directory.

`layout-grid.svg` (Fleet), `server.svg` (Machines), `plug.svg` (Settings →
Agents), `keyboard.svg` (keyboard shortcuts), `clipboard-list.svg` (session
details), `history.svg` (conversation history), `list-checks.svg` (background
tasks, a static mark: the spinner means running), `rotate-ccw.svg` (Retry) and
`refresh-cw.svg` (Refresh) give each concept its own glyph under design v2
(r4-calm, R4.6). They are written from the Lucide icon nodes of the same names.

`sticky-note.svg` (Scratch: files agents wrote for you to review and run) gives
Scratch a glyph of its own, so no two sidebar destinations share a folder
(r5-shell, R5.4). It is written from the Lucide icon node of the same name.
