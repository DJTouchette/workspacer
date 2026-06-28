//! Provider adapters for non-Claude coding agents. See
//! docs/multi-agent-providers.md.
//!
//! Each adapter drives an agent's native machine interface (OpenCode's
//! `serve` HTTP+SSE, Codex's `app-server` JSON-RPC) and translates its events
//! into claudemon's existing session model — `SessionState` (mode / pending),
//! the conversation delta stream, and the status line (model / usage / cost) —
//! so the hub bus, renderer, and Fleet Deck observe every provider identically
//! to a Claude session.
//!
//! The translation is split into a *pure* layer (event JSON → typed updates,
//! unit-tested) and a thin *apply* layer that drives the stores. The live
//! process/transport client lives alongside each adapter.

pub mod opencode;
