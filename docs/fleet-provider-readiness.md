# Desktop provider readiness

`agents.checkProviderOnStartup` defaults to true. Settings → Fleet Manager →
Check provider at startup persists the opt-out through both config writers.
The desktop process schedules one small request for its selected manager provider
(two-second delay after local daemon startup completes). Disabling the setting
cancels in-flight work and prevents remaining automatic startup work. Enabling it
after startup does not launch another automatic request. Check again explicitly
requests a manual check regardless of the setting; concurrent requests and
immediate repeated clicks are coalesced. Renderer mounts/polls never spend.

A successful check means the provider responded to a small test request. It does
not guarantee a later manager launch, model, tool, profile or plugin works.
Authentication failures, allowance limits, connectivity failures, timeouts,
unsupported checks and unchecked accounts are distinct advisory facts. None adds
a launch gate. CLI discovery and runtime/facade health retain their contracts.

## Supported isolation

| Provider/context | Readiness ping |
| --- | --- |
| Native Claude with safe-mode capabilities, stream launch, desktop-owned default CLI account | Supported |
| Codex 0.153.4, native executable or exact verified npm wrapper with one matching installed native package, desktop-owned default OpenAI account route | Supported |
| Claude PTY | Unavailable: its legacy launcher resolver differs from stream |
| OpenCode | Unsupported: `--pure` is not a no-tools boundary |
| Copilot | Unsupported: tool filters do not establish isolation from all plugins/hooks, and CLI session state persists |
| Pi | Unsupported: stricter isolation contract not verified |
| Windows/script launchers except the exact verified Codex wrapper | Unsupported |
| Adopted daemons, remote/web, missing owner/capabilities, selected profiles/integrations | Unsupported/unchecked; never borrow another account's result |

The daemon must be a child started by this desktop with an unchanged inherited
environment. Its private pid and environment fingerprint never cross renderer
IPC. An adopted healthy daemon can have another account environment and remains
unavailable for inference checks. Runtime health still supports adoption.

### Claude

Installed Linux Claude 2.1.258 help establishes `--safe-mode`: custom instructions,
skills, plugins, hooks and MCP customizations are disabled while authentication
is preserved. Each native executable is capability-checked before inference.
`--bare` is unsuitable because it changes subscription authentication.

