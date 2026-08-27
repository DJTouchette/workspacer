---
title: SubagentUpdate is a sparse patch and it RECOMPUTES background_tasks, overriding the wire count
date: 2026-08-26
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/session/state.rs
promoted: false
---

# SubagentUpdate is a sparse patch and it RECOMPUTES background_tasks, overriding the wire count

## Observation
`SessionStore::apply_subagent_update` (services/claudemon/src/session/store.rs) has three behaviours that are not visible from its signature:

- It is a **sparse patch**: every `Option` field is only written when `Some`. There is therefore NO way to clear a description/model/tool summary once set — a later update carrying `None` keeps the old value. Only `status` is applied unconditionally. The store test asserts this directly (a Complete update with `description: None` still reads back the original "inspect").
- It **derives `background_tasks` from the row list** on every call: `subagents.iter().filter(status == Running).count()`. For a managed Codex session that silently overrides whatever `background_tasks` the wire/hooks reported. Anything that writes `background_tasks` for a session that also has subagent rows will be clobbered by the next subagent update, and vice versa — the two writers do not merge.
- `completed_at` is set once via `get_or_insert` on the Running→Complete edge, and cleared back to `None` if a row goes Running again. Timestamps are epoch **milliseconds** (`now_millis()`, a local helper), while the surrounding `SessionState.updated_at` is an `OffsetDateTime` — do not mix the two units.

The stop path force-closes rows: when a session is marked stopped, every still-Running subagent is flipped to Complete with `completed_at` stamped, alongside the existing `background_tasks = 0` reset. Without that a dead Codex session would badge "working in background" forever.</observation>
<parameter name="impact">A future field that legitimately needs clearing (say a `last_error`) cannot be cleared through this API. And any attempt to make `background_tasks` authoritative from the stream/hook side will fight this recomputation on Codex sessions.

## Recommendation
Treat `background_tasks` as DERIVED for providers with `subagents[]` rows and as the wire count elsewhere; if a provider ever needs both, add a separate field rather than teaching two writers to share one. To support clearing a field, change it to a tri-state rather than adding a sentinel string.</recommendation>
<parameter name="confidence">high
