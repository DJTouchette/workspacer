---
title: The MCP facade's untokened default is now deny, and nothing needed a new credential to make that work
date: 2026-08-29
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub/cmd/mcp/main.go
  - services/hub/cmd/mcp/auth_test.go
  - apps/desktop/src/main/services/mcpFacadeDaemon.ts
  - apps/desktop/src/main/services/mcpConfig.ts
  - services/hub/cmd/brain/facade.go
promoted: false
---

# The MCP facade's untokened default is now deny, and nothing needed a new credential to make that work

## Observation
cmd/mcp shipped `-untokened operator`, so an uncredentialed POST to 127.0.0.1:7897/mcp got the full 68-tool operator surface (spawn_agent, write_file, save_config, send_message) — verified live against the running dev build before the change. The fix was to flip `defaultUntokened` to `untokenedDeny` and make `resolveRecord`'s untokened switch allowlisted (only operator/view return a record; deny, the zero value, and any unrecognized value fail closed). NO token provisioning was needed, because no legitimate consumer is credential-less: claudeSpawn.ts and managedSpawn.ts ALWAYS mint a per-session scoped token when `wantsFacade` (Authorization header on the generated --mcp-config for Claude PTY/stream; `?t=` on the URL for codex/opencode/copilot), cmd/brain/facade.go does the same headlessly, and deploy/fly/node/entrypoint.sh already defaulted WKS_MCP_UNTOKENED=deny. `pi` gets the bare MCP_FACADE_URL but ships no MCP client (providers/pi.rs only warns), and `supervisorMcpConfigPath()` / supervisor-mcp.json is dead — facadeSpawnArgs only falls back to it when called without a token, which no caller does.</observation>
<parameter name="impact">Any other local process or local user account — and a container sharing the host network namespace — could drive the whole fleet with no credential. DNS rebinding was already closed by requireHost, and non-loopback binds were already refused without a token, so the residual risk was strictly local-process/local-user. The only thing `deny` breaks is a HAND-CONFIGURED MCP client that sends no token (the one documented at landing/build.html), which now shows the tokened form; `workspacer token create --scope operator` mints one, and `facade.untokenedAccess: operator` opts back open.

## Recommendation
Do not restore untokened operator — TestUntokenedDefaultDeniesFleetControl (cmd/mcp/auth_test.go) pins the constant, untokenedDefault()'s env resolution, the gate at both the shipped default and the zero value, and a 401 at the HTTP boundary. When adding a facade consumer, mint it a per-session token rather than relying on loopback. Still open and deliberately out of that fix: claudemon's session API on 127.0.0.1:7891 has no credential check at all (POST /sessions/:id/message, /sessions/spawn) — closing it means distributing a secret to clients in three languages including the PTY wrapper inside every agent process, which is why the facade fix did not extend to it.
