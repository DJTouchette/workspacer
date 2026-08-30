---
title: `workspacer serve` left claudemon's SQLite session store unpinned — two stacks on alternate ports shared one state.db
date: 2026-08-24
promoted: true
promoted_to: workspacer-serve-cli
---

# `workspacer serve` left claudemon's SQLite session store unpinned — two stacks on alternate ports shared one state.db

## Observation
Until 2026-08-24, buildServePlan (services/hub/cmd/workspacer/plan.go) passed claudemon `serve --host --hook-port --api-port` and NO `--db-path`. cli.rs:77 then fell back to `crate::store::default_db_path()`: $XDG_DATA_HOME/claudemon/state.db, else ~/.claudemon/state.db, else the RELATIVE `.claudemon/state.db` under the process CWD. Nothing downstream supplied it — the desktop's claudemonDaemon.ts spawn omits it too, and there is no env override besides XDG_DATA_HOME (read inside claudemon).

Consequence: bootStack refuses busy ports, so the ONLY way to run a second stack is to change claudemon's ports — and that second daemon then opened the SAME ~/.claudemon/state.db. The two share the `sessions` and `events` tables: the newcomer's boot hydration (daemon/mod.rs load_recent_sessions → store.hydrate, which marks rows Stopped IN MEMORY only) lists the live stack's agents as its own resumable sessions, its clients can `claude --resume` them, and its fleet.quiescence sampler counts them. Silent on both sides.

Fix: `resolveDBPath` (serve.go) decides before any port probe or spawn, and buildServePlan carries the answer into the argv. The derived default MIRRORS default_db_path exactly — desktop and serve share one session store on purpose (adopt-don't-kill), so relocating it would strand every install. Two deliberate non-copies: claudemon's relative third fallback, and Rust's `env::var` returning Ok("") for a set-but-empty XDG_DATA_HOME (which also yields a relative path) — both now refuse with a named fix instead of silently landing the DB on an ephemeral CWD. New `--claudemon-db-path` flag, REQUIRED whenever either claudemon port differs from its default.

Also verified while checking this: `serve` does refuse busy ports rather than killing them (serve.go probeListen) — unlike the desktop, which kills stale listeners because it owns its daemons.</observation>
<impact>Local testing procedures that start a second stack, `plugin dev` on alternate ports, and any container running two stacks. Also removes the container footgun where an unresolvable HOME put the session DB on the ephemeral rootfs.</impact>
<recommendation>Anything added to claudemon's argv in plan.go should ask "is this persistent state?" — ports were all pinned and the one file it opens was not. Keep deriveClaudemonDBPath in lockstep with store/mod.rs default_db_path; TestResolveDBPath is the guard.</recommendation>
<related_paths>["services/hub/cmd/workspacer/plan.go", "services/hub/cmd/workspacer/serve.go", "services/claudemon/src/store/mod.rs", "services/claudemon/src/cli.rs", "services/claudemon/src/daemon/mod.rs"]</related_paths>
<suggested_doc>workspacer-serve-cli</suggested_doc>
<confidence>high</confidence>
