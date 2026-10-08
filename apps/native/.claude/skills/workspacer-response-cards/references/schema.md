<!-- workspacer:generated response-cards skill -->
# `wks-html-card` schema

## Envelope

| field | required | notes |
|---|---|---|
| `v` | yes | Exactly `1`. Any other value renders the fallback instead — there is no best-effort parse. |
| `title` | yes | Plain text, ≤120 chars. Shown in the card header and used as the frame's accessible name. |
| `bodyHtml` | yes | An HTML **fragment**. The host owns the document, its stylesheet and its security policy; you supply the inside. |
| `fallback` | yes | Plain text, ≤4000 chars. The whole answer, without the card. |
| `css` | no | Stylesheet text, ≤16 KB. Resource requests, including escaped `@import` and `url()`, are blocked by the host CSP. |
| `actions` | no | Up to 8; see below. An invalid or unknown action refuses the whole card. |

Whole block ≤ 64 KiB of UTF-8. Over that, or malformed, or a version this
build does not know, and the reader gets the raw block on demand (plus
`fallback` when it can be recovered) — never a blank space.

## Markup that survives

Text, headings, lists, tables, `<details>`, `<pre>`/`<code>`, `<input type="search|text|checkbox|radio">`, `<button>`, and
`class`/`id`/`style`/`aria-*`/`data-wks-*` attributes.

Removed: `<script>`, `<style>` (use the `css` field), `<iframe>`, `<form>`,
`<svg>`, `<object>`, `<meta>`, `<link>`, every `on*` attribute, and any
`href` or `src`. The card runs with no network
access at all, so an external image or font would only ever be a blank box.

## Actions

Each entry is `{ "kind": …, "label": "…", … }`. `label` is ≤48 chars and must
say what the button does.

```json
{ "kind": "open_worker",   "label": "Open the worker",  "sessionId": "abc123" }
{ "kind": "view_diff",     "label": "See the diff",     "path": "src/app.ts" }
{ "kind": "fill_composer", "label": "Draft the reply",  "text": "Please …" }
```

- `open_worker` opens a viewer pane for that agent. The session is checked
  against live state when clicked; only a direct child of this chat session can open. Gone or foreign sessions are refused.
- `view_diff` shows an inline HEAD-to-working-file snapshot (text files up to 256 KiB). `path` is absolute or
  relative to the chat's own project, and a path outside that project is
  refused — so link files you actually worked on, not arbitrary ones.
- `fill_composer` puts `text` in the message box, appended to whatever the
  user has already typed. **It does not send.** Write the text as something the
  user will read and choose to send, not as a command that assumes they will.

Keep any required `wks-result` or `wks-escalation` fence last, after cards.

There is deliberately no action that spawns an agent, changes configuration,
grants a permission, writes a file or sends a message. Do not ask for one in a
card's prose either — the button is the whole vocabulary.
