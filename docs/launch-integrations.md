# Optional launch integrations

A sidecar plugin can prepare a **selected local desktop agent launch** through
its existing hub capability connection. This extends the plugin manifest with
one optional contribution; it does not introduce a second plugin system.
[Headroom](../services/hub/examples/headroom/README.md) is the bundled example,
installed on demand from **Plugins → Browse examples**.

```json
{
  "id": "example.route",
  "apiVersion": "1",
  "server": { "command": "node", "args": ["server.mjs"] },
  "provides": ["example.route.prepareLaunch"],
  "launchIntegration": {
    "version": 1,
    "agents": ["claude", "codex"],
    "prepareMethod": "example.route.prepareLaunch"
  }
}
```

The method must be an exact entry in `provides`, within the plugin's namespace.
Existing scoped bus registration and plugin consent apply. `launchIntegration`
requires a sidecar; webview-only plugins cannot contribute it. Sidecars already
run as the OS user, so install only ones you trust.

On an opted-in spawn or resume the desktop calls the method with:

```json
{
  "version": 1,
  "agent": "codex",
  "cwd": "/project",
  "model": "configured-model",
  "resume": true,
  "provider": { "id": "openai" }
}
```

`model` is optional. `provider` is Codex-only routing metadata with an optional
`baseUrl`; the host resolves it with the installed CLI. The request contains no
raw provider config, inherited environment, tokens, or authentication headers.

Return an object with optional `env` (string-to-string map) and `args` (string
array). Environment values overlay the launch environment; arguments append to
the existing profile/host arguments. Neither field may be null. Up to 64 entries
are accepted in each; environment names must be normal identifiers (128 chars
maximum), values at most 32,768 chars and arguments at most 8,192 chars. NULs and
prototype keys are rejected. Extra fields, including an executable or cwd
override, are rejected. Only the child launch receives the patch.

Selection defaults to None. With None, no plugin discovery, routing probe, or
preparation RPC runs. An explicit selection is saved with the agent workspace
and retained on restarts/resumes. Invalid responses, unavailable plugins, RPC
errors, and readiness failures stop the launch; there is no direct-route fallback.
Existing sessions are not reconfigured when settings change.

Current host entry points are Claude PTY/stream and Codex managed/hybrid launches
from the local desktop. Web and federated launches explicitly reject a selected
integration. Headless agents.spawn does not implement the contribution. Other
harnesses and automatic Fleet dispatch integration defaults are outside this
initial contract. See the Headroom README for Codex preset/routing restrictions.
