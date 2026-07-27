/**
 * Handoff dialog — the composer's ⇄ button.
 *
 * A handoff is "spawn a successor agent that inherits this session's work but
 * not its context window". That successor is a brand-new agent, so the launch
 * decisions a spawn makes (provider, model, effort, permission mode) belong to
 * the person doing the handoff, not to a hardcoded default. Every knob starts
 * on the SOURCE session's own value — the common case is "same setup, fresh
 * context", and that must be zero clicks.
 *
 * Two things are chosen here and nothing else (a handoff is not a full spawn
 * screen — cwd, worktree, profiles and MCP loadout all follow the source):
 *  1. the successor's launch settings, and
 *  2. who writes the brief — the source agent itself (best: it holds the
 *     session in context, but it costs a turn) or claudemon's mechanical
 *     digest (instant, derived from the transcript).
 *
 * Confirming hands back to ClaudePane, which writes the brief and then spawns
 * through the normal workspacer spawn path (`spawnAgent` → `spawnClaude`) — the
 * same one the hub's `agents.spawn` capability drives.
 */

import React, { useEffect, useMemo, useState } from 'react';
import { ArrowRightLeft } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { AgentProvider } from '../../types/pane';
import { capsFor, effortLevelLabel, type EffortLevel } from '../../lib/providerCaps';
import { loadModelOptions, type ModelOption } from '../../lib/modelOptions';
import { deriveSessionStats } from '../../lib/sessionStats';
import { shortModelLabel } from '../../lib/modelLabel';
import { AgentLogo } from '../agentLogos';

/** What the dialog hands back — everything the successor spawns with. */
export interface HandoffSettings {
  provider: AgentProvider;
  /** Undefined = the provider's own default model. */
  model?: string;
  /** Undefined = the harness's configured default effort. */
  effort?: string;
  permissionMode: string;
  /** Legacy bypass boolean, kept in step with `permissionMode`. */
  skipPermissions: boolean;
  /** Who authors the handoff brief. */
  brief: 'agent' | 'mechanical';
}

const PROVIDERS: { value: AgentProvider; label: string }[] = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'opencode', label: 'OpenCode' },
  { value: 'pi', label: 'Pi' },
];

const isBypass = (mode: string): boolean => mode === 'bypassPermissions' || mode === 'yolo';

/** Bypass-everything mode id per provider family (claude vs managed). */
const bypassModeFor = (provider: AgentProvider): string =>
  provider === 'claude' ? 'bypassPermissions' : 'yolo';

/**
 * Carry a permission mode across a provider switch. The bypass-family ids
 * translate (bypassPermissions ↔ yolo — the same intent in two vocabularies);
 * anything the target provider doesn't offer — including live-telemetry-only
 * ids like 'auto'/'dontAsk', which no launch flag accepts — falls back to that
 * provider's first (safest) mode rather than reaching the CLI as invalid argv.
 */
export function carryPermissionMode(target: AgentProvider, mode: string | undefined): string {
  const caps = capsFor(target);
  const fallback = caps.permissionModes[0]?.id ?? 'default';
  const cur = (mode ?? '').trim();
  if (!cur) return fallback;
  if (caps.permissionModes.some((m) => m.id === cur)) return cur;
  if (isBypass(cur)) return bypassModeFor(target);
  return fallback;
}

