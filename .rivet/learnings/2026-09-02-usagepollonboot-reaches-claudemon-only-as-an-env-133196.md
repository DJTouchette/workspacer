---
title: usage.pollOnBoot reaches claudemon only as an env var, and the Go launcher needed its own config read
date: 2026-09-02
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/account_usage.rs
  - services/claudemon/src/daemon/mod.rs
  - services/hub/cmd/workspacer/usagecfg.go
  - services/hub/cmd/workspacer/plan.go
  - apps/desktop/src/main/services/claudemonDaemon.ts
promoted: false
---

# usage.pollOnBoot reaches claudemon only as an env var, and the Go launcher needed its own config read

## Observation
claudemon has no config file and no config-push route — it reads a few env vars at the point of use (CLAUDE_CONFIG_DIR is the precedent). usage.pollOnBoot therefore travels as WORKSPACER_USAGE_POLL_ON_BOOT from BOTH spawn sites, and the two sites are asymmetric: apps/desktop's claudemonDaemon.ts has configService and computes 0/1; services/hub/cmd/workspacer (the `workspacer serve` launcher) has NO config service at all — cmd/brain's configService is package-main and not importable — so it needed a new 40-line usagecfg.go that reads <configDir>/config.yaml directly. Three defaults have to agree or the feature half-works: config_defaults.json says true, the daemon's poll_on_boot_from_env(None) says true, and the launcher's serveOptions.UsagePollOnBoot is a *bool where nil means "unstated, leave the variable unset". The pointer is load-bearing: a plain bool would make every serveOptions literal that forgets the field spawn a daemon with polling OFF.

## Impact
Any future setting that must reach claudemon hits the same shape. Getting the "absent means on" direction wrong in any one of the three places silently blanks every account gauge for skewed desktop/daemon versions.

## Recommendation
For a claudemon-consumed setting: env var (not a clap flag), absent = the shipped default, write it in BOTH directions from Electron (the crash-respawn path would otherwise inherit a stale value), and use a pointer/tri-state in the Go plan so the zero value is not an assertion. Verify by running the binary: `WORKSPACER_USAGE_POLL_ON_BOOT=0 claudemon serve --hook-port N --api-port M --db-path /tmp/x.db < /dev/zero` and grep the log — stdin MUST be held open or the parent-death pipe (internal/parentwatch) exits the daemon in ~2ms, before the poller's first tick.