The readiness entry point reuses direct completion's process transport, with
safe mode, empty built-in/MCP tools, no persistence, fixed `Reply OK.` system
prompt/input, a temporary-directory cwd, and the registry's cheap Claude default
(`haiku`). API retries are disabled and output generation is capped at 64 tokens.
Help has a 3-second deadline, inference 15 seconds, and capture is bounded.
The [Claude environment reference](https://code.claude.com/docs/en/env-vars)
documents retry/output controls. No daemon upgrade or restart is required.

### Codex

`codexReadinessPing.ts` is separate from the generic completion adapter. The
existing `complete(requireNoTools)` Codex refusal remains intact. Support is pinned
to the inspected **0.153.4** contract; other versions fail closed before inference.

1. Resolve the configured native executable. The exact checked-in wrapper fixture
   can be resolved to a unique already-installed `@openai/codex@0.153.4` native
   package. The wrapper's hash must match; no shell parsing, `npx`, package fetch,
   runtime installation or package update runs. Unknown wrappers are refused.
2. A disposable app-server uses only initialize, config/read,
   configRequirements/read and model/list. It creates no thread. Plugins, hooks,
   bundled skill installation and incidental telemetry are disabled before boot.
   Config responses stay in memory and are never logged or returned to the UI.
   Custom provider/endpoints, relative account roots, forced auth policies, unknown auth controls and
   managed constraints are refused instead of silently changing accounts.
3. Select an advertised small model (mini/nano/luna naming convention) with an
   advertised none/minimal/low reasoning option. There is no fallback to another
   provider, an unadvertised model, or an expensive default. Preserve the selected
   model's transport flags from Codex's public `models_cache.json`, validating its
   client version and selected model. This file is model metadata, not auth storage.
4. Run one `exec --ignore-user-config --ignore-rules --ephemeral` request, with
   fixed `Reply OK.` instructions/input, zero project-doc bytes, automatic skill
   instructions disabled, and explicit tool/feature restrictions. A small local
   model catalog disables model-driven tools too; only transport flags are copied
   from the real catalog. `CODEX_HOME`, credential-store mode and known auth/proxy
   feature controls are preserved. Workspacer never reads auth.json/keyrings or
   copies credentials.
5. Built-in provider IDs cannot be overridden, so a private configuration name
   recreates the OpenAI auth routing (`requires_openai_auth=true`, standard auth-
   mode-derived endpoint and organization/project environment headers) with zero
   request/stream retries. This still uses Codex and the same account. It is not
   provider fallback. Explicit custom routes are rejected by the prerequisite.
6. Version check: 2 seconds; metadata: 4 seconds/128 KiB combined output;
   inference: 12 seconds/8,000 characters per pipe; outer scheduler: 20 seconds.
   Cancellation kills only disposable CLI processes. Scratch files contain only
   public model metadata and are removed. No worker or persistent thread is created.

Primary source evidence for 0.153.4:

- [Tool registry construction](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/src/tools/spec_plan.rs): feature switches alone do not remove model-driven clock/async tools; local metadata must remove those declarations.
- [Configuration schema](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/config.schema.json): skills.include_instructions, project_doc_max_bytes, tool switches and model_catalog_json.
- [OpenAI provider routing](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/model-provider-info/src/lib.rs): auth-mode-derived endpoints, required auth, headers and retry controls.
- [Responses Lite contract tests](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core/tests/suite/responses_lite.rs): Lite carries instructions/tools as input items rather than top-level fields.
- [Plugin startup gate](https://github.com/openai/codex/blob/rust-v0.153.4/codex-rs/core-plugins/src/manager.rs): disabled plugins do not perform startup sync.

## Data and ownership boundary

`provider:readiness` is local-only IPC. A free read and an explicit `check=true`
request are separate operations. Replies contain normalized state and an optional
timestamp, never completion text/errors, identities, binary paths, tokens or
environment. No providers.checkAll or headless capability changed. The bridged
desktop retains local preload; web/remote backends return unsupported.

The main cache uses private config/provider/resolved-binary/owner identity. Config
changes cancel checks and discard results; completions recheck identity. The
renderer keys results by provider/owner/account/config generation and rejects late
responses. Fleet Manager and Spawn share the same presentation.

## Verification and entry points

Read `services/providerReadinessRuntime.ts`, `services/providerReadiness.ts`,
`services/codexReadinessPing.ts`, `services/codexReadinessBinary.ts`,
`services/directCompletion.ts` and `renderer/src/hooks/useProviderReadiness.ts`
first (under `apps/desktop/src`).

The installed Codex loopback tests use a fresh temporary CODEX_HOME and no real
credentials. They assert one request, no tools, only the fixed tiny instructions/
input, ignored hostile config/repo instructions, no auth file and no sessions
folder. Both classic Responses and Responses Lite passed. Native 429/500 fixtures proved
there is no retry, and aborting a request stopped the disposable process. Wrapper resolution was
also checked against the installed wrapper/package without executing the wrapper.

Run the native wire tests explicitly with WORKSPACER_TEST_CODEX_BIN pointing to the
installed native binary. WORKSPACER_TEST_CODEX_WRAPPER optionally checks its wrapper.
The live test additionally requires WORKSPACER_TEST_CODEX_LIVE=1; it can consume
allowance and emits only normalized facts. The authorized live Codex ping passed.
Earlier attempts identified unsupported bundled model selection and the missing
Responses Lite transport flag; neither is used as a fallback now.

Config-through-adapter tests cover persisted disabled-zero-request behavior,
manual override, selected Codex binary/model, cancellation and unknown ownership.
No live app/daemon restart, fleet mutation, login/logout, direct credential
inspection/modification, upstream change, push or merge was performed. The primary
checkout remains at 551e1731; landing was paused when Codex became required.
Independent review was waived; implementation validation is separate.

Task: d451dd25-12c2-4bf7-845d-4bf7129d96ce;
implementation dispatch: 103ba6e0-3b90-4ff4-8281-2eaa373ed1df.

## Latest checks

On Node 22.22.2, desktop main passed 3,392 tests and renderer passed 1,877 tests,
run sequentially. Typechecks passed for both builds. The five affected readiness
browser cases passed in isolated Chromium. Opt-in installed-CLI tests separately
cover both wire formats, rate/server errors without retries, cancellation and
wrapper resolution; the opt-in live Codex test passed once a supported model and
its transport were preserved. No further live requests are needed for validation.
