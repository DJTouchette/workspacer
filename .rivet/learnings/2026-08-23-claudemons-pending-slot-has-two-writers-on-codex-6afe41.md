---
title: claudemon's pending slot has TWO writers on codex/opencode/pi, not one
date: 2026-08-23
confidence: high
suggested_doc: session-lifecycle
promoted: false
---

# claudemon's pending slot has TWO writers on codex/opencode/pi, not one

## Observation
The "exactly one feed owns a session's pending slot" invariant was false for the three managed non-claude providers. daemon/mcp_ask.rs (the AskUserQuestion MCP shim registered for codex via --config mcp_servers, opencode via ask_mcp_entry, pi via ask_extension_source) parks and resolves the same slot the driver task uses for approval cards. Before 2026-08-23 both of its writes were unattributed, because PendingWrite fenced only Keep: a question raised beside a parked approval overwrote the card, and QuestionGuard::finish/drop then cleared the slot — while the driver's FIFO still held the approval and only re-surfaces a queued card from its /approve decision arm, so nothing could answer it. Verified by test (both assertions read "pending is None" before the fix). Fix: PendingWrite::Park/::Resolve carry a PendingOwner (Primary = hook feed or driver task, Ask = the shim); a foreign park displaces the card and it is restored when the displacing request is released; a resolve clears only what its own feed raised. The slot is now private (SessionState::pending_card, read via pending(), written only via write_pending), which surfaced a third writer nobody had counted: SessionStore::park_decision, the PTY hook gateway.</observation>
<parameter name="impact">Every worker-freeze this project has had was a pending-slot ownership violation, and censuses of the writers have been wrong four times in a row (each one found more writers than the last). Anyone adding a feed that can block the user — a new provider shim, a federation mirror, a new MCP endpoint — is adding a slot writer, and the single-owner framing will mislead them.</impact>
<parameter name="recommendation">Never count slot writers by reading the doc comments; grep for writes and check who else can raise a user-blocking request for that provider. New writes go through SessionState::write_pending with an explicit PendingOwner — the field is private, so the compiler will make you. Note "one feed" is not "one request": a codex session can legitimately hold an approval AND a question at once, and a driver must take cur_mode from what set_managed_mode RETURNS, since releasing its own card can leave the session parked on the other feed's.</recommendation>
<parameter name="related_paths">["services/claudemon/src/daemon/mcp_ask.rs", "services/claudemon/src/session/state.rs", "services/claudemon/src/session/store.rs", "services/claudemon/src/providers/*.rs"]
