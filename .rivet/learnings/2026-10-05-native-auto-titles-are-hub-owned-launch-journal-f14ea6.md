---
title: Native auto titles are hub-owned launch-journal state, opt-in per launch
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/src/services/sessions/titles.rs
  - services/hub-rs/src/services/agent_lifecycle.rs
  - services/hub-rs/src/services/spawn_plan.rs
  - apps/native/src/controller.rs
promoted: false
---

# Native auto titles are hub-owned launch-journal state, opt-in per launch

## Observation
Automatic session titles for native (#25) live in hub-rs, not the client: agents.spawn autoTitle:true (sent by native only when no label) records autoTitle{state:pending,prompt} in the launch journal (spawn_plan::resolve). services/sessions/titles.rs is offered every published row; at the first turn boundary (mode input/approval/question/stopped) with an assistant_text after the first user_message (or a stop) it calls provider_utilities::Service::suggest and commits via Lifecycle::note_auto_title, fenced on exact generation + still pending + no launch label. Snapshot precedence: launch label > tui-names cwd rename > autoTitle.title (snapshots::with_auto_title, and the same chain in live_controls sessions.recent, whose label field is 'name' — 'title' is always empty). Resume copies prior autoTitle. Lifecycle::enrich strips autoTitle.prompt so the opening request never reaches snapshots. Opt-in (not all unlabelled launches) because Electron's renderer already titles its own layout agents and MCP children would otherwise silently spend calls.

## Impact
Anyone changing labels, resume, or session projection must keep the three-level precedence and the generation fence or a late title can overwrite a user's name.

## Recommendation
New title consumers should read snapshot label; new rename paths should write a launch label or cwd name, never autoTitle.
