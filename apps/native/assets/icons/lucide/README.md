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

`history.svg` and `server.svg` (the sidebar Workspace menu's Session history
and Hosts rows, so neither reads as another folder or as the title bar's
conversation history) come from `lucide-react` 0.564's icon nodes.

`text-quote.svg` (the chat's Quote action on selected text) comes from the
same `lucide-react` 0.564 icon nodes.
