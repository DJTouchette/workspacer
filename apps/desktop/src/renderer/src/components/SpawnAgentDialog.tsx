import React, { useEffect, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { deriveAgentName } from '../hooks/useAgentManager';
import { AgentLogo } from './agentLogos';
import type { LibraryItem } from '../types/library';
import type { AgentProvider } from '../types/pane';
import { capsFor, effortLevelLabel, type EffortLevel } from '../lib/providerCaps';

/** Bypass-everything mode id per provider family (claude vs managed). */
const bypassModeFor = (provider: AgentProvider): string =>
  provider === 'claude' ? 'bypassPermissions' : 'yolo';

/** Provider defaults are rendered as the empty select value in the spawn UI. */
const defaultModeFor = (provider: AgentProvider): string =>
  provider === 'claude' ? 'default' : 'ask';

function normalizePermissionModeForProvider(provider: AgentProvider, mode: string): string {
  const cur = mode.trim();
  if (!cur || cur === defaultModeFor(provider)) return '';
  if (capsFor(provider).permissionModes.some((m) => m.id === cur)) return cur;
  if (cur === 'bypassPermissions' || cur === 'yolo') return bypassModeFor(provider);
  return '';
}

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
  /** Pre-check the git-worktree toggle (config.agents.spawnInWorktree). */
  defaultWorktree?: boolean;
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
    /** Spawn into a fresh git worktree of `cwd` instead of `cwd` itself. */
    worktree?: boolean;
  }) => void;
  onCancel: () => void;
}

const CUSTOM = '__custom__';
const ADVANCED_OPEN_KEY = 'workspacer.spawn.advancedOpen';

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

interface ProviderModel {
  id: string;
  label: string;
  default: boolean;
  /** Exact effort ids reported for this model by the provider catalog. */
  effortLevels?: string[];
}

/**
 * The "new agent" screen. Despite the (legacy) name it renders as a full-bleed
 * workspace page — a blank agent about to be born — not a floating modal:
 * provider mark up top, a hero working-directory input, provider logo cards,
 * then the remaining knobs in a collapsible "advanced" card of labeled rows.
 */
