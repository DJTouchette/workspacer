package main

// TestSpawnRemoteBypassProfileExtraArgsScrubbed: the remote spawn clamp must not
// be defeatable by pointing agents.spawn at a local profile whose extraArgs pin
// --dangerously-skip-permissions / --permission-mode bypassPermissions. The clamp
// zeroes the request fields; it must also strip the profile's smuggled flags, or
// a bus/web/MCP caller starts a YOLO agent the clamp claims to forbid.
// TestSpawnRemoteProfileSettingsAndConfigDirDropped exercises the same clamp
// through the handler a bus caller actually reaches, for the two doors the old
// denylist left open: --settings (a settings file can carry permissions and
// hooks) and the profile's configDir (which becomes CLAUDE_CONFIG_DIR, the
// directory those very settings are read from).
// The SHIPPING DEFAULT leg. config_defaults.json sets claude.transport to
// "stream", so a spawn that names no transport goes through spawnManagedSession
// and POSTs /sessions/spawn-managed — and both scrub tests above pin
// `"transport":"pty"` in their params, so the leg that actually answers by
// default was untested. Deleting `scrubBypassProfile(...)` from that call site
// left the whole hub suite green while the default bus/remote/MCP spawn
// forwarded `--dangerously-skip-permissions`, `--settings /tmp/evil.json`,
// `--allowedTools Bash,Edit` and `CLAUDE_CONFIG_DIR=/tmp/attacker-claude-home`
// to claudemon. CLAUDE_CONFIG_DIR supplies settings.json — permissions.allow
// and hooks, i.e. commands claude runs unprompted — which is the exact
// escalation scrubBypassProfile's own comment names.
