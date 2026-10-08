---
name: workspacer-response-cards
description: Present a structured answer in Workspacer chat as an inline review summary, comparison, checklist, or filterable table. Use when the structure benefits from expansion, filtering, or sorting; ordinary prose and code use normal Markdown.
---

# Response cards

Write a useful prose summary, then add one or more complete `wks-html-card`
fences in your assistant reply. Keep any required result or escalation fence last.
Cards supplement the answer; other clients display the original JSON as text.

```wks-html-card
{"v":1,"title":"Summary","bodyHtml":"<p>An HTML fragment.</p>","fallback":"The same useful answer in plain text.","actions":[]}
```

The entire JSON body must fit in 64 KiB of UTF-8, with at most eight actions.
`fallback` is required and appears in an accessible Text alternative disclosure.
Malformed, incomplete, unsupported, or oversized cards remain readable without
mounting a frame.

Supply declarative content only: no JavaScript, event handlers, navigating links,
resource URLs, embedded documents, forms, or network access. The host supplies
its theme and fixed interaction runtime. Use `<details>/<summary>` for expansion.
Read [interactivity](references/interactivity.md) for filter and sort attributes.

The host draws explicit-click buttons outside the frame:

- `open_worker`: view a live direct child of the owning chat session.
- `view_diff`: inspect an exact, contained HEAD-to-working-file snapshot inline.
- `fill_composer`: append a draft to this chat's composer; never send it.

Read [schema](references/schema.md) for fields and limits. Read the
[three examples](references/examples.md) for a review finding, comparison, and
checklist. Preserve the user's chosen task and output contracts.
