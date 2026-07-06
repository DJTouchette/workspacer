import React, { useEffect, useState } from 'react';
import { deriveAgentName } from '../hooks/useAgentManager';
import { AgentLogo } from './agentLogos';
import type { LibraryItem } from '../types/library';
import type { AgentProvider } from '../types/pane';
import { capsFor } from '../lib/providerCaps';

/** Bypass-everything mode id per provider family (claude vs managed). */
const bypassModeFor = (provider: AgentProvider): string =>
  provider === 'claude' ? 'bypassPermissions' : 'yolo';

interface SpawnProfile {
  id: string;
  name: string;
  mcpItemIds?: string[];
}

interface SpawnAgentDialogProps {
  defaultCwd: string;
  /** Provider pre-selected in the picker (config.agents.defaultProvider). */
  defaultProvider?: AgentProvider;
  /** Claude transport pre-selected in the picker (config.claude.transport). */
  defaultTransport?: 'pty' | 'stream';
  onSpawn: (opts: {
    cwd: string;
    name?: string;
    provider?: AgentProvider;
    /** Claude only: 'pty' | 'stream'. */
    transport?: 'pty' | 'stream';
    profileId?: string;
    model?: string;
    effort?: string;
    permissionMode?: string;
    skipPermissions?: boolean;
    mcpItemIds?: string[];
    resumeSessionId?: string;
  }) => void;
  onCancel: () => void;
}

const CUSTOM = '__custom__';

const PROVIDERS: { value: AgentProvider; label: string; beta?: boolean }[] = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  // Not yet thoroughly tested — surfaced with a Beta badge so expectations are set.
  { value: 'opencode', label: 'OpenCode', beta: true },
  { value: 'pi', label: 'Pi', beta: true },
];

/** Free-text model placeholder per managed provider (their own id formats). */
function modelPlaceholder(provider: AgentProvider): string {
  switch (provider) {
    case 'codex':
      return 'gpt-5.4  (blank = Codex default)';
    case 'pi':
      return 'claude-sonnet-4 / gpt-5  (blank = Pi default)';
    default:
      return 'anthropic/claude-sonnet-4  (blank = OpenCode default)';
  }
}

/** Detection result for one provider (mirrors main-process ProviderStatus). */
interface ProviderDetection {
  provider: string;
  found: boolean;
  resolvedPath: string | null;
  customBin: string;
}