const SpawnAgentDialog: React.FC<SpawnAgentDialogProps> = ({
  defaultCwd,
  defaultProvider,
  defaultTransport,
  defaultWorktree,
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
  const [providerModels, setProviderModels] = useState<ProviderModel[]>([]);
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
  const [aliases, setAliases] = useState<Array<{ value: string; label: string; context?: string }>>(
    [],
  );
  const [seen, setSeen] = useState<string[]>([]);
  const [modelSel, setModelSel] = useState<string>('');
  const [customModel, setCustomModel] = useState('');
  // Permission mode ('' = provider default: claude 'default', managed 'ask').
  // Effort is kept per provider: Claude and Codex have different ladders and a
  // choice made for one harness must not leak into the other. An empty entry
  // means that harness's own configured default. See providerCaps.
  const [permissionMode, setPermissionMode] = useState('');
  const [effortByProvider, setEffortByProvider] = useState<Partial<Record<AgentProvider, string>>>(
    {},
  );
  const effort = effortByProvider[provider] ?? '';
  const setEffort = (value: string) =>
    setEffortByProvider((current) => ({ ...current, [provider]: value }));
  const [advancedOpen, setAdvancedOpen] = useState(() => {
    try {
      return window.localStorage?.getItem(ADVANCED_OPEN_KEY) === 'true';
    } catch {
      return false;
    }
  });

  // Hero input focus — drives the underline accent (no :focus-within inline).
  const [cwdFocus, setCwdFocus] = useState(false);

  // Resume an existing Claude session in this cwd. ''=start fresh.
  const [sessions, setSessions] = useState<
    Array<{ sessionId: string; timestamp: string; summary: string }>
  >([]);
  const [resumeSessionId, setResumeSessionId] = useState('');
  // Git worktree isolation: available when the cwd is a git repo (host only —
  // the web mirror can't shell out, so the pill hides there).
  const [useWorktree, setUseWorktree] = useState(!!defaultWorktree);
  const [repoInfo, setRepoInfo] = useState<{ isRepo: boolean; branch?: string } | null>(null);
  useEffect(() => {
    const dir = cwd.trim();
    if (!dir || !window.electronAPI.worktreeInfo) {
      setRepoInfo(null);
      return;
    }
    let cancelled = false;
    window.electronAPI
      .worktreeInfo(dir)
      .then((info) => {
        if (!cancelled) setRepoInfo(info);
      })
      .catch(() => {
        if (!cancelled) setRepoInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, [cwd]);
  const worktreeEligible = !!repoInfo?.isRepo && !resumeSessionId;

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
  // doesn't offer resets to its default. Effort needs no switch-time cleanup:
  // effortByProvider isolates each harness and effort-less providers hide it.
  useEffect(() => {
    setPermissionMode((cur) => {
      return normalizePermissionModeForProvider(provider, cur);
    });
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
        // Seed the permission pill from the last spawn's saved mode, but only
        // when it's valid for the pre-selected provider (the saved value is a
        // Claude-family mode; a managed provider keeps its own default). The
        // bypass default (below) still wins so an explicit "Full access" default
        // isn't lost.
        if (
          res.defaultPermissionMode &&
          capsFor(provider).permissionModes.some((m) => m.id === res.defaultPermissionMode)
        )
          setPermissionMode(
            normalizePermissionModeForProvider(provider, res.defaultPermissionMode),
          );
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
  const selectedProviderModel = providerModels.find((model) =>
    resolvedProviderModel ? model.id === resolvedProviderModel : model.default,
  );
  // Codex reports supported reasoning efforts per model. Prefer that live
  // catalog over the provider fallback so, for example, xhigh appears only
  // when the selected model accepts it. Claude currently reports one
  // harness-wide ladder through its CLI, so it keeps providerCaps' list.
  const effortLevels: EffortLevel[] =
    selectedProviderModel?.effortLevels?.length && provider === 'codex'
      ? selectedProviderModel.effortLevels.map((id) => ({ id, label: effortLevelLabel(id) }))
      : (capsFor(provider).effort?.levels ?? []);
  const effortLevelKey = effortLevels.map((level) => level.id).join('\0');

  // If changing models makes the current effort invalid, return this harness
  // to its default instead of sending a value the selected model rejects.
  useEffect(() => {
    const supported = effortLevelKey ? effortLevelKey.split('\0') : [];
    setEffortByProvider((current) => {
      const selected = current[provider];
      if (!selected || supported.includes(selected)) return current;
      return { ...current, [provider]: '' };
    });
  }, [provider, effortLevelKey]);

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
    const resolvedMode = permissionMode || defaultModeFor(provider);
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
            effort: effort || undefined,
            permissionMode: resolvedMode,
            skipPermissions,
            mcpItemIds: mcpSel.length ? mcpSel : undefined,
            resumeSessionId: resumeSessionId || undefined,
            worktree: useWorktree && worktreeEligible ? true : undefined,
          }
        : {
            cwd: cwd.trim(),
            name: name.trim() || undefined,
            provider,
            // Codex only: 'stream' spawns headless (its default is hybrid, so
            // only the non-default is worth sending).
            transport: provider === 'codex' && transport === 'stream' ? transport : undefined,
            model: resolvedProviderModel || undefined,
            effort: effort || undefined,
            permissionMode: resolvedMode,
            skipPermissions,
            worktree: useWorktree && worktreeEligible ? true : undefined,
          },
    );
  };

  const placeholderName = cwd.trim() ? deriveAgentName(cwd.trim()) : 'agent';
  const providerLabel = PROVIDERS.find((p) => p.value === provider)?.label ?? provider;
  const bypassSelected = permissionMode === 'bypassPermissions' || permissionMode === 'yolo';
  // Shared by the advanced header button and its attached panel so the two
  // read as one card even when the bypass tint is on.
  const advBorderColor = bypassSelected
    ? 'color-mix(in srgb, var(--wks-danger, #e05555) 55%, var(--wks-border-input))'
    : 'var(--wks-border-input)';
  const permissionSummary = bypassSelected ? 'full access' : 'approval prompts';
  const modelSummary = isClaude
    ? resolvedModel || 'default model'
    : resolvedProviderModel || 'default model';
  const transportSummary =
    isClaude || provider === 'codex'
      ? transport === 'pty'
        ? isClaude
          ? 'terminal'
          : 'hybrid'
        : 'headless'
      : null;
  const advancedSummary = [
    modelSummary,
    transportSummary,
    permissionSummary,
    useWorktree && worktreeEligible ? 'isolated worktree' : null,
    resumeSessionId ? 'resume' : null,
    profileId ? 'profile' : null,
    mcpSel.length ? `${mcpSel.length} MCP` : null,
    effort ? `${effort} effort` : null,
  ]
    .filter(Boolean)
    .join(' · ');

  const toggleAdvanced = () => {
    setAdvancedOpen((open) => {
      const next = !open;
      try {
        window.localStorage?.setItem(ADVANCED_OPEN_KEY, String(next));
      } catch {
        // Ignore private-mode storage failures; the in-memory state still works.
      }
      return next;
    });
  };

  // Enter/Escape on the text inputs — same behavior as the old modal.
  const keySubmit = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') submit();
    if (e.key === 'Escape') onCancel();
  };

  // ── Advanced options as labeled rows — only the relevant knobs appear ─────
  interface AdvRow {
    key: string;
    label: string;
    title?: string;
    control: React.ReactNode;
  }
  const rows: AdvRow[] = [];

  if (isClaude) {
    rows.push({
      key: 'model',
      label: 'model',
      control: (
        <>
          <select value={modelSel} onChange={(e) => setModelSel(e.target.value)} style={rowSelect}>
            <option value="">Default (Claude Code setting)</option>
            {aliases.length > 0 && (
              <optgroup label="Latest">
                {aliases.map((a) => (
                  <option key={a.value} value={a.value}>
                    {a.label}
                    {a.context ? ` · ${a.context}` : ''}
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
              onKeyDown={keySubmit}
              placeholder="claude-opus-4-8  or  opus"
              spellCheck={false}
              style={{ ...inlineInput, display: 'block', marginTop: 8, width: 260 }}
            />
          )}
        </>
      ),
    });
  } else if (providerModels.length > 0) {
    rows.push({
      key: 'model',
      label: 'model',
      control: (
        <>
          <select
            value={providerSel}
            onChange={(e) => setProviderSel(e.target.value)}
            style={rowSelect}
          >
            <option value="">Default ({providerLabel} setting)</option>
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
              onKeyDown={keySubmit}
              placeholder={modelPlaceholder(provider)}
              spellCheck={false}
              style={{ ...inlineInput, display: 'block', marginTop: 8, width: 300 }}
            />
          )}
        </>
      ),
    });
  } else {
    rows.push({
      key: 'model',
      label: 'model',
      control: (
        <input
          value={providerCustom}
          onChange={(e) => setProviderCustom(e.target.value)}
          onKeyDown={keySubmit}
          placeholder={providerModelsLoading ? 'Loading models…' : modelPlaceholder(provider)}
          spellCheck={false}
          style={{ ...inlineInput, width: '100%', maxWidth: 300 }}
        />
      ),
    });
  }

  if (capsFor(provider).effort) {
    rows.push({
      key: 'effort',
      label: 'effort',
      control: (
        <select value={effort} onChange={(e) => setEffort(e.target.value)} style={rowSelect}>
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

  if (isClaude || provider === 'codex') {
    rows.push({
      key: 'transport',
      label: 'transport',
      title:
        transport === 'pty'
          ? isClaude
            ? 'The classic Claude Code TUI in a terminal — Term and GUI views.'
            : 'Hybrid: the native Codex TUI in a terminal plus the structured GUI, one shared thread.'
          : isClaude
            ? 'Headless stream-json via claudemon — structured GUI only, no terminal view.'
            : 'Headless app-server via claudemon — structured GUI only, no terminal view.',
      control: (
        <div style={segGroup}>
          {(
            [
              { value: 'pty', label: isClaude ? 'terminal' : 'hybrid' },
              { value: 'stream', label: 'headless' },
            ] as const
          ).map((t) => (
            <button
              key={t.value}
              onClick={() => setTransport(t.value)}
              style={segBtn(transport === t.value)}
            >
              {t.label}
            </button>
          ))}
        </div>
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
        style={{
          ...rowSelect,
          color: bypassSelected ? 'var(--wks-danger, #e05555)' : rowSelect.color,
        }}
      >
        {capsFor(provider).permissionModes.map((m, i) => (
          <option key={m.id} value={i === 0 ? '' : m.id}>
            {m.label}
          </option>
        ))}
      </select>
    ),
  });

  if (window.electronAPI.worktreeCreate) {
    rows.push({
      key: 'worktree',
      label: 'worktree',
      title: !repoInfo?.isRepo
        ? 'Not a git repository — worktree isolation needs one.'
        : resumeSessionId
          ? "Resuming reuses the session's original directory."
          : `Run this agent in a fresh git worktree (a new branch cut from ${
              repoInfo.branch ?? 'HEAD'
            }, under ~/.workspacer/worktrees) so parallel agents in this repo never collide. Everything scoped to the agent — plugins, watchers, checks — follows the worktree.`,
      control: (
        <div style={{ ...segGroup, opacity: worktreeEligible ? 1 : 0.5 }}>
          {(
            [
              { value: false, label: 'repo directory' },
              { value: true, label: 'isolated worktree' },
            ] as const
          ).map((w) => (
            <button
              key={String(w.value)}
              disabled={!worktreeEligible}
              onClick={() => setUseWorktree(w.value)}
              style={{
                ...segBtn((useWorktree && worktreeEligible) === w.value),
                cursor: worktreeEligible ? 'pointer' : 'default',
              }}
            >
              {w.label}
            </button>
          ))}
        </div>
      ),
    });
  }

  if (isClaude && sessions.length > 0) {
    rows.push({
      key: 'resume',
      label: 'resume',
      control: (
        <select
          value={resumeSessionId}
          onChange={(e) => setResumeSessionId(e.target.value)}
          style={rowSelect}
        >
          <option value="">Start fresh</option>
          {sessions.map((s) => (
            <option key={s.sessionId} value={s.sessionId}>
              {relTime(s.timestamp)} — {s.summary}
            </option>
          ))}
        </select>
      ),
    });
  }

  if (isClaude && profiles.length > 0) {
    rows.push({
      key: 'profile',
      label: 'profile',
      control: (
        <select value={profileId} onChange={(e) => setProfileId(e.target.value)} style={rowSelect}>
          <option value="">Default</option>
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      ),
    });
  }

  if (isClaude && mcpItems.length > 0) {
    rows.push({
      key: 'mcp',
      label: 'mcp',
      control: (
        <div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {mcpItems.map((it) => {
              const on = mcpSel.includes(it.id);
              return (
                <button
                  key={it.id}
                  onClick={() => toggleMcp(it.id)}
                  title={it.description || it.id}
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.68rem',
                    fontWeight: 500,
                    fontFamily: 'inherit',
                    padding: '4px 11px',
                    borderRadius: 'var(--wks-radius-pill, 999px)',
                    cursor: 'pointer',
                    maxWidth: 220,
                    border: on
                      ? '1px solid var(--wks-accent)'
                      : '1px solid var(--wks-border-input)',
                    background: on ? 'var(--wks-accent-bg)' : 'transparent',
                    color: on
                      ? 'var(--wks-accent-text, var(--wks-text-primary))'
                      : 'var(--wks-text-tertiary)',
                    transition: 'border-color 0.15s, color 0.15s',
                  }}
                >
                  <span
                    style={{
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {it.title}
                  </span>
                  <span style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)' }}>
                    {it.mcp?.url ? (it.mcp.type === 'sse' ? 'sse' : 'http') : 'stdio'}
                  </span>
                </button>
              );
            })}
          </div>
          <div style={{ color: 'var(--wks-text-faint)', fontSize: '0.6rem', marginTop: 6 }}>
            Only the checked servers are exposed to this session (--strict-mcp-config).
          </div>
        </div>
      ),
    });
  }

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 20000,
        background: 'var(--wks-bg-base)',
        overflow: 'hidden',
        animation: 'wks-fade-in 0.25s ease-out',
      }}
    >
      {/* Soft accent glow behind the centerpiece — pure decoration */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: '-18%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: 720,
          height: 720,
          borderRadius: '50%',
          background:
            'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 8%, transparent) 0%, transparent 65%)',
          pointerEvents: 'none',
        }}
      />

      <div style={{ position: 'relative', height: '100%', overflowY: 'auto' }}>
        <div
          style={{
            minHeight: '100%',
            maxWidth: 660,
            margin: '0 auto',
            padding: '9vh 32px 40px',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            boxSizing: 'border-box',
          }}
        >
          {/* ── Centerpiece: the agent about to be born ─────────────────── */}
          <div
            style={{
              width: 64,
              height: 64,
              borderRadius: '50%',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              border: '1px solid var(--wks-border-input)',
              background: 'color-mix(in srgb, var(--wks-accent) 5%, transparent)',
              color: 'var(--wks-text-primary)',
            }}
          >
            <AgentLogo provider={provider} size={30} />
          </div>
          <div
            style={{
              marginTop: 16,
              fontSize: '1.05rem',
              fontWeight: 650,
              letterSpacing: '-0.01em',
              color: 'var(--wks-text-primary)',
            }}
          >
            New agent
          </div>
          <div style={{ marginTop: 5, fontSize: '0.72rem', color: 'var(--wks-text-muted)' }}>
            Give it a home directory and set it loose.
          </div>

          {/* ── Hero: working directory ─────────────────────────────────── */}
          <div style={{ width: '100%', maxWidth: 560, marginTop: 40 }}>
            <div style={quietLabel}>working directory</div>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                marginTop: 6,
                paddingBottom: 7,
                borderBottom: `1px solid ${
                  cwdFocus ? 'var(--wks-accent)' : 'var(--wks-border-input)'
                }`,
                transition: 'border-color 0.15s',
              }}
            >
              <span
                aria-hidden
                style={{
                  fontFamily: 'var(--wks-font-mono)',
                  fontSize: '0.95rem',
                  color: cwdFocus ? 'var(--wks-accent)' : 'var(--wks-text-faint)',
                  userSelect: 'none',
                  transition: 'color 0.15s',
                }}
              >
                ❯
              </span>
              <input
                autoFocus
                value={cwd}
                onChange={(e) => setCwd(e.target.value)}
                onKeyDown={keySubmit}
                onFocus={() => setCwdFocus(true)}
                onBlur={() => setCwdFocus(false)}
                placeholder="/path/to/project"
                spellCheck={false}
                style={{
                  flex: 1,
                  minWidth: 0,
                  background: 'transparent',
                  border: 'none',
                  outline: 'none',
                  fontFamily: 'var(--wks-font-mono)',
                  fontSize: '0.98rem',
                  color: 'var(--wks-text-primary)',
                  padding: 0,
                }}
              />
              <button onClick={browse} className="wks-composer-ctl" style={ghostBtnSmall}>
                Browse…
              </button>
            </div>

            {/* Optional name — a ghost input, not a form row */}
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={keySubmit}
              placeholder={`name it (optional) · ${placeholderName}`}
              spellCheck={false}
              style={{
                marginTop: 10,
                width: '100%',
                background: 'transparent',
                border: 'none',
                outline: 'none',
                fontFamily: 'var(--wks-font-mono)',
                fontSize: '0.72rem',
                color: 'var(--wks-text-tertiary)',
                padding: 0,
              }}
            />
          </div>

          {/* ── Provider: a row of logo cards ───────────────────────────── */}
          <div
            style={{
              display: 'flex',
              gap: 8,
              marginTop: 34,
              width: '100%',
              maxWidth: 560,
              justifyContent: 'center',
            }}
          >
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
                    maxWidth: 136,
                    position: 'relative',
                    display: 'flex',
                    flexDirection: 'column',
                    alignItems: 'center',
                    gap: 7,
                    padding: '14px 8px 11px',
                    borderRadius: 'var(--wks-radius-md, 8px)',
                    cursor: 'pointer',
                    fontFamily: 'inherit',
                    border: active
                      ? '1px solid var(--wks-accent)'
                      : '1px solid var(--wks-border-input)',
                    background: active ? 'var(--wks-accent-bg)' : 'transparent',
                    transition: 'border-color 0.15s, background-color 0.15s',
                  }}
                >
                  <span
                    aria-hidden
                    style={{
                      position: 'absolute',
                      top: 8,
                      right: 8,
                      width: 6,
                      height: 6,
                      borderRadius: '50%',
                      background: dotColor,
                    }}
                  />
                  <AgentLogo
                    provider={p.value}
                    size={20}
                    style={{
                      color: 'var(--wks-text-primary)',
                      opacity: active ? 1 : 0.65,
                      transition: 'opacity 0.15s',
                    }}
                  />
                  <span
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: 5,
                      fontSize: '0.68rem',
                      fontWeight: 600,
                      color: active
                        ? 'var(--wks-accent-text, var(--wks-text-primary))'
                        : 'var(--wks-text-tertiary)',
                    }}
                  >
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
                          color: 'var(--wks-warning, #e0a000)',
                          border: '1px solid var(--wks-warning, #e0a000)',
                          opacity: active ? 1 : 0.7,
                        }}
                      >
                        BETA
                      </span>
                    )}
                  </span>
                </button>
              );
            })}
          </div>

          {/* Detection status + custom binary for the selected provider */}
          {currentDetection && (
            <div style={{ width: '100%', maxWidth: 560, marginTop: 10, textAlign: 'center' }}>
              {currentDetection.found ? (
                <>
                  <div
                    style={{
                      color: 'var(--wks-text-faint)',
                      fontSize: '0.62rem',
                      fontFamily: 'var(--wks-font-mono)',
                    }}
                  >
                    <span style={{ color: 'var(--wks-success)' }}>✓</span>{' '}
                    {currentDetection.resolvedPath}
                  </div>
                  <details style={{ marginTop: 4 }}>
                    <summary
                      style={{
                        fontSize: '0.6rem',
                        color: 'var(--wks-text-faint)',
                        cursor: 'pointer',
                        listStylePosition: 'inside',
                      }}
                    >
                      custom binary…
                    </summary>
                    <div
                      style={{
                        display: 'flex',
                        gap: 8,
                        marginTop: 6,
                        justifyContent: 'center',
                        alignItems: 'center',
                      }}
                    >
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
                        style={{ ...inlineInput, flex: 1, maxWidth: 380 }}
                      />
                      <button
                        onClick={browseCustomBin}
                        className="wks-composer-ctl"
                        style={ghostBtnSmall}
                      >
                        Browse…
                      </button>
                    </div>
                  </details>
                </>
              ) : (
                <div>
                  <div
                    style={{
                      color: 'var(--wks-danger, #e05555)',
                      fontSize: '0.62rem',
                      marginBottom: 6,
                    }}
                  >
                    Not found on PATH — set a custom path or install the CLI
                  </div>
                  <div
                    style={{
                      display: 'flex',
                      gap: 8,
                      justifyContent: 'center',
                      alignItems: 'center',
                    }}
                  >
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
                      style={{ ...inlineInput, flex: 1, maxWidth: 380 }}
                    />
                    <button
                      onClick={browseCustomBin}
                      className="wks-composer-ctl"
                      style={ghostBtnSmall}
                    >
                      Browse…
                    </button>
                  </div>
                </div>
              )}
            </div>
          )}

          {!isClaude && (
            <div
              style={{
                marginTop: 10,
                maxWidth: 460,
                textAlign: 'center',
                color: 'var(--wks-text-faint)',
                fontSize: '0.62rem',
                lineHeight: 1.5,
              }}
            >
              Runs via claudemon's {providerLabel} adapter — conversation and usage stream into the
              agent view. Approvals are auto-accepted for now.
            </div>
          )}

          {/* ── Advanced options — header button + attached row card ────── */}
          <div style={{ width: '100%', maxWidth: 620, marginTop: 28 }}>
            <button
              type="button"
              aria-expanded={advancedOpen}
              onClick={toggleAdvanced}
              style={{
                width: '100%',
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '9px 12px',
                borderRadius: advancedOpen
                  ? 'var(--wks-radius-md, 8px) var(--wks-radius-md, 8px) 0 0'
                  : 'var(--wks-radius-md, 8px)',
                border: `1px solid ${advBorderColor}`,
                background: bypassSelected
                  ? 'color-mix(in srgb, var(--wks-danger, #e05555) 6%, transparent)'
                  : 'transparent',
                color: 'var(--wks-text-primary)',
                cursor: 'pointer',
                fontFamily: 'inherit',
                textAlign: 'left',
              }}
            >
              <span style={{ ...quietLabel, color: 'var(--wks-text-muted)' }}>advanced</span>
              <span
                style={{
                  flex: 1,
                  minWidth: 0,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  fontSize: '0.68rem',
                  color: bypassSelected ? 'var(--wks-danger, #e05555)' : 'var(--wks-text-faint)',
                }}
              >
                {advancedSummary}
              </span>
              <ChevronDown
                size={14}
                aria-hidden
                style={{
                  flexShrink: 0,
                  color: 'var(--wks-text-faint)',
                  transform: advancedOpen ? 'rotate(180deg)' : 'none',
                  transition: 'transform 0.15s',
                }}
              />
            </button>

            {advancedOpen && (
              <div
                style={{
                  border: `1px solid ${advBorderColor}`,
                  borderTop: 'none',
                  borderRadius: '0 0 var(--wks-radius-md, 8px) var(--wks-radius-md, 8px)',
                  padding: '2px 14px 4px',
                }}
              >
                {rows.map((row, i) => (
                  <div
                    key={row.key}
                    title={row.title}
                    style={{
                      display: 'grid',
                      gridTemplateColumns: '96px minmax(0, 1fr)',
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
            )}
          </div>

          {/* Bypass danger note — always visible when a bypass mode is picked */}
          {bypassSelected && (
            <div
              style={{
                marginTop: 12,
                fontSize: '0.68rem',
                color: 'var(--wks-danger, #e05555)',
                textAlign: 'center',
              }}
            >
              {isClaude
                ? 'Dangerous — bypasses all approval prompts (--dangerously-skip-permissions).'
                : 'Dangerous — auto-approves every command and file change, no prompts.'}
            </div>
          )}

          {/* ── Launch ──────────────────────────────────────────────────── */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginTop: 38 }}>
            <button
              onClick={onCancel}
              className="wks-composer-ctl"
              style={{
                fontSize: '0.78rem',
                fontFamily: 'inherit',
                fontWeight: 500,
                cursor: 'pointer',
                background: 'transparent',
                color: 'var(--wks-text-muted)',
                border: 'none',
                borderRadius: 'var(--wks-radius-md, 6px)',
                padding: '9px 16px',
              }}
            >
              Cancel
            </button>
            <button
              onClick={submit}
              disabled={!cwd.trim()}
              style={{
                fontSize: '0.82rem',
                fontFamily: 'inherit',
                fontWeight: 600,
                cursor: !cwd.trim() ? 'default' : 'pointer',
                background: !cwd.trim() ? 'var(--wks-bg-input)' : 'var(--wks-accent)',
                color: !cwd.trim() ? 'var(--wks-text-faint)' : 'var(--wks-text-on-accent, #fff)',
                border: 'none',
                borderRadius: 'var(--wks-radius-md, 6px)',
                padding: '9px 26px',
              }}
            >
              Spawn agent
            </button>
          </div>
          <div style={{ marginTop: 14, fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>
            enter to spawn · esc to cancel
          </div>
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

const quietLabel: React.CSSProperties = {
  fontSize: '0.58rem',
  fontWeight: 600,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: 'var(--wks-text-faint)',
  userSelect: 'none',
  whiteSpace: 'nowrap',
};

/** Flat, borderless select inside an advanced row — the value IS the control. */
const rowSelect: React.CSSProperties = {
  background: 'transparent',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm, 4px)',
  padding: '3px 2px',
  fontSize: '0.72rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  color: 'var(--wks-text-primary)',
  cursor: 'pointer',
  maxWidth: '100%',
  textOverflow: 'ellipsis',
};

/** Container for a two-option segmented toggle (transport, worktree). */
const segGroup: React.CSSProperties = {
  display: 'inline-flex',
  gap: 2,
  padding: 2,
  border: '1px solid var(--wks-border-input)',
  borderRadius: 999,
};

/** Segmented toggle button (transport, worktree). */
const segBtn = (active: boolean): React.CSSProperties => ({
  fontSize: '0.7rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: 'pointer',
  padding: '3px 10px',
  borderRadius: 999,
  border: 'none',
  background: active ? 'var(--wks-accent-bg)' : 'transparent',
  color: active ? 'var(--wks-accent-text, var(--wks-text-primary))' : 'var(--wks-text-muted)',
  transition: 'background-color 0.15s, color 0.15s',
});

/** Low-chrome mono text input — underline only. */
const inlineInput: React.CSSProperties = {
  background: 'transparent',
  border: 'none',
  outline: 'none',
  borderBottom: '1px solid var(--wks-border-input)',
  fontFamily: 'var(--wks-font-mono)',
  fontSize: '0.7rem',
  color: 'var(--wks-text-primary)',
  padding: '2px 2px 4px',
  boxSizing: 'border-box',
};

/** Small ghost button (Browse…) — flat, rounds via the composer hover class. */
const ghostBtnSmall: React.CSSProperties = {
  fontSize: '0.68rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  cursor: 'pointer',
  background: 'transparent',
  color: 'var(--wks-text-muted)',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm, 4px)',
  padding: '4px 9px',
  whiteSpace: 'nowrap',
  flexShrink: 0,
};

export default SpawnAgentDialog;
