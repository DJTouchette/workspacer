---
title: "Limit-aware routing: roles to capabilities to (provider, model, effort), plus the two things it enforces"
tags: [hub, go, routing, fleet-manager, agent-spawn, model-selection, ceilings, security-invariant, desktop]
related_paths:
  - "services/hub/internal/routing/routing.default.yaml"
  - "services/hub/internal/routing/policy.go"
  - "services/hub/internal/routing/fresh.go"
  - "services/hub/internal/routing/ceiling.go"
  - "services/hub/internal/bus/rpc.go"
  - "services/hub/cmd/hub/main.go"
  - "services/hub/cmd/hub/routingceiling.go"
  - "services/hub/cmd/mcp/main.go"
  - "apps/desktop/src/renderer/src/lib/fleetManager.ts"
  - "apps/desktop/src/renderer/src/components/settings/SupervisorSection.tsx"
  - "docs/limit-aware-routing.md"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Limit-aware routing

## Policy sources and evidence

The hub composes compiled routing defaults, host routing.yaml and managed
preferences. File-backed policy is content-hash checked on a 30-second schedule;
managed preference changes install immediately. A bad edit retains the last
valid policy. Compiled defaults change with the binary, not by editing the source
YAML beside a running process.

The matrix maps role → capability → provider/model/effort and carries profiles,
ranks, thresholds, directory ceilings, freshness and unknown-provider policy.
Explicit user/model choices and manager preferences also exist; it is not true
that no model can ever be named outside this matrix.

`services/hub/internal/routing/policy.go` is a pure decision layer over
the matrix, current time and `services/hub/internal/limits` evidence. Observed health and
assumed effective health are separate: an unreadable quota bucket must not become
green just because policy permits proceeding. Window currency is checked before
thresholds or spend-down arithmetic, so a reset already in the past cannot look
like an attractive imminent reset.

Forecast weights produce weighted work units, not a quota percentage.
`expectedWork` alone does not establish demand before reset; a genuine
forecastDemandBeforeResetPct is a distinct input. Keep unknown assumptions and
refusal reasons on the decision returned to callers.

## Selection versus spawn enforcement

`routing.select` is a read-only selection RPC. A raw `agents.spawn` does not
need a prior selection receipt; a decisionId is correlation metadata, not an
authorization token. Nevertheless selection can be composed into a dispatch:
`services/hub/cmd/mcp/workflow_dispatch.go` selects locally or through the paired
routing method before spawning. The old “nothing calls selection during a
spawn flow” statement is too broad.

The bus router separately calls its configured ceiling/freshness hook during
spawn sanitization. This shared site protects desktop/headless bus providers
and qualified routing paths; native Electron IPC is a different entry point.
Do not infer all entry points have identical policy merely because they launch
the same provider family.

The binding policy includes:

- **Directory capability ceiling:** canonical cwd is compared against configured
  ancestors, longest applicable path wins. Invalid configured ceiling values
  refuse rather than silently falling through to an uncapped result.
- **Concrete model enforcement:** a refused capability/model can be replaced by
  the permitted matrix tuple. Merely deleting a strong model and allowing a
  strong provider default would not enforce the ceiling. Explicit exactModel
  conflicts refuse instead of silently substituting; an explicitly named harness
  is not casually swapped for another provider.
- **Freshness:** a role/capability requiring fresh context cannot also resume an
  old conversation. That is refused, not converted into an unannounced fresh start.

Legacy max_tool_scope values are parse-compatible but do not restrict the ambient
Workspacer tool surface. Capability/model ceilings and provider permission modes
are different axes.

## Wire and caller responsibilities

`role` expresses the work role and participates in selection/freshness.
`capability` is checked against the configured ceiling. `decisionId` joins the
selection to a spawn record. None of those caller-supplied strings proves task
ownership or that a prior result was accepted. Preserve host-owned task/workflow
identities through the dedicated dispatch APIs.

`escalationScrubbed` reports what the router/provider removed or rewrote; callers
must inspect it instead of assuming all requested options were honored. The
router removes caller-authored scrub receipts before constructing its own.

The manager doctrine lives in `main/shared/managerDoctrine.ts`; automatic worker
routing asks for a selection and forwards its result. Composed workflow dispatch
can perform that sequence itself. Paired work uses the selected remote cwd and
remote provider/readiness view rather than guessing from the local machine.
The ordinary spawn dialog and legacy resume paths may supply no role and make
no role-based freshness claim; inspect the exact caller before documenting it.

The manager’s own harness/model/effort/context choices come from
agents.managerProvider/managerModels/managerEfforts/managerContextWindows, with
host launcher resolution. The matrix’s supervisor role entry is not a second
independent selector for the manager’s launch settings.

## Managed preferences and authority

Managed preference operations write a typed private sidecar over host policy;
they do not arbitrarily edit the host YAML. Combined-source stamp checks protect
against racing policy changes, reset reveals inherited values, and protected model
classifications/ranks/ceilings/freshness floors cannot be weakened by a preference
patch. A configured operator scope is not the same as authenticated host identity.
The hub and facade administration paths perform their own stronger owner checks.

The select_model MCP tool belongs to the operator catalog rather than view/triage.
This is tool-catalog policy, separate from enabled-plugin ambient methods and
separate from owner-only preference administration. Do not collapse those three
boundaries into “operator means every administrative action.”

## Decision logs and platform comparisons

The decision log records selection/spawn correlation and reasons, not the whole
prompt or raw credential. Rotation preserves a prior generation. Log creation
and repair are platform-specific: Unix mode 0600; Windows protected owner DACL
with append/write-DACL access. Synthetic Windows mode bits do not prove privacy.

Routing canonicalizes cwd before comparing ceiling paths. Windows comparison uses
CompareStringOrdinal with counted UTF-16 rather than generic Unicode case folding;
component boundaries still reject sibling directories such as client-old for a
client root. Unix has its own comparison path. Build dirty-path fixtures literally:
filepath.Join can normalize away the spelling a test meant to exercise.

Historical CI IDs and old merged fixes are not current test results. Platform
comparison/privacy tests must run on the relevant OS before claiming runtime
Windows verification.

## Verification

From `services/hub`, run routing and limits tests, bus spawn-ceiling/freshness
checks, and MCP selection/preference/composed-dispatch tests. This checkout’s
Unix append-repair fixture assumes creating a 0644 file actually produces that
mode; with umask 0077 it fails before the production operation. A validation run
can use a separate shell with umask 0022; record that condition rather than
hiding the initial fixture failure.

Keep `contracts/usage-window-currency-cases.json` and the context/manager selection
contracts aligned with their other consumers. A green policy test does not
prove current account availability or that a real provider accepted its selected
model; those observations belong to runtime readiness and dispatch results.