const SpawnAgentDialog: React.FC<SpawnAgentDialogProps> = ({
  defaultCwd,
  defaultProvider,
  defaultTransport,
  onSpawn,
  onCancel,
}) => {
  const [cwd, setCwd] = useState(defaultCwd);
  const [name, setName] = useState('');
  const [provider, setProvider] = useState<AgentProvider>(defaultProvider ?? 'claude');
  // Claude transport override for this spawn — pre-set from config.claude.transport.
  const [transport, setTransport] = useState<'pty' | 'stream'>(defaultTransport ?? 'pty');
  const [providerDetection, setProviderDetection] = useState<ProviderDetection[]>([]);
  const [customBinPath, setCustomBinPath] = useState('');
  const isClaude = provider === 'claude';
  // Model picker for non-Claude providers. The list is live-queried from the
  // provider's own CLI/server (codex/opencode/pi); `providerSel` is the dropdown
  // value (''=provider default, a model id, or CUSTOM), and `providerCustom`
  // holds the free-text id when CUSTOM — or whenever the live list is empty
  // (e.g. Pi with no authed providers), in which case the field is shown bare.
  const [providerModels, setProviderModels] = useState<
    Array<{ id: string; label: string; default: boolean }>
  >([]);
  const [providerModelsLoading, setProviderModelsLoading] = useState(false);
  const [providerSel, setProviderSel] = useState('');
  const [providerCustom, setProviderCustom] = useState('');
  const [profiles, setProfiles] = useState<SpawnProfile[]>([]);
  const [profileId, setProfileId] = useState<string>('');

  // MCP servers available in the Library, and the per-spawn selection. Pre-filled
  // from the chosen profile's default loadout; overridable here.
  const [mcpItems, setMcpItems] = useState<LibraryItem[]>([]);
  const [mcpSel, setMcpSel] = useState<string[]>([]);

  // Model selection. `modelSel` is the dropdown value (''=Default, an alias/id,
  // or the CUSTOM sentinel); `customModel` holds the free-text id when CUSTOM.
  const [aliases, setAliases] = useState<Array<{ value: string; label: string }>>([]);
  const [seen, setSeen] = useState<string[]>([]);
  const [modelSel, setModelSel] = useState<string>('');
  const [customModel, setCustomModel] = useState('');
  // Permission mode ('' = provider default: claude 'default', managed 'ask')
  // and reasoning effort ('' = provider default; codex only). See providerCaps.
  const [permissionMode, setPermissionMode] = useState('');
  const [effort, setEffort] = useState('');

  // Resume an existing Claude session in this cwd. ''=start fresh.
  const [sessions, setSessions] = useState<
    Array<{ sessionId: string; timestamp: string; summary: string }>
  >([]);
  const [resumeSessionId, setResumeSessionId] = useState('');

  useEffect(() => {
    setCwd(defaultCwd);
  }, [defaultCwd]);

  // Fetch provider detection status once on mount.
  useEffect(() => {
    window.electronAPI
      .providerCheckAll?.()
      .then((list) => setProviderDetection(list ?? []))
      .catch(() => {});
  }, []);

  // When switching provider, seed the custom-bin field from the detected config.
  useEffect(() => {
    const det = providerDetection.find((d) => d.provider === provider);
    setCustomBinPath(det?.customBin ?? '');
  }, [provider, providerDetection]);

  // Keep the permission mode valid across provider switches: bypass-family ids
  // translate (bypassPermissions ↔ yolo); anything else the new provider
  // doesn't offer resets to its default. Effort only exists on some providers.
  useEffect(() => {
    setPermissionMode((cur) => {
      if (!cur) return cur;
      if (capsFor(provider).permissionModes.some((m) => m.id === cur)) return cur;
      if (cur === 'bypassPermissions' || cur === 'yolo') return bypassModeFor(provider);
      return '';
    });
    if (!capsFor(provider).effort) setEffort('');
  }, [provider]);

  // Close on Escape regardless of which inner element has focus.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onCancel]);

  // Discover resumable sessions whenever the directory settles (debounced).
  useEffect(() => {
    const dir = cwd.trim();
    if (!dir) {
      setSessions([]);
      return;
    }
    let cancelled = false;
    const handle = setTimeout(() => {
      window.electronAPI
        .claudeListSessionsForDir?.(dir)
        .then((list) => {
          if (!cancelled) setSessions(list ?? []);
        })
        .catch(() => {
          if (!cancelled) setSessions([]);
        });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [cwd]);

  // If the picked session disappears from the list (cwd changed), reset to fresh.
  useEffect(() => {
    if (resumeSessionId && !sessions.some((s) => s.sessionId === resumeSessionId))
      setResumeSessionId('');
  }, [sessions, resumeSessionId]);

  useEffect(() => {
    window.electronAPI
      .claudeProfilesList?.()
      .then((list: any[]) => setProfiles(list ?? []))
      .catch(() => {});
    window.electronAPI
      .libraryList?.(defaultCwd || undefined)
      .then((list) => setMcpItems((list ?? []).filter((it) => it.kind === 'mcp')))
      .catch(() => {});
    window.electronAPI
      .claudeListModels?.()
      .then((res) => {
        if (!res) return;
        setAliases(res.aliases ?? []);
        setSeen(res.seen ?? []);
        if (res.skipPermissionsDefault === true) setPermissionMode(bypassModeFor(provider));
        // Pre-select the saved default. If it's a concrete id we don't have in
        // a list, keep it as a custom entry so the saved value isn't dropped.
        const d = res.defaultModel ?? '';
        const known =
          d === '' ||
          (res.aliases ?? []).some((a) => a.value === d) ||
          (res.seen ?? []).includes(d);
        if (known) {
          setModelSel(d);
        } else {
          setModelSel(CUSTOM);
          setCustomModel(d);
        }
      })
      .catch(() => {});
  }, []);

  // Live-query the model catalog when a managed provider is picked. Each lookup
  // boots the provider's CLI/server, so we key only on `provider` (not cwd) to
  // avoid re-spawning on every keystroke; the list is auth/global, not
  // cwd-specific. Resets the selection so a stale pick can't leak across
  // providers. An empty list (failure or no authed models) is fine — the field
  // falls back to free-text entry.
  useEffect(() => {
    if (isClaude) {
      setProviderModels([]);
      return;
    }
    setProviderSel('');
    setProviderCustom('');
    setProviderModels([]);
    setProviderModelsLoading(true);
    let cancelled = false;
    window.electronAPI
      .providerListModels?.(provider as 'codex' | 'opencode' | 'pi', cwd.trim() || undefined)
      .then((list) => {
        if (!cancelled) setProviderModels(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setProviderModels([]);
      })
      .finally(() => {
        if (!cancelled) setProviderModelsLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  // Re-list MCP servers when the directory settles — project-scoped servers
  // live under the chosen cwd's .workspacer/library.
  useEffect(() => {
    const dir = cwd.trim();
    let cancelled = false;
    const handle = setTimeout(() => {
      window.electronAPI
        .libraryList?.(dir || undefined)
        .then((list) => {
          if (!cancelled) setMcpItems((list ?? []).filter((it) => it.kind === 'mcp'));
        })
        .catch(() => {});
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [cwd]);

  // Pre-fill the MCP selection from the chosen profile's default loadout.
  useEffect(() => {
    const p = profiles.find((x) => x.id === profileId);
    setMcpSel(p?.mcpItemIds ?? []);
  }, [profileId, profiles]);

  // Drop any selected server that no longer exists (e.g. cwd changed).
  useEffect(() => {
    setMcpSel((sel) => sel.filter((id) => mcpItems.some((it) => it.id === id)));
  }, [mcpItems]);

  const toggleMcp = (id: string) =>
    setMcpSel((sel) => (sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id]));

  const resolvedModel = modelSel === CUSTOM ? customModel.trim() : modelSel;
  // For managed providers: the free-text custom field is the source whenever the
  // live list is empty or Custom… is chosen; otherwise it's the dropdown value.
  const resolvedProviderModel =
    providerModels.length === 0 || providerSel === CUSTOM ? providerCustom.trim() : providerSel;

  const browse = async () => {
    const picked = await window.electronAPI.pickFolder?.(cwd || undefined);
    if (picked) setCwd(picked);
  };

  const saveCustomBin = (value: string) => {
    const binaries = { [provider]: value.trim() };
    window.electronAPI
      .saveConfig?.({ agents: { binaries } } as any)
      .then(() =>
        window.electronAPI
          .providerCheckAll?.()
          .then((list) => setProviderDetection(list ?? []))
          .catch(() => {}),
      )
      .catch(() => {});
  };

  const browseCustomBin = async () => {
    const files = await window.electronAPI.pickFiles?.(undefined);
    if (files?.length) {
      setCustomBinPath(files[0]);
      saveCustomBin(files[0]);
    }
  };

  const currentDetection = providerDetection.find((d) => d.provider === provider);

  const submit = () => {
    if (!cwd.trim()) return;
    // '' means the provider's own default mode; the legacy boolean tracks the
    // bypass-family modes for back-compat consumers (saved defaults, respawn).
    const resolvedMode = permissionMode || (isClaude ? 'default' : 'ask');
    const skipPermissions = resolvedMode === 'bypassPermissions' || resolvedMode === 'yolo';
    // Claude-only options are dropped for other providers (they run their own
    // TUI in Tier-1 and don't take Claude's profile/model/MCP/resume flags).
    onSpawn(
      isClaude
        ? {
            cwd: cwd.trim(),
            name: name.trim() || undefined,
            transport,
            profileId: profileId || undefined,
            model: resolvedModel || undefined,
            permissionMode: resolvedMode,
            skipPermissions,
            mcpItemIds: mcpSel.length ? mcpSel : undefined,
            resumeSessionId: resumeSessionId || undefined,
          }
        : {
            cwd: cwd.trim(),
            name: name.trim() || undefined,
            provider,
            model: resolvedProviderModel || undefined,
            effort: effort || undefined,
            permissionMode: resolvedMode,
            skipPermissions,
          },
    );
  };

  const placeholderName = cwd.trim() ? deriveAgentName(cwd.trim()) : 'agent';

  return (
    <div
      onMouseDown={onCancel}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 20000,
        backgroundColor: 'var(--wks-overlay, rgba(0,0,0,0.5))',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={(e) => e.stopPropagation()}
        style={{
          width: 420,
          maxWidth: '90vw',
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: 20,
          boxShadow:
            '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
          fontFamily: 'inherit',
        }}
      >
        <div
          style={{
            fontSize: '0.9rem',
            fontWeight: 600,
            color: 'var(--wks-text-primary)',
            marginBottom: 16,
          }}
        >
          Spawn agent
        </div>

        <Field label="Working directory">
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              autoFocus
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') submit();
                if (e.key === 'Escape') onCancel();
              }}
              placeholder="/path/to/project"
              style={inputStyle}
            />
            <button onClick={browse} style={browseBtnStyle}>
              Browse…
            </button>
          </div>
        </Field>

        <Field label="Name (optional)">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit();
              if (e.key === 'Escape') onCancel();
            }}
            placeholder={placeholderName}
            style={inputStyle}
          />
        </Field>

        <Field label="Agent">
          <div style={{ display: 'flex', gap: 4 }}>
            {PROVIDERS.map((p) => {
              const active = provider === p.value;
              const det = providerDetection.find((d) => d.provider === p.value);
              const dotColor =
                det === undefined
                  ? 'var(--wks-text-disabled)'
                  : det.found
                    ? 'var(--wks-success)'
                    : 'var(--wks-danger, #e05555)';
              return (
                <button
                  key={p.value}
                  onClick={() => setProvider(p.value)}
                  title={
                    det
                      ? det.found
                        ? `Found: ${det.resolvedPath}`
                        : 'Not found on PATH'
                      : 'Checking…'
                  }
                  style={{
                    flex: 1,
                    padding: '6px 8px',
                    borderRadius: 6,
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    gap: 6,
                    fontSize: '0.72rem',
                    fontFamily: 'inherit',
                    fontWeight: 600,
                    border: active
                      ? '1px solid var(--wks-accent)'
                      : '1px solid var(--wks-border-input)',
                    background: active ? 'var(--wks-accent-bg)' : 'transparent',
                    color: active
                      ? 'var(--wks-accent-text, var(--wks-text-primary))'
                      : 'var(--wks-text-tertiary)',
                  }}
                >
                  <AgentLogo
                    provider={p.value}
                    size={14}
                    style={{ flexShrink: 0, opacity: active ? 1 : 0.75 }}
                  />
                  {p.label}
                  {p.beta && (
                    <span
                      title="Beta — not yet thoroughly tested"
                      style={{
                        fontSize: '0.5rem',
                        fontWeight: 700,
                        letterSpacing: '0.04em',
                        lineHeight: 1,
                        padding: '2px 3px',
                        borderRadius: 3,
                        flexShrink: 0,
                        color: 'var(--wks-warning, #e0a000)',
                        border: '1px solid var(--wks-warning, #e0a000)',
                        opacity: active ? 1 : 0.7,
                      }}
                    >
                      BETA
                    </span>
                  )}
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: '50%',
                      background: dotColor,
                      flexShrink: 0,
                    }}
                  />
                </button>
              );
            })}
          </div>

          {/* Detection status + custom binary path for the selected provider */}
          {currentDetection && (
            <div style={{ marginTop: 6 }}>
              {currentDetection.found ? (
                <div style={{ color: 'var(--wks-success)', fontSize: '0.62rem' }}>
                  ✓ {currentDetection.resolvedPath}
                </div>
              ) : (
                <div>
                  <div
                    style={{
                      color: 'var(--wks-danger, #e05555)',
                      fontSize: '0.62rem',
                      marginBottom: 4,
                    }}
                  >
                    Not found on PATH — set a custom path or install the CLI
                  </div>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <input
                      value={customBinPath}
                      onChange={(e) => setCustomBinPath(e.target.value)}
                      onBlur={(e) => saveCustomBin(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') saveCustomBin(customBinPath);
                        if (e.key === 'Escape') onCancel();
                      }}
                      placeholder={`/usr/local/bin/${provider}`}
                      spellCheck={false}
                      style={{ ...inputStyle, fontSize: '0.72rem' }}
                    />
                    <button onClick={browseCustomBin} style={browseBtnStyle}>
                      Browse…
                    </button>
                  </div>
                </div>
              )}
              {/* Let the user override even when auto-detected */}
              {currentDetection.found && (
                <details style={{ marginTop: 4 }}>
                  <summary
                    style={{
                      fontSize: '0.6rem',
                      color: 'var(--wks-text-faint)',
                      cursor: 'pointer',
                    }}
                  >
                    Override binary path…
                  </summary>
                  <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
                    <input
                      value={customBinPath}
                      onChange={(e) => setCustomBinPath(e.target.value)}
                      onBlur={(e) => saveCustomBin(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') saveCustomBin(customBinPath);
                        if (e.key === 'Escape') onCancel();
                      }}
                      placeholder={currentDetection.resolvedPath ?? ''}
                      spellCheck={false}
                      style={{ ...inputStyle, fontSize: '0.72rem' }}
                    />
                    <button onClick={browseCustomBin} style={browseBtnStyle}>
                      Browse…
                    </button>
                  </div>
                </details>
              )}
            </div>
          )}

          {!isClaude && (
            <div
              style={{
                color: 'var(--wks-text-faint)',
                fontSize: '0.62rem',
                marginTop: 5,
                lineHeight: 1.4,
              }}
            >
              Runs via claudemon's {PROVIDERS.find((p) => p.value === provider)?.label} adapter —
              conversation and usage stream into the agent view. Approvals are auto-accepted for
              now.
            </div>
          )}
        </Field>

        {!isClaude && (
          <Field label="Model (optional)">
            {providerModels.length > 0 ? (
              <>
                <select
                  value={providerSel}
                  onChange={(e) => setProviderSel(e.target.value)}
                  style={inputStyle}
                >
                  <option value="">
                    Default ({PROVIDERS.find((p) => p.value === provider)?.label} setting)
                  </option>
                  <optgroup label="Available">
                    {providerModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                        {m.default ? '  — default' : ''}
                      </option>
                    ))}
                  </optgroup>
                  <option value={CUSTOM}>Custom…</option>
                </select>
                {providerSel === CUSTOM && (
                  <input
                    value={providerCustom}
                    onChange={(e) => setProviderCustom(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') submit();
                      if (e.key === 'Escape') onCancel();
                    }}
                    placeholder={modelPlaceholder(provider)}
                    style={{ ...inputStyle, marginTop: 6 }}
                  />
                )}
              </>
            ) : (
              <input
                value={providerCustom}
                onChange={(e) => setProviderCustom(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') submit();
                  if (e.key === 'Escape') onCancel();
                }}
                placeholder={providerModelsLoading ? 'Loading models…' : modelPlaceholder(provider)}
                style={inputStyle}
              />
            )}
          </Field>
        )}

        {isClaude && (
          <Field label="Transport">
            <div style={{ display: 'flex', gap: 4 }}>
              {(
                [
                  { value: 'pty', label: 'Terminal (PTY)' },
                  { value: 'stream', label: 'Headless (stream)' },
                ] as const
              ).map((t) => {
                const active = transport === t.value;
                return (
                  <button
                    key={t.value}
                    onClick={() => setTransport(t.value)}
                    style={{
                      flex: 1,
                      padding: '6px 8px',
                      borderRadius: 6,
                      cursor: 'pointer',
                      fontSize: '0.72rem',
                      fontFamily: 'inherit',
                      fontWeight: 600,
                      border: active
                        ? '1px solid var(--wks-accent)'
                        : '1px solid var(--wks-border-input)',
                      background: active ? 'var(--wks-accent-bg)' : 'transparent',
                      color: active
                        ? 'var(--wks-accent-text, var(--wks-text-primary))'
                        : 'var(--wks-text-tertiary)',
                    }}
                  >
                    {t.label}
                  </button>
                );
              })}
            </div>
            <div
              style={{
                color: 'var(--wks-text-faint)',
                fontSize: '0.62rem',
                marginTop: 5,
                lineHeight: 1.4,
              }}
            >
              {transport === 'pty'
                ? 'The classic Claude Code TUI in a terminal — Term and GUI views.'
                : 'Headless stream-json via claudemon — structured GUI only, no terminal view.'}
            </div>
          </Field>
        )}

        {isClaude && sessions.length > 0 && (
          <Field label="Resume session (optional)">
            <select
              value={resumeSessionId}
              onChange={(e) => setResumeSessionId(e.target.value)}
              style={inputStyle}
            >
              <option value="">Start fresh</option>
              {sessions.map((s) => (
                <option key={s.sessionId} value={s.sessionId}>
                  {relTime(s.timestamp)} — {s.summary}
                </option>
              ))}
            </select>
          </Field>
        )}

        {isClaude && (
          <Field label="Model">
            <select
              value={modelSel}
              onChange={(e) => setModelSel(e.target.value)}
              style={inputStyle}
            >
              <option value="">Default (Claude Code setting)</option>
              {aliases.length > 0 && (
                <optgroup label="Latest">
                  {aliases.map((a) => (
                    <option key={a.value} value={a.value}>
                      {a.label}
                    </option>
                  ))}
                </optgroup>
              )}
              {seen.length > 0 && (
                <optgroup label="Seen in sessions">
                  {seen.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </optgroup>
              )}
              <option value={CUSTOM}>Custom…</option>
            </select>
            {modelSel === CUSTOM && (
              <input
                value={customModel}
                onChange={(e) => setCustomModel(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') submit();
                  if (e.key === 'Escape') onCancel();
                }}
                placeholder="claude-opus-4-8  or  opus"
                style={{ ...inputStyle, marginTop: 6 }}
              />
            )}
          </Field>
        )}

        {isClaude && profiles.length > 0 && (
          <Field label="Profile (optional)">
            <select
              value={profileId}
              onChange={(e) => setProfileId(e.target.value)}
              style={inputStyle}
            >
              <option value="">Default</option>
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </Field>
        )}

        {isClaude && mcpItems.length > 0 && (
          <Field label="MCP servers (optional)">
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 2,
                maxHeight: 132,
                overflowY: 'auto',
                border: '1px solid var(--wks-border-input)',
                borderRadius: 4,
                padding: 4,
              }}
            >
              {mcpItems.map((it) => (
                <label
                  key={it.id}
                  title={it.description || it.id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '4px 6px',
                    borderRadius: 4,
                    cursor: 'pointer',
                    fontSize: '0.75rem',
                    color: 'var(--wks-text-primary)',
                  }}
                >
                  <input
                    type="checkbox"
                    checked={mcpSel.includes(it.id)}
                    onChange={() => toggleMcp(it.id)}
                    style={{ cursor: 'pointer' }}
                  />
                  <span
                    style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                  >
                    {it.title}
                  </span>
                  <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)' }}>
                    {it.mcp?.url ? (it.mcp.type === 'sse' ? 'sse' : 'http') : 'stdio'}
                  </span>
                </label>
              ))}
            </div>
            <div style={{ color: 'var(--wks-text-faint)', fontSize: '0.62rem', marginTop: 3 }}>
              Only the checked servers are exposed to this session (--strict-mcp-config).
            </div>
          </Field>
        )}

        {capsFor(provider).effort && (
          <Field label="Reasoning effort">
            <select value={effort} onChange={(e) => setEffort(e.target.value)} style={inputStyle}>
              <option value="">Default</option>
              {capsFor(provider).effort!.levels.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.label}
                </option>
              ))}
            </select>
          </Field>
        )}

        <Field label="Permissions">
          <select
            value={permissionMode}
            onChange={(e) => setPermissionMode(e.target.value)}
            style={inputStyle}
          >
            {capsFor(provider).permissionModes.map((m, i) => (
              <option key={m.id} value={i === 0 ? '' : m.id}>
                {m.label}
              </option>
            ))}
          </select>
          {(permissionMode === 'bypassPermissions' || permissionMode === 'yolo') && (
            <div style={{ color: 'var(--wks-danger, #e05555)', fontSize: '0.65rem', marginTop: 3 }}>
              {isClaude
                ? 'Dangerous — bypasses all approval prompts (--dangerously-skip-permissions).'
                : 'Dangerous — auto-approves every command and file change, no prompts.'}
            </div>
          )}
        </Field>

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20 }}>
          <button onClick={onCancel} style={secondaryBtnStyle}>
            Cancel
          </button>
          <button onClick={submit} disabled={!cwd.trim()} style={primaryBtnStyle(!cwd.trim())}>
            Spawn
          </button>
        </div>
      </div>
    </div>
  );
};

