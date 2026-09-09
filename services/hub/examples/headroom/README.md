# Optional Headroom launch integration

This example connects selected local Workspacer desktop sessions to a Headroom
proxy you run yourself. It supports Claude Code (GUI/stream and terminal) and
Codex (managed GUI, hybrid, and Windows terminal). Workspacer keeps its existing
provider drivers, approval UI, and resume handling.

## Setup

1. Install and run [Headroom](https://github.com/headroomlabs-ai/headroom) using
   its upstream instructions. The Python package supplies the CLI; the npm
   package is an SDK. Start the proxy with `headroom proxy --port 8787` and
   configure its upstream authentication and compression as needed.
2. In Workspacer, open **Plugins → Browse examples** and install **Headroom**. The
   adapter runs with Workspacer's bundled Node runtime; it needs no npm install.
3. In the plugin settings, set **Headroom proxy URL** if it differs from
   `http://127.0.0.1:8787`. Only loopback HTTP origins are supported. The adapter
   pane uses port 9130; **Workspacer hub URL** defaults to the local bus on 7895.
4. Open **Spawn agent**, select **Claude Code** or **Codex**, then choose
   **Launch integration → Headroom**. The default is **None**.
5. Open the Headroom pane for readiness, then run a small task and confirm the
   request in Headroom's dashboard. Readiness does not verify authenticated
   model traffic or establish token savings.

The selection is stored with the agent workspace and re-applied on restart and
resume. If the plugin is disabled, missing, or its proxy is unavailable, launch
fails with an error. Restore the plugin/proxy, or start a new session with None.
Existing running processes keep their launch environment. Changing a plugin
setting restarts the adapter; it takes effect on the next agent launch.

## Codex routing

Workspacer asks the installed Codex CLI's `config/read` API to resolve routing
under the selected account's `CODEX_HOME` and config overrides. It passes only
the provider ID and optional upstream URL to the adapter, alongside the session
cwd, model, and resume flag. Credentials, raw configuration, and inherited
environment are not included in that request.

The adapter preserves the provider ID and existing login. For built-in OpenAI
it sets child-local `OPENAI_BASE_URL` and `openai_base_url`; for a custom provider
it overrides that provider's base URL and supplies the original upstream to
Headroom via an environment-backed header. Integration arguments follow profile
arguments. No global Codex config or Claude settings file is rewritten by the
integration.

Native Codex named presets (`-p` / `--profile`, including a default legacy
profile) are currently rejected when this integration is selected: use routing
in the base config of a Workspacer account profile instead. Custom providers need
an explicit HTTP(S) base URL without embedded credentials, query, or fragment.
A config that already points at this proxy is rejected to prevent a routing loop.

## Scope and development

This is an optional **trusted sidecar**, not a bundled Headroom service. Workspacer
maintains the launch contract and example adapter; you maintain Headroom and its
provider compatibility. Headless/web/federated spawns and other agent harnesses
do not support this contribution yet. The plugin is not installed automatically.

The adapter uses the supervisor-issued `HUB_TOKEN` and merged `WKS_SETTINGS`.
It does not read the host's shared hub token. It registers only its declared
`workspacer.headroom.prepareLaunch` method and uses Node 22+ built-ins at runtime.

From the repository root, after installing desktop development dependencies:

```sh
node --test services/hub/examples/headroom/adapter.test.mjs
cd services/hub && go test ./internal/plugin
```

The tests use a fake proxy and bus. Authenticated Claude/Codex traffic remains a
manual integration check with your own account.