export const HandoffDialog: React.FC<{
  /** The source session's provider — seeds the target and the settings. */
  provider: AgentProvider | undefined;
  /** Source session snapshot: every default is read from it. */
  snapshot?: ClaudeSessionSnapshot | null;
  /** Working directory; the successor spawns here too. */
  cwd?: string;
  /** Non-null while the brief is being written (the agent tier takes a turn). */
  busy: 'agent' | 'mechanical' | null;
  onCancel: () => void;
  onConfirm: (settings: HandoffSettings) => void;
}> = ({ provider, snapshot, cwd, busy, onCancel, onConfirm }) => {
  const sourceProvider = provider ?? 'claude';

  // ── The source session's current values: what every knob defaults to ──
  // Same precedence as the composer pills: live telemetry wins over what was
  // requested at spawn (the user may have switched model/mode mid-session, and
  // the successor should inherit what the session actually IS, not how it
  // started).
  const sourceModel = deriveSessionStats(snapshot).model ?? snapshot?.settings?.model ?? '';
  const sourceEffort =
    snapshot?.statusLine?.effort?.trim() ||
    snapshot?.liveEffort?.trim() ||
    snapshot?.settings?.effort?.trim() ||
    '';
  const sourceMode = snapshot?.livePermissionMode ?? snapshot?.settings?.permissionMode;

  const [target, setTarget] = useState<AgentProvider>(sourceProvider);
  const [model, setModel] = useState(sourceModel);
  const [permissionMode, setPermissionMode] = useState(() =>
    carryPermissionMode(sourceProvider, sourceMode),
  );
  // Effort is kept per provider: the ladders differ per harness, and a level
  // picked for one must not leak into another. Seeded with the source's level
  // for the source's harness.
  const [effortByProvider, setEffortByProvider] = useState<Partial<Record<AgentProvider, string>>>(
    () => ({ [sourceProvider]: sourceEffort }),
  );
  const [brief, setBrief] = useState<'agent' | 'mechanical'>('agent');
  const [models, setModels] = useState<ModelOption[]>([]);
  const [modelsLoading, setModelsLoading] = useState(true);

  const caps = capsFor(target);
  const effort = effortByProvider[target] ?? '';

  // Load the target provider's catalog. Re-runs on every provider switch: the
  // lists share no ids (a claude alias means nothing to codex).
  useEffect(() => {
    let cancelled = false;
    setModelsLoading(true);
    void loadModelOptions(target, capsFor(target).modelSource, cwd).then((list) => {
      if (cancelled) return;
      setModels(list);
      setModelsLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [target, cwd]);

  // Switching provider re-seeds the settings: the model id can't cross harnesses
  // (back on the source provider it returns to the source's model), and the
  // permission mode translates where it can. Effort needs no cleanup here —
  // effortByProvider already isolates each harness.
  const handleProvider = (next: AgentProvider) => {
    if (next === target) return;
    setTarget(next);
    setModel(next === sourceProvider ? sourceModel : '');
    setPermissionMode((cur) => carryPermissionMode(next, cur));
  };

  // Escape closes — unless a brief is in flight, where cancelling the dialog
  // would leave the agent writing a file for a handoff that isn't coming.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) {
        e.preventDefault();
        e.stopPropagation();
        onCancel();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onCancel, busy]);

  // The source's model may not be in the catalog (a concrete dated id, a model
  // the provider no longer lists). Keep it as a row of its own so the default
  // is never silently dropped to "provider default".
  const modelRows = useMemo(() => {
    if (!model || models.some((m) => m.id === model)) return models;
    return [{ id: model, label: shortModelLabel(model) || model }, ...models];
  }, [models, model]);

  // Codex reports supported efforts per model; prefer that over the provider
  // fallback so a level the selected model rejects is never offered.
  const selectedModel = models.find((m) => (model ? m.id === model : m.default));
  const effortLevels: EffortLevel[] =
    target === 'codex' && selectedModel?.effortLevels?.length
      ? selectedModel.effortLevels.map((id) => ({ id, label: effortLevelLabel(id) }))
      : (caps.effort?.levels ?? []);

  // A level the selected model doesn't support returns to the harness default
  // rather than being sent as invalid argv.
  const effortKey = effortLevels.map((l) => l.id).join('\0');
  useEffect(() => {
    const supported = effortKey ? effortKey.split('\0') : [];
    setEffortByProvider((cur) => {
      const picked = cur[target];
      if (!picked || supported.includes(picked)) return cur;
      return { ...cur, [target]: '' };
    });
  }, [target, effortKey]);

  const bypassSelected = isBypass(permissionMode);
  const providerLabel = PROVIDERS.find((p) => p.value === target)?.label ?? target;

  const submit = () => {
    if (busy) return;
    onConfirm({
      provider: target,
      model: model || undefined,
      effort: effort || undefined,
      permissionMode,
      skipPermissions: bypassSelected,
      brief,
    });
  };

  const rows: Array<{ key: string; label: string; title?: string; control: React.ReactNode }> = [
    {
      key: 'model',
      label: 'model',
      control: (
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          disabled={!!busy}
          style={rowSelect}
        >
          <option value="">
            {modelsLoading ? 'Loading models…' : `Default (${providerLabel} setting)`}
          </option>
          {modelRows.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
              {m.context ? ` · ${m.context}` : ''}
              {m.default ? '  — default' : ''}
            </option>
          ))}
        </select>
      ),
    },
  ];

  if (caps.effort) {
    rows.push({
      key: 'effort',
      label: 'effort',
      control: (
        <select
          value={effort}
          onChange={(e) => setEffortByProvider((cur) => ({ ...cur, [target]: e.target.value }))}
          disabled={!!busy}
          style={rowSelect}
        >
          <option value="">Default ({providerLabel} setting)</option>
          {effortLevels.map((l) => (
            <option key={l.id} value={l.id}>
              {l.label}
            </option>
          ))}
        </select>
      ),
    });
  }

  rows.push({
    key: 'permissions',
    label: 'permissions',
    control: (
      <select
        value={permissionMode}
        onChange={(e) => setPermissionMode(e.target.value)}
        disabled={!!busy}
        style={{ ...rowSelect, color: bypassSelected ? 'var(--wks-error)' : rowSelect.color }}
      >
        {caps.permissionModes.map((m) => (
          <option key={m.id} value={m.id}>
            {m.label}
          </option>
        ))}
      </select>
    ),
  });

  rows.push({
    key: 'brief',
    label: 'brief',
    title:
      brief === 'agent'
        ? 'The source agent writes the brief itself — it has the session in context (why things were done, dead ends, constraints), so its brief beats a transcript digest. Costs it one turn, and falls back to the digest if it does not deliver.'
        : "claudemon derives the brief from the conversation log — instant, and it doesn't disturb the source agent.",
    control: (
      <div style={{ ...segGroup, opacity: busy ? 0.6 : 1 }}>
        {(
          [
            { value: 'agent', label: 'this agent' },
            { value: 'mechanical', label: 'digest' },
          ] as const
        ).map((b) => (
          <button
            key={b.value}
            disabled={!!busy}
            onClick={() => setBrief(b.value)}
            style={segBtn(brief === b.value)}
          >
            {b.label}
          </button>
        ))}
      </div>
    ),
  });

  return (
    <div
      onClick={() => !busy && onCancel()}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 20000,
        background: 'var(--wks-overlay)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        animation: 'wks-fade-in 0.15s ease-out',
      }}
    >
      <div
        role="dialog"
        aria-label="Hand off session"
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 440,
          maxWidth: '92vw',
          maxHeight: '86vh',
          overflowY: 'auto',
          background: 'var(--wks-bg-raised)',
          borderRadius: 'var(--wks-radius-lg)',
          boxShadow: 'var(--wks-shadow)',
          padding: '20px 20px 16px',
          boxSizing: 'border-box',
        }}
      >
        {/* ── Header ─────────────────────────────────────────────────── */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <ArrowRightLeft size={15} strokeWidth={1.9} style={{ color: 'var(--wks-accent)' }} />
          <span style={{ fontSize: '1.05rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>
            Hand off session
          </span>
        </div>
        <div
          style={{
            marginTop: 6,
            fontSize: '0.72rem',
            lineHeight: 1.5,
            color: 'var(--wks-text-muted)',
          }}
        >
          Spawns a new agent in this directory, primed with a brief of the work so far. Settings
          start from this session.
        </div>

        {/* ── Target provider ────────────────────────────────────────── */}
        <div style={{ ...quietLabel, marginTop: 20 }}>hand off to</div>
        <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
          {PROVIDERS.map((p) => {
            const active = target === p.value;
            return (
              <button
                key={p.value}
                onClick={() => handleProvider(p.value)}
                disabled={!!busy}
                title={
                  p.value === sourceProvider
                    ? `${p.label} — same harness, fresh context`
                    : `Hand off to ${p.label}`
                }
                style={{
                  flex: 1,
                  minWidth: 0,
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  gap: 6,
                  padding: '10px 6px 8px',
                  borderRadius: 'var(--wks-radius-md)',
                  cursor: busy ? 'default' : 'pointer',
                  fontFamily: 'inherit',
                  border: active
                    ? '1px solid var(--wks-accent)'
                    : '1px solid var(--wks-border-input)',
                  background: active ? 'var(--wks-accent-bg)' : 'transparent',
                  opacity: busy ? 0.6 : 1,
                  transition: 'border-color 0.12s, background-color 0.12s',
                }}
              >
                <AgentLogo
                  provider={p.value}
                  size={18}
                  style={{
                    color: 'var(--wks-text-primary)',
                    opacity: active ? 1 : 0.65,
                    transition: 'opacity 0.12s',
                  }}
                />
                <span
                  style={{
                    fontSize: '0.66rem',
                    fontWeight: 600,
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    maxWidth: '100%',
                    color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                  }}
                >
                  {p.label}
                </span>
              </button>
            );
          })}
        </div>

        {/* ── Launch settings ────────────────────────────────────────── */}
        <div style={{ marginTop: 16 }}>
          {rows.map((row, i) => (
            <div
              key={row.key}
              title={row.title}
              style={{
                display: 'grid',
                gridTemplateColumns: '88px minmax(0, 1fr)',
                alignItems: 'center',
                gap: 12,
                padding: '10px 0',
                borderTop:
                  i > 0
                    ? '1px solid color-mix(in srgb, var(--wks-border-input) 55%, transparent)'
                    : 'none',
              }}
            >
              <span style={quietLabel}>{row.label}</span>
              <div style={{ minWidth: 0 }}>{row.control}</div>
            </div>
          ))}
        </div>

        {bypassSelected && (
          <div style={{ marginTop: 8, fontSize: '0.7rem', color: 'var(--wks-error)' }}>
            {target === 'claude'
              ? 'Dangerous — the successor bypasses all approval prompts.'
              : 'Dangerous — the successor auto-approves every command and file change.'}
          </div>
        )}

        {/* ── Actions ────────────────────────────────────────────────── */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'flex-end',
            gap: 10,
            marginTop: 18,
          }}
        >
          {busy && (
            <span
              style={{
                flex: 1,
                minWidth: 0,
                fontSize: '0.7rem',
                color: 'var(--wks-text-muted)',
              }}
            >
              {busy === 'agent'
                ? 'Waiting for the agent to write its brief…'
                : 'Writing the brief…'}
            </span>
          )}
          <button
            onClick={onCancel}
            disabled={!!busy}
            className="wks-composer-ctl"
            style={{
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 500,
              cursor: busy ? 'default' : 'pointer',
              background: 'transparent',
              color: busy ? 'var(--wks-text-disabled)' : 'var(--wks-text-muted)',
              border: 'none',
              borderRadius: 'var(--wks-radius-md)',
              padding: '8px 12px',
            }}
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={!!busy}
            style={{
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 600,
              cursor: busy ? 'default' : 'pointer',
              background: busy ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
              color: busy ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent)',
              border: 'none',
              borderRadius: 'var(--wks-radius-md)',
              padding: '8px 20px',
            }}
          >
            Hand off
          </button>
        </div>
      </div>
    </div>
  );
};

const quietLabel: React.CSSProperties = {
  fontSize: '0.6rem',
  fontWeight: 600,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: 'var(--wks-text-faint)',
  userSelect: 'none',
  whiteSpace: 'nowrap',
};

/** Flat, borderless select inside a labeled row — the value IS the control. */
const rowSelect: React.CSSProperties = {
  background: 'transparent',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm)',
  padding: '3px 2px',
  fontSize: '0.72rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  color: 'var(--wks-text-primary)',
  cursor: 'pointer',
  maxWidth: '100%',
  textOverflow: 'ellipsis',
};

/** Container for the two-option brief-author toggle. */
const segGroup: React.CSSProperties = {
  display: 'inline-flex',
  gap: 2,
  padding: 2,
  border: '1px solid var(--wks-border-input)',
  borderRadius: 'var(--wks-radius-pill)',
};

const segBtn = (active: boolean): React.CSSProperties => ({
  fontSize: '0.66rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: 'pointer',
  padding: '3px 10px',
  borderRadius: 'var(--wks-radius-pill)',
  border: 'none',
  background: active ? 'var(--wks-accent-bg)' : 'transparent',
  color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-muted)',
  transition: 'background-color 0.12s, color 0.12s',
});

export default HandoffDialog;