/** Compact relative time for the resume picker, e.g. "2h ago", "3d ago". */
function relTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return 'just now';
  const m = s / 60;
  if (m < 60) return `${Math.floor(m)}m ago`;
  const h = m / 60;
  if (h < 24) return `${Math.floor(h)}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{ fontSize: '0.65rem', color: 'var(--wks-text-muted)', marginBottom: 4 }}>
        {label}
      </div>
      {children}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  flex: 1,
  width: '100%',
  boxSizing: 'border-box',
  fontSize: '0.8rem',
  fontFamily: 'inherit',
  background: 'var(--wks-bg-base)',
  color: 'var(--wks-text-primary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 8px',
};

const browseBtnStyle: React.CSSProperties = {
  fontSize: '0.75rem',
  fontFamily: 'inherit',
  cursor: 'pointer',
  background: 'var(--wks-bg-input)',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '0 10px',
  whiteSpace: 'nowrap',
};

const secondaryBtnStyle: React.CSSProperties = {
  fontSize: '0.78rem',
  fontFamily: 'inherit',
  cursor: 'pointer',
  background: 'transparent',
  color: 'var(--wks-text-tertiary)',
  border: '1px solid var(--wks-border-input)',
  borderRadius: 4,
  padding: '6px 14px',
};

const primaryBtnStyle = (disabled: boolean): React.CSSProperties => ({
  fontSize: '0.78rem',
  fontFamily: 'inherit',
  cursor: disabled ? 'default' : 'pointer',
  background: disabled ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
  color: disabled ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
  border: 'none',
  borderRadius: 4,
  padding: '6px 14px',
  fontWeight: 600,
});

export default SpawnAgentDialog;
