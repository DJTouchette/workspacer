---
title: `workspacer serve` never ran `claudemon init` — hookless PTY sessions never receive their first message
date: 2026-08-24
promoted: true
promoted_to: workspacer-serve-cli
---

# `workspacer serve` never ran `claudemon init` — hookless PTY sessions never receive their first message

## Observation
Until 2026-08-24, `workspacer serve` (services/hub/cmd/workspacer/plan.go buildServePlan) passed claudemon only `serve --host --hook-port --api-port`. `claudemon init` is a PEER subcommand (services/claudemon/src/cli.rs) and its only caller in the whole repo was the desktop's Electron main (apps/desktop/src/main/index.ts, runClaudemonInit). Since both share one ~/.claude/settings.json, serve inherited working hooks on any machine where the desktop had ever run, and registered nothing on a state dir where it had not.

The severe consequence is NOT missing telemetry. A PTY session is born SessionMode::Unknown (session/state.rs) and ONLY hook events move it to Input/Responding. A spawn's `first_message` is held until the `Input` transition (session/store.rs queue_first_message → schedule_pending_flush, and the test `a_pty_first_message_waits_for_the_input_transition`). So with no hooks a dispatched PTY worker NEVER RECEIVES ITS PROMPT — it sits at an empty composer looking alive. POST /message likewise only queues. Permission prompts also produce no approvable record (see the note on HookEventKind::PermissionRequest).

Counter to what a scout report claimed, fleet.quiescence is NOT fooled: internal/quiescence stateBlocker treats `mode: "unknown"` as KindSessionUnknown, i.e. a BLOCKER, not rest. A hookless PTY session pins the machine awake — the safe direction, and the reason this stayed invisible. Stream transport (the default, cmd/brain/config_defaults.json) is unaffected: managed adapters call providers::set_mode directly.</observation>
<impact>Any headless/container/CI deployment on a fresh state directory: dispatched PTY workers silently do nothing. Fixed by running `claudemon init --hook-port N` as a one-shot pre-flight inside bootStack, before the daemons (servePlan.Init), with `--no-claudemon-init` to opt out.</impact>
<recommendation>When adding anything to the serve path that depends on Claude Code hooks, remember the desktop and the CLI share ~/.claude/settings.json — a dev machine cannot reproduce the hookless case. Test against a HOME with no settings.json.</recommendation>
<related_paths>["services/hub/cmd/workspacer/plan.go", "services/hub/cmd/workspacer/serve.go", "services/claudemon/src/cli.rs", "services/claudemon/src/session/store.rs", "services/hub/internal/quiescence/quiescence.go"]</related_paths>
<suggested_doc>workspacer-serve-cli</suggested_doc>
<confidence>high</confidence>
</invoke>
