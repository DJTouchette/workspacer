# Desktop provider readiness

`agents.checkProviderOnStartup` defaults to true. Settings → Fleet Manager →
Check provider at startup persists the opt-out through both config writers.
The desktop process schedules one small request for its selected manager provider
(two-second delay). Disabling the setting cancels in-flight work and prevents
remaining automatic startup work. Enabling it after startup does not launch a
new automatic request. Check again is an explicit manual request regardless of
the setting; concurrent requests and immediate repeated clicks are coalesced.
Mounting a renderer component and polling a snapshot never launch inference.

A successful check means the provider responded to a small test request. It does
not guarantee that a later manager launch, model, tool, profile, or plugin works.
Authentication failures, allowance limits, connectivity failures, timeouts,
unsupported checks and unchecked accounts are distinct advisory facts. None adds
a launch gate. CLI discovery and runtime/facade health retain their own contracts.

## Supported isolation

Initial inference support is **desktop-local native Claude with the required
safe-mode capabilities**. Windows and script/package-manager launchers are
unsupported. The installed Linux Claude 2.1.258 help documents `--safe-mode` as
disabling CLAUDE.md, skills, plugins, hooks, MCP and other customizations while
preserving authentication. The exact configured executable is checked with
bounded help before each inference; absent flags mean unsupported, without a
provider fallback. It runs with the same inherited account environment, no
credential-file inspection or credential copying by Workspacer.

The existing direct-completion transport now has a separate readiness entry
point. It uses safe mode, empty built-in/MCP tools, no session persistence, a
fixed `Reply OK.` system prompt and input, a temporary-directory working directory,
and the completion registry's cheap Claude default (`haiku`). No project prompt,
history, repo instructions, plugin or worker/session is attached. CLI API retries
are disabled, output generation is capped at 64 tokens, each pipe is capped at
8,000 characters, help has a 3-second deadline, inference 15 seconds, and the
scheduler has an outer 20-second bound. No daemon restart or newer-daemon
assumption is needed: safe mode disables hooks before a session can be ingested.
Unknown or unsuccessful JSON output never means successful inference.

The CLI environment controls for retry/output bounds are documented in the
[Claude environment reference](https://code.claude.com/docs/en/env-vars).
Safe-mode semantics were established from the installed executable's help;
`--bare` is deliberately unsuitable because it changes subscription auth.

| Provider/context | Readiness ping |
| --- | --- |
| Native Claude with required safe-mode flags, local default CLI account | Supported |
| Codex | Unsupported: existing no-tools guard remains intact; installed exec help offers no blanket tool disable |
| OpenCode | Unsupported: `--pure` is not a no-tools boundary |
| Copilot | Unsupported: existing tool filters do not establish isolation from all loaded plugins/hooks; it also persists CLI session state |
| Pi | Unsupported: not installed for verifying the stricter instruction/isolation contract |
| Remote/web, older hosts without local IPC, selected profiles/integrations | Unsupported or unchecked; no local account inference |

Codex's local PATH wrapper also runs `npx --prefer-online`; it was not invoked
for auth status. No login-status fallback is shipped. Codex readiness is **not
fixed by an inference ping** in this iteration.

## Data and ownership boundary

`provider:readiness` is local-only IPC. A free read and an explicit `check=true`
request are separate operations. Replies contain only normalized state and an
optional timestamp, never completion text/errors, identities, binary paths,
tokens or environment. No `providers.checkAll` or headless capability changed.
The bridged desktop retains the local preload; web/remote backends return
unsupported. Selected profiles/integrations cannot reuse default-account facts.

The main cache uses private config/provider/binary/owner identity. Config changes
cancel checks and discard results; every completion also rechecks identity.
The renderer keys its display by provider/owner/account/config generation and
rejects late responses. Fleet Manager and Spawn share the same presentation.

## Verification and entry points

Read `services/providerReadiness.ts`, `services/providerReadinessRuntime.ts`,
`services/directCompletion.ts` (`completeReadinessPing`), and
`renderer/src/hooks/useProviderReadiness.ts` first (under `apps/desktop/src`).
Tests use controlled process/IPC fixtures, including successful responses,
explicit auth failures, quota, unsupported CLI versions, cancellation, deadlines,
late responses, ownership, and persisted opt-out through the actual scheduler.
No live inference or authentication-status command was run during implementation.

Task attribution: d451dd25-12c2-4bf7-845d-4bf7129d96ce;
implementation dispatch: 103ba6e0-3b90-4ff4-8281-2eaa373ed1df.
