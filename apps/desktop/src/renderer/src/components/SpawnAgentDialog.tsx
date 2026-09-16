import { useProviderReadiness } from '../hooks/useProviderReadiness';
import { useAgentRuntimeStatus } from '../hooks/useAgentRuntimeStatus';
import type { WorktreeInfo } from '../types/electron';
import { containDialogTab } from '../lib/dialogKeyboard';
import { spawnFailureMessage } from '../lib/spawnFailure';
import { ProjectMark } from './ProjectMark';
import { resolveProject } from '../lib/projectIdentity';
import { projectKey } from '../lib/projectKey';
import type { ProjectIdentity } from '../hooks/useConfig';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { deriveAgentName } from '../hooks/useAgentManager';
import { AgentLogo } from './agentLogos';
import type { LibraryItem } from '../types/library';
import type { PluginManifest } from '../types/plugin';
import type { AgentProvider } from '../types/pane';
import { capsFor, effortLevelLabel, type EffortLevel } from '../lib/providerCaps';
import { fetchFederationPeers, type FederationPeer } from '../lib/federation';
import { useProviderDetection } from '../hooks/useProviderDetection';
import {
  providerAvailability,
  visibleProviderOptions,
  type ProviderDetection,
} from '../lib/providerAvailability';
import { profilesForProvider } from '../lib/profileFields';
import { PROFILE_CAPS, type ProfileProvider } from '../../../main/shared/agentProfiles';
import { claudeCatalogOptions, modelOptionCommand, type ModelOption } from '../lib/modelOptions';
import { normalizeModelSelection } from '../../../main/shared/modelContextWindows';
import { DEFAULT_CODEX_CONTEXT_WINDOW } from '../../../main/shared/providerContext';
import { ModelContextPopover } from './ModelContextPopover';
import './SpawnAgentDialog.css';

/**
 * What a profile chip promises, in the vocabulary of the harness it belongs to.
 * The three differ in what the config root actually switches, and saying
 * "Claude profile" over a Codex chip was the old text's whole problem.
 */
function profileChipTitle(profile: SpawnProfile, provider: AgentProvider): string {
  const caps = provider in PROFILE_CAPS ? PROFILE_CAPS[provider as ProfileProvider] : undefined;
  if (!caps) return 'Dispatch with this profile.';
  const root = `sets ${caps.configRootEnv}`;
  const preset = profile.preset ? `, plus ${caps.presetFlag} ${profile.preset}` : '';
  if (profile.isDefault)
    return `Dispatch under the default ${caps.label} login — plus whatever this profile carries${preset}.`;
  return `Dispatch with this ${caps.label} profile — ${root}${preset}.`;
}

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
  /** Which harness this profile configures. ABSENT MEANS CLAUDE — see
   *  shared/agentProfiles.profileProviderOf. */
  provider?: ProfileProvider;
  mcpItemIds?: string[];
  isDefault?: boolean;
  /** Codex only: `-p <name>`, a same-account settings preset. */
  preset?: string;
}

interface SpawnAgentDialogProps {
  defaultCwd: string;
  /** config.projects — the identity shown once the typed cwd resolves to a
   *  project you already know. */
  projects?: Record<string, ProjectIdentity>;
  /** Provider pre-selected in the picker (config.agents.defaultProvider). */
  defaultProvider?: AgentProvider;
  /** Claude transport pre-selected in the picker (config.claude.transport). */
  defaultTransport?: 'pty' | 'stream';
  /** Codex transport pre-selected in the picker (config.codex.transport).
   *  Its own prop because the two harnesses default differently — codex ships
   *  headless — and one shared picker value used to inherit Claude's. */
  defaultCodexTransport?: 'pty' | 'stream';
  /** Pre-check the git-worktree toggle (config.agents.spawnInWorktree). */
  defaultWorktree?: boolean;
  /** User-authored task handed over by the caller, editable before dispatch. */
  defaultPrompt?: string;
  requireTask?: boolean;
  onSpawn: (opts: {
    cwd: string;
    name?: string;
    provider?: AgentProvider;
    /** Claude only: 'pty' | 'stream'. */
    transport?: 'pty' | 'stream';
    profileId?: string;
    launchIntegrationId?: string | null;
    model?: string;
    modelIdentity?: string;
    contextWindow?: number | null;
    effort?: string;
    permissionMode?: string;
    skipPermissions?: boolean;
    mcpItemIds?: string[];
    /** Workspacer MCP tool tier (view/triage/operator); omitted = none. */
    toolScope?: 'view' | 'triage' | 'operator';
    /** Plugin ids whose contributed facade tools the agent may use (needs toolScope). */
    pluginTools?: string[];
    resumeSessionId?: string;
    /** Spawn into a fresh git worktree of `cwd` instead of `cwd` itself. */
    worktree?: boolean;
    /** Auto-sent atomically with spawn; never sent by a follow-up call. */
    kickoffMessage?: string;
    /** Federation: spawn on this peer hub instead of this machine. */
    targetHub?: string;
  }) => Promise<unknown> | void;
  onCancel: () => void;
}

const CUSTOM = '__custom__';
/** Stable empty list so effects keyed on the detection array don't re-run. */
const NO_DETECTION: ProviderDetection[] = [];
const ADVANCED_OPEN_KEY = 'workspacer.spawn.advancedOpen';

const PROVIDERS: { value: AgentProvider; label: string; beta?: boolean }[] = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  // Not yet thoroughly tested — surfaced with a Beta badge so expectations are set.
  { value: 'copilot', label: 'GitHub Copilot', beta: true },
  { value: 'opencode', label: 'OpenCode', beta: true },
  { value: 'pi', label: 'Pi', beta: true },
];

/** Free-text model placeholder per managed provider (their own id formats). */
function modelPlaceholder(provider: AgentProvider): string {
  switch (provider) {
    case 'codex':
      return 'gpt-5.4  (blank = Codex default)';
    // Copilot's picker is account-gated: on a plan with `model_picker_enabled:
    // false` every explicit id is refused and only `auto` works, so that is
    // what the daemon's model list offers and what this hint names.
    case 'copilot':
      return 'auto  (blank = Copilot picks the model)';
    case 'pi':
      return 'claude-sonnet-4 / gpt-5  (blank = Pi default)';
    default:
      return 'anthropic/claude-sonnet-4  (blank = OpenCode default)';
  }
}

interface ProviderModel {
  id: string;
  label: string;
  default: boolean;
  /** Exact effort ids reported for this model by the provider catalog. */
  effortLevels?: string[];
  defaultContextWindow?: number;
  maxContextWindow?: number;
  effectiveContextWindowPercent?: number;
}

/**
 * The "new agent" screen. Despite the (legacy) name it renders as a full-bleed
 * workspace page — a blank agent about to be born — not a floating modal:
 * the existing branded header above an F-line project rule and word controls.
 * Less common launch options remain available through Advanced.
 */
const SpawnAgentDialog: React.FC<SpawnAgentDialogProps> = ({
  projects,
  defaultCwd,
  defaultProvider,
  defaultTransport,
  defaultCodexTransport,
  defaultWorktree,
  defaultPrompt,
  requireTask = false,
  onSpawn,
  onCancel,
}) => {
  const [cwd, setCwd] = useState(defaultCwd);
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState(defaultPrompt ?? '');
  // Only explicit task handoffs (onboarding / command palette) collect a message.
  // Ordinary New Agent starts an empty chat.
  const hasTaskHandoff = requireTask || defaultPrompt !== undefined;
  const [busy, setBusy] = useState(false);
  const submitting = useRef(false);
  const [error, setError] = useState('');
  const errorRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (error) errorRef.current?.focus();
  }, [error]);
  const [provider, setProvider] = useState<AgentProvider>(defaultProvider ?? 'claude');
  // The two harnesses that HAVE a transport choice default differently (claude
  // 'pty' historically, codex 'stream'), so the picker's default is a function
  // of the selected provider, not one shared config read. Fallbacks mirror
  // main/lib/spawnTransport's TRANSPORT_FALLBACK.
  const transportDefaultFor = (p: AgentProvider): 'pty' | 'stream' =>
    p === 'codex' ? (defaultCodexTransport ?? 'stream') : (defaultTransport ?? 'pty');
  // Transport override for this spawn. `touched` is what keeps switching the
  // provider picker from stomping a choice the user actually made: until they
  // click a transport button, the value TRACKS the selected harness's default.
  const [transportTouched, setTransportTouched] = useState(false);
  const [transportChoice, setTransportChoice] = useState<'pty' | 'stream'>(
    transportDefaultFor(defaultProvider ?? 'claude'),
  );
  const transport = transportTouched ? transportChoice : transportDefaultFor(provider);
  const setTransport = (t: 'pty' | 'stream') => {
    setTransportTouched(true);
    setTransportChoice(t);
  };
  // Detection drives availability feedback and which provider words appear:
  // the picker offers installed harnesses (see visibleProviders below).
  const { detection, refresh: refreshDetection } = useProviderDetection();
  const providerDetection: ProviderDetection[] = detection ?? NO_DETECTION;
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
  const [launchPlugins, setLaunchPlugins] = useState<PluginManifest[]>([]);
  const [launchIntegrationId, setLaunchIntegrationId] = useState('');

  // MCP servers available in the Library, and the per-spawn selection. Pre-filled
  // from the chosen profile's default loadout; overridable here.
  const [mcpItems, setMcpItems] = useState<LibraryItem[]>([]);
  const [mcpSel, setMcpSel] = useState<string[]>([]);
  // Workspacer MCP tools for the new agent ('' = none). The tier decides which
  // tool subset the facade serves it — and what it pays context for.
  const [toolScope, setToolScope] = useState<'' | 'view' | 'triage' | 'operator'>('');
  // Installed plugins that contribute agent tools (manifest `tools`), and the
  // per-spawn grant. Only meaningful with a tier — the token records the grant.
  const [toolPlugins, setToolPlugins] = useState<
    Array<{ id: string; name: string; tools: number }>
  >([]);
  const [pluginToolsSel, setPluginToolsSel] = useState<string[]>([]);

  // Model selection. `modelSel` is the dropdown value (''=Default, an alias/id,
  // or the CUSTOM sentinel); `customModel` holds the free-text id when CUSTOM.
  const [aliases, setAliases] = useState<ModelOption[]>([]);
  const [seen, setSeen] = useState<ModelOption[]>([]);
  const [modelSel, setModelSel] = useState<string>('');
  const [customModel, setCustomModel] = useState('');
  const [codexContextWindow, setCodexContextWindow] = useState<number | null>(
    DEFAULT_CODEX_CONTEXT_WINDOW,
  );
  const [codexContextTouched, setCodexContextTouched] = useState(false);
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

  const providerChoicesRef = useRef<HTMLDivElement>(null);

  // Federation: peer hubs this one can spawn onto. The Machine row only
  // appears when at least one peer is connected; '' = this machine.
  const [peers, setPeers] = useState<FederationPeer[]>([]);
  const [targetHub, setTargetHub] = useState('');
  const eligibleLaunchPlugins = launchPlugins.filter(
    (pl) =>
      ['claude', 'codex'].includes(provider) &&
      !targetHub &&
      String(window.electronAPI.platform) !== 'web' &&
      !pl.disabled &&
      pl.launchIntegration?.version === 1 &&
      pl.launchIntegration.agents.includes(provider),
  );
  useEffect(() => {
    setLaunchIntegrationId('');
  }, [provider, targetHub]);
  useEffect(() => {
    let live = true;
    fetchFederationPeers().then((p) => {
      if (live) setPeers(p);
    });
    return () => {
      live = false;
    };
  }, []);
  const connectedPeers = peers.filter((p) => p.connected);

  // Resume an existing Claude session in this cwd. ''=start fresh.
  const [sessions, setSessions] = useState<
    Array<{ sessionId: string; timestamp: string; summary: string }>
  >([]);
  const [resumeSessionId, setResumeSessionId] = useState('');
  // Facts belong to the selected owner and exact path, never the last reply.
  const readiness = useProviderReadiness(
    provider,
    targetHub,
    profileId ||
      launchIntegrationId ||
      (provider === 'claude' && transport === 'pty' ? 'claude-pty' : ''),
  );
  const runtimeStatus = useAgentRuntimeStatus(!!toolScope, !!targetHub);
  const [useWorktree, setUseWorktree] = useState(!!defaultWorktree);
  const [folderResult, setFolderResult] = useState<{ key: string; info: WorktreeInfo } | null>(
    null,
  );
  const [folderCheck, setFolderCheck] = useState(0);
  const folderKey = JSON.stringify([targetHub, cwd.trim()]);
  const repoInfo = folderResult?.key === folderKey ? folderResult.info : null;
  useEffect(() => {
    const dir = cwd.trim();
    setFolderResult(null);
    if (!dir || targetHub || !window.electronAPI.worktreeInfo) return;
    let cancelled = false;
    window.electronAPI
      .worktreeInfo(dir)
      .then((info) => {
        if (!cancelled && info) setFolderResult({ key: folderKey, info });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [folderKey, folderCheck]);
  const worktreeEligible = !targetHub && repoInfo?.isRepo === true && !resumeSessionId;
  const folderDetail = targetHub
    ? 'Folder belongs to the selected remote machine. Its host has not verified this path; local folders are not checked.'
    : repoInfo?.directory === 'invalid'
      ? 'Choose an existing accessible directory. A leading ~ is not expanded; use an absolute path.'
      : repoInfo?.isRepo === true
        ? `Existing git folder${repoInfo.branch ? ` · ${repoInfo.branch}` : ''}. Branch isolation is available in Advanced.`
        : repoInfo?.directory === 'accessible'
          ? repoInfo.gitStatus === 'non-git'
            ? 'Existing non-git folder. Agents can work here; no branch isolation.'
            : 'Existing accessible folder. Git eligibility could not be verified.'
          : 'Folder has not been verified by this host. Use an existing absolute path on the agent’s machine.';

  useEffect(() => {
    setCwd(defaultCwd);
  }, [defaultCwd]);

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
        if (!submitting.current) onCancel();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onCancel]);

  // Discover resumable sessions whenever the directory settles (debounced).
  useEffect(() => {
    const dir = cwd.trim();
    if (!dir || targetHub) {
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
  }, [cwd, targetHub]);

  // If the picked session disappears from the list (cwd changed), reset to fresh.
  useEffect(() => {
    if (resumeSessionId && !sessions.some((s) => s.sessionId === resumeSessionId))
      setResumeSessionId('');
  }, [sessions, resumeSessionId]);

  useEffect(() => {
    window.electronAPI
      .claudeProfilesList?.()
      .then((list: SpawnProfile[]) => setProfiles(list ?? []))
      .catch(() => {});
    window.electronAPI
      .libraryList?.(defaultCwd || undefined)
      .then((list) => setMcpItems((list ?? []).filter((it) => it.kind === 'mcp')))
      .catch(() => {});
    window.electronAPI
      .claudeListModels?.()
      .then((res) => {
        if (!res) return;
        const catalog = claudeCatalogOptions(res);
        setAliases(catalog.filter((option) => !option.seen));
        setSeen(catalog.filter((option) => option.seen));
        // Seed the permission control from the last spawn's saved mode, but only
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
        const defaultCommand = d
          ? modelOptionCommand({
              key: '',
              id: d,
              label: d,
              contextWindow: res.contextWindow ?? null,
            })
          : '';
        const known =
          defaultCommand === '' ||
          catalog.some((option) => modelOptionCommand(option) === defaultCommand);
        if (known) {
          setModelSel(defaultCommand);
        } else {
          setModelSel(CUSTOM);
          setCustomModel(defaultCommand);
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
      .providerListModels?.(
        provider as 'codex' | 'copilot' | 'opencode' | 'pi',
        targetHub ? undefined : cwd.trim() || undefined,
      )
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
    if (targetHub) {
      setMcpItems([]);
      return;
    }
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
  }, [cwd, targetHub]);

  // Plugins contributing agent tools, for the workspacer-tier picker. One-shot:
  // the installed-plugin set doesn't change while the dialog is open.
  useEffect(() => {
    let cancelled = false;
    window.electronAPI
      .listHubPlugins?.()
      .then((list) => {
        if (cancelled || !Array.isArray(list)) return;
        setLaunchPlugins(list);
        setToolPlugins(
          list
            .filter((pl) => !pl.disabled && (pl.tools?.length ?? 0) > 0)
            .map((pl) => ({ id: pl.id, name: pl.name || pl.id, tools: pl.tools!.length })),
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  // The profiles this spawn may actually use. A Claude profile on a Codex
  // spawn would point CODEX_HOME at a Claude config root, and a harness with no
  // config root (OpenCode, Pi) gets an empty list — which is what makes the
  // picker disappear rather than offer chips that set nothing.
  const eligibleProfiles = useMemo(
    () => profilesForProvider(profiles, provider),
    [profiles, provider],
  );

  // Selection follows the harness. Switching provider must not leave the other
  // harness's profile id selected — it would be dropped at spawn while the
  // dialog still showed it as chosen.
  //
  // The default row is SELECTED rather than offered beside a "no profile" chip:
  // the service MATERIALIZES an `id: 'default'` row named "Default"
  // (claudeProfiles.ts / the brain's twin), so a synthetic one rendered two
  // chips both labelled Default — and picking the synthetic one silently
  // skipped the MCP loadout attached to the real Default in Settings.
  useEffect(() => {
    setProfileId((cur) => {
      if (cur && eligibleProfiles.some((p) => p.id === cur)) return cur;
      return eligibleProfiles.find((p) => p.isDefault)?.id || eligibleProfiles[0]?.id || '';
    });
  }, [eligibleProfiles]);

  // Pre-fill the MCP selection from the chosen profile's default loadout.
  // Claude only: the loadout rides Claude's --mcp-config, and PROFILE_CAPS says
  // so — a Codex/Copilot profile is forced to carry an empty one on write.
  useEffect(() => {
    const p = isClaude ? eligibleProfiles.find((x) => x.id === profileId) : undefined;
    setMcpSel(p?.mcpItemIds ?? []);
  }, [profileId, eligibleProfiles, isClaude]);

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
  const claudeSelection = resolvedModel ? normalizeModelSelection(resolvedModel) : undefined;
  const setClaudeContext = (contextWindow: number | null) => {
    if (!claudeSelection || contextWindow == null) return;
    const sibling = [...aliases, ...seen].find(
      (option) => option.id === claudeSelection.model && option.contextWindow === contextWindow,
    );
    if (sibling) setModelSel(modelOptionCommand(sibling));
  };
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
      .then(() => refreshDetection())
      .catch(() =>
        setError(
          'The binary override could not be saved. Retry saving the path or check the provider again.',
        ),
      );
  };

  const browseCustomBin = async () => {
    const files = await window.electronAPI.pickFiles?.(undefined);
    if (files?.length) {
      setCustomBinPath(files[0]);
      saveCustomBin(files[0]);
    }
  };

  // Only harnesses whose CLI is actually on this machine — plus whatever this
  // dialog is already pointed at (the picked provider, the configured default),
  // which stays listed and flagged rather than vanishing under the selection.
  const visibleProviders = visibleProviderOptions(PROVIDERS, detection, [
    provider,
    defaultProvider,
  ]);

  const currentDetection = providerDetection.find((d) => d.provider === provider);
  const missingProvider = !targetHub && providerAvailability(detection, provider) === 'missing';
  const canSubmit =
    !!cwd.trim() &&
    !missingProvider &&
    !runtimeStatus.blocked &&
    repoInfo?.directory !== 'invalid' &&
    (!hasTaskHandoff || !!prompt.trim()) &&
    !busy;
  const submit = async () => {
    if (!canSubmit || submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError('');
    try {
      // '' means the provider's own default mode; the legacy boolean tracks the
      // bypass-family modes for back-compat consumers (saved defaults, respawn).
      const resolvedMode = permissionMode || defaultModeFor(provider);
      const skipPermissions = resolvedMode === 'bypassPermissions' || resolvedMode === 'yolo';
      const kickoff = hasTaskHandoff ? { kickoffMessage: prompt.trim() } : {};
      // Claude-only options are dropped for other providers (they run their own
      // TUI in Tier-1 and don't take Claude's profile/model/MCP/resume flags).
      await onSpawn(
        isClaude
          ? {
              cwd: cwd.trim(),
              name: name.trim() || undefined,
              transport,
              profileId: profileId || undefined,
              launchIntegrationId: launchIntegrationId || undefined,
              model: resolvedModel || undefined,
              modelIdentity: claudeSelection?.model,
              contextWindow: claudeSelection?.contextWindow,
              effort: effort || undefined,
              permissionMode: resolvedMode,
              skipPermissions,
              // Facade sessions take the workspacer MCP config instead of the
              // Library selection (backend rule) — don't send a selection that
              // would be silently ignored.
              mcpItemIds: !toolScope && mcpSel.length ? mcpSel : undefined,
              toolScope: toolScope || undefined,
              pluginTools: toolScope && pluginToolsSel.length ? pluginToolsSel : undefined,
              resumeSessionId: resumeSessionId || undefined,
              // Worktree is local-machine isolation — moot on a peer hub.
              worktree: useWorktree && worktreeEligible && !targetHub ? true : undefined,
              ...kickoff,
              targetHub: targetHub || undefined,
            }
          : {
              cwd: cwd.trim(),
              name: name.trim() || undefined,
              model: resolvedProviderModel || undefined,
              modelIdentity: resolvedProviderModel || undefined,
              contextWindow:
                provider === 'codex'
                  ? resumeSessionId && !codexContextTouched
                    ? undefined
                    : codexContextWindow
                  : undefined,
              provider,
              // The harness's own profile — its config root (CODEX_HOME /
              // COPILOT_HOME), extra argv, `-p` preset and, for Copilot, the
              // referenced token. Empty for a harness with no config root,
              // because eligibleProfiles is empty there.
              profileId: profileId || undefined,
              launchIntegrationId: launchIntegrationId || undefined,
              // Codex only, and ALWAYS stated: both shapes are real choices now
              // (headless is the default, hybrid the opt-in), so sending only the
              // non-default would leave "hybrid" indistinguishable from "the user
              // said nothing" — which main resolves to the configured default.
              transport: provider === 'codex' ? transport : undefined,
              effort: effort || undefined,
              permissionMode: resolvedMode,
              skipPermissions,
              // Pi ships no MCP client — a tier grant would only mint a dangling
              // token, so it isn't sent.
              toolScope: provider !== 'pi' ? toolScope || undefined : undefined,
              pluginTools:
                provider !== 'pi' && toolScope && pluginToolsSel.length
                  ? pluginToolsSel
                  : undefined,
              worktree: useWorktree && worktreeEligible && !targetHub ? true : undefined,
              ...kickoff,
              targetHub: targetHub || undefined,
            },
      );
    } catch (err) {
      setError(spawnFailureMessage(provider, err));
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  };

  // A project you have NAMED should name its agents. deriveAgentName falls back
  // to the directory basename, which is the same thing when nobody set a label
  // — so this only differs where the user actually said what to call the place.
  const placeholderName = cwd.trim()
    ? (!targetHub && resolveProject(cwd.trim(), projects)?.label) || deriveAgentName(cwd.trim())
    : 'agent';
  const providerLabel = PROVIDERS.find((p) => p.value === provider)?.label ?? provider;
  const bypassSelected = permissionMode === 'bypassPermissions' || permissionMode === 'yolo';
  // Keep hidden overrides visible after closing More, without repeating the
  // model, permissions, transport and context already shown in the word strip.
  const deviations: { key: string; label: string }[] = [];
  if (targetHub) deviations.push({ key: 'machine', label: `on ${targetHub}` });
  if (name.trim()) deviations.push({ key: 'name', label: name.trim() });
  const selectedProfile = eligibleProfiles.find((p) => p.id === profileId);
  if (selectedProfile && !selectedProfile.isDefault)
    deviations.push({ key: 'profile', label: selectedProfile.name });
  if (launchIntegrationId)
    deviations.push({
      key: 'integration',
      label:
        eligibleLaunchPlugins.find((p) => p.id === launchIntegrationId)?.name ||
        launchIntegrationId,
    });
  if (effort) deviations.push({ key: 'effort', label: `${effort} effort` });
  if (useWorktree && worktreeEligible) deviations.push({ key: 'worktree', label: 'worktree' });
  if (resumeSessionId) deviations.push({ key: 'resume', label: 'resume' });
  if (mcpSel.length) deviations.push({ key: 'mcp', label: `${mcpSel.length} MCP` });
  if (toolScope)
    deviations.push({
      key: 'wks',
      label:
        `workspacer: ${toolScope}` +
        (pluginToolsSel.length ? ` +${pluginToolsSel.length} plugin` : ''),
    });
  if (customBinPath.trim()) deviations.push({ key: 'binary', label: 'custom binary' });

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

  // Federation: pick the machine to spawn on. Hidden entirely unless a peer
  // hub is actually connected — the local-only case never sees it.
  if (connectedPeers.length > 0) {
    rows.push({
      key: 'machine',
      label: 'machine',
      title: 'Which hub runs this agent — its cwd, files and shell live there.',
      control: (
        <select
          aria-label="Machine"
          value={targetHub}
          onChange={(e) => setTargetHub(e.target.value)}
          style={rowSelect}
        >
          <option value="">This machine</option>
          {connectedPeers.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
      ),
    });
  }

  if (isClaude) {
    rows.push({
      key: 'model',
      label: 'model',
      control: (
        <>
          <select
            aria-label="Model"
            value={modelSel}
            onChange={(e) => setModelSel(e.target.value)}
            style={rowSelect}
          >
            <option value="">default model</option>
            {aliases.length > 0 && (
              <optgroup label="Latest">
                {aliases.map((a) => (
                  <option key={a.key} value={modelOptionCommand(a)}>
                    {a.label}
                    {a.context ? ` · ${a.context}` : ''}
                  </option>
                ))}
              </optgroup>
            )}
            {seen.length > 0 && (
              <optgroup label="Seen in sessions">
                {seen.map((m) => (
                  <option key={m.key} value={modelOptionCommand(m)}>
                    {m.label}
                  </option>
                ))}
              </optgroup>
            )}
            <option value={CUSTOM}>Custom…</option>
          </select>
          {modelSel === CUSTOM && (
            <input
              aria-label="Custom model"
              value={customModel}
              onChange={(e) => setCustomModel(e.target.value)}
              onKeyDown={keySubmit}
              placeholder="claude-opus-4-8  or  opus"
              spellCheck={false}
              style={{
                ...inlineInput,
                display: 'block',
                marginTop: 8,
                width: '100%',
                maxWidth: 260,
              }}
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
            aria-label="Model"
            value={providerSel}
            onChange={(e) => setProviderSel(e.target.value)}
            style={rowSelect}
          >
            <option value="">default model</option>
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
              aria-label="Custom model"
              value={providerCustom}
              onChange={(e) => setProviderCustom(e.target.value)}
              onKeyDown={keySubmit}
              placeholder={modelPlaceholder(provider)}
              spellCheck={false}
              style={{
                ...inlineInput,
                display: 'block',
                marginTop: 8,
                width: '100%',
                maxWidth: 300,
              }}
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
          aria-label="Custom model"
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

  const claudeContextChoices = claudeSelection
    ? [...aliases, ...seen]
        .filter((option) => option.id === claudeSelection.model && option.contextWindow != null)
        .map((option) => ({ value: option.contextWindow!, label: option.context ?? option.label }))
        .filter(
          (choice, index, all) => all.findIndex((item) => item.value === choice.value) === index,
        )
    : [];
  rows.push({
    key: 'context',
    label: 'context',
    control: (
      <ModelContextPopover
        provider={provider}
        requested={
          isClaude
            ? claudeSelection?.contextWindow
            : provider === 'codex'
              ? resumeSessionId && !codexContextTouched
                ? undefined
                : codexContextWindow
              : null
        }
        providerDefault={selectedProviderModel?.defaultContextWindow}
        advertisedMaximum={selectedProviderModel?.maxContextWindow}
        choices={
          isClaude
            ? claudeContextChoices
            : provider === 'codex'
              ? [
                  { value: null, label: 'Provider default' },
                  { value: DEFAULT_CODEX_CONTEXT_WINDOW, label: 'Request 1M' },
                ]
              : undefined
        }
        allowNumeric={provider === 'codex'}
        onChange={
          isClaude
            ? setClaudeContext
            : provider === 'codex'
              ? (value) => {
                  setCodexContextTouched(true);
                  setCodexContextWindow(value);
                }
              : undefined
        }
      />
    ),
  });

  if (capsFor(provider).effort) {
    rows.push({
      key: 'effort',
      label: 'effort',
      control: (
        <select
          aria-label="Effort"
          value={effort}
          onChange={(e) => setEffort(e.target.value)}
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
        <div className="spawn-transport">
          {(
            [
              { value: 'pty', label: isClaude ? 'terminal' : 'hybrid' },
              { value: 'stream', label: 'headless' },
            ] as const
          ).map((t) => (
            <button
              key={t.value}
              aria-pressed={transport === t.value}
              onClick={() => setTransport(t.value)}
              className="spawn-word"
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
        aria-label="Permissions"
        value={permissionMode}
        onChange={(e) => setPermissionMode(e.target.value)}
        style={{
          ...rowSelect,
          color: bypassSelected ? 'var(--wks-error)' : rowSelect.color,
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
              aria-pressed={(useWorktree && worktreeEligible) === w.value}
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
          aria-label="Resume session"
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

  // Workspacer MCP tools — the facade tier the new agent gets (see help tool /
  // toolScope). Any provider with an MCP client; pi has none.
  if (provider !== 'pi') {
    rows.push({
      key: 'workspacer',
      label: 'workspacer',
      title:
        'Give this agent the workspacer MCP tools at a tier. View: read-only fleet observation (transcripts, snapshots). Triage: view + approve/reply/interrupt + UI navigation. Operator: full control including spawning agents and host files.',
      control: (
        <div>
          <select
            aria-label="Workspacer tools"
            value={toolScope}
            onChange={(e) => setToolScope(e.target.value as '' | 'view' | 'triage' | 'operator')}
            style={rowSelect}
          >
            <option value="">Off (no workspacer tools)</option>
            <option value="view">View — observe the fleet (read-only)</option>
            <option value="triage">Triage — view + approve, reply, navigate UI</option>
            <option value="operator">Operator — full control (spawn, files, config)</option>
          </select>
          {toolScope && toolPlugins.length > 0 && (
            <div style={{ marginTop: 8 }}>
              <div
                style={{
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-faint)',
                  marginBottom: 4,
                }}
              >
                Plugin tools this agent may use:
              </div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                {toolPlugins.map((pl) => {
                  const on = pluginToolsSel.includes(pl.id);
                  return (
                    <button
                      key={pl.id}
                      aria-pressed={on}
                      onClick={() =>
                        setPluginToolsSel((sel) =>
                          on ? sel.filter((id) => id !== pl.id) : [...sel, pl.id],
                        )
                      }
                      title={`${pl.id} — ${pl.tools} tool${pl.tools === 1 ? '' : 's'}`}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 6,
                        fontSize: '0.72rem',
                        fontWeight: 500,
                        fontFamily: 'inherit',
                        padding: '4px 11px',
                        borderRadius: 'var(--wks-radius-pill)',
                        cursor: 'pointer',
                        maxWidth: 220,
                        border: on
                          ? '1px solid var(--wks-accent)'
                          : '1px solid var(--wks-border-input)',
                        background: on ? 'var(--wks-accent-bg)' : 'transparent',
                        color: on ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
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
                        {pl.name}
                      </span>
                      <span style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}>
                        {pl.tools}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
          {toolScope && mcpSel.length > 0 && (
            <div style={{ marginTop: 6, fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}>
              Workspacer tools replace the Library MCP selection — the {mcpSel.length} selected
              server{mcpSel.length === 1 ? '' : 's'} won't be loaded.
            </div>
          )}
        </div>
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
                  aria-pressed={on}
                  onClick={() => toggleMcp(it.id)}
                  title={it.description || it.id}
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.72rem',
                    fontWeight: 500,
                    fontFamily: 'inherit',
                    padding: '4px 11px',
                    borderRadius: 'var(--wks-radius-pill)',
                    cursor: 'pointer',
                    maxWidth: 220,
                    border: on
                      ? '1px solid var(--wks-accent)'
                      : '1px solid var(--wks-border-input)',
                    background: on ? 'var(--wks-accent-bg)' : 'transparent',
                    color: on ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
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
                  <span style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)' }}>
                    {it.mcp?.url ? (it.mcp.type === 'sse' ? 'sse' : 'http') : 'stdio'}
                  </span>
                </button>
              );
            })}
          </div>
          <div style={{ color: 'var(--wks-text-faint)', fontSize: '0.66rem', marginTop: 6 }}>
            Only the checked servers are exposed to this session (--strict-mcp-config).
          </div>
        </div>
      ),
    });
  }

  // Keep the binary override in Advanced when healthy, and beside the
  // diagnostic when missing. The detected path doubles as the placeholder.
  rows.push({
    key: 'binary',
    label: 'binary',
    title: currentDetection?.found ? `Detected: ${currentDetection.resolvedPath}` : undefined,
    control: (
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <input
          aria-label="Provider binary override"
          value={customBinPath}
          onChange={(e) => setCustomBinPath(e.target.value)}
          onBlur={(e) => saveCustomBin(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') saveCustomBin(customBinPath);
            if (e.key === 'Escape' && !submitting.current) onCancel();
          }}
          placeholder={currentDetection?.resolvedPath ?? `/usr/local/bin/${provider}`}
          spellCheck={false}
          style={{ ...inlineInput, flex: 1, minWidth: 0, maxWidth: 300 }}
        />
        <button onClick={browseCustomBin} className="wks-composer-ctl" style={ghostBtnSmall}>
          Browse…
        </button>
      </div>
    ),
  });

  // The F-line strip exposes the common decisions; everything else stays behind More.
  const PRIMARY_KEYS = ['model', 'permissions', 'transport', 'context'];
  const primaryRows = PRIMARY_KEYS.flatMap((key) => rows.filter((r) => r.key === key));
  const advRows = rows.filter((r) => !PRIMARY_KEYS.includes(r.key));

  return (
    <div
      // The chord leader never arms from inside this dialog: mid-typing a
      // kickoff prompt, an armed layer would steal the following keystrokes
      // (useKeyboardNav's leaderSuppressed check).
      className="spawn-screen"
      role="dialog"
      aria-modal="true"
      aria-label={hasTaskHandoff ? 'Dispatch agent' : 'New Agent'}
      onKeyDown={containDialogTab}
      data-leader-suppress="true"
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
          width: 'min(720px, 100%)',
          height: 720,
          borderRadius: '50%',
          background:
            'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 8%, transparent) 0%, transparent 65%)',
          pointerEvents: 'none',
        }}
      />

      <div style={{ position: 'relative', height: '100%', overflowY: 'auto' }}>
        <fieldset disabled={busy} className="spawn-page">
          <header className="spawn-header">
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
              {hasTaskHandoff ? 'Dispatch agent' : 'New Agent'}
            </div>
            <div style={{ marginTop: 5, fontSize: '0.72rem', color: 'var(--wks-text-muted)' }}>
              {hasTaskHandoff
                ? 'Describe a task, choose its directory, and dispatch.'
                : 'Choose an agent and directory, then start chatting.'}
            </div>
          </header>

          <div className="spawn-form">
            {hasTaskHandoff && (
              <div className="spawn-task">
                <label htmlFor="first-task" className="spawn-sr-only">
                  What should this agent do?
                </label>
                <textarea
                  id="first-task"
                  aria-required="true"
                  aria-describedby="spawn-task-help"
                  autoFocus
                  value={prompt}
                  onChange={(e) => setPrompt(e.target.value)}
                  onKeyDown={(e) => {
                    // Prose keeps Enter; only an explicit chord dispatches it.
                    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
                  }}
                  rows={2}
                  placeholder="What should this agent do first?"
                />
                <div id="spawn-task-help" className="spawn-hint">
                  A task is required. Sent once when you dispatch. Your provider’s permission
                  choices apply.
                </div>
              </div>
            )}

            <div className="spawn-project-line">
              {visibleProviders.length > 1 ? (
                <button
                  className="spawn-provider-mark"
                  aria-label="Change provider"
                  title={`${providerLabel} — choose a provider below`}
                  onClick={() =>
                    providerChoicesRef.current
                      ?.querySelector<HTMLButtonElement>('[aria-pressed="true"]')
                      ?.focus()
                  }
                >
                  <AgentLogo provider={provider} size={22} />
                </button>
              ) : (
                <span className="spawn-provider-mark" title={providerLabel}>
                  <AgentLogo provider={provider} size={22} />
                </span>
              )}
              <input
                aria-label="Working directory"
                aria-describedby="spawn-folder-status"
                aria-invalid={repoInfo?.directory === 'invalid' || undefined}
                autoFocus={!hasTaskHandoff}
                value={cwd}
                onChange={(e) => setCwd(e.target.value)}
                onKeyDown={keySubmit}
                placeholder="/path/to/project"
                spellCheck={false}
              />
              <button
                disabled={!!targetHub}
                onClick={browse}
                className="spawn-word"
                title={
                  targetHub
                    ? 'Enter a path on the selected remote machine'
                    : 'Browse for a project directory'
                }
              >
                Browse…
              </button>
            </div>

            {visibleProviders.length > 1 && (
              <div
                ref={providerChoicesRef}
                role="group"
                aria-label="Provider"
                className="spawn-words spawn-providers"
              >
                {visibleProviders.map((p) => {
                  const availability = providerAvailability(detection, p.value);
                  return (
                    <button
                      key={p.value}
                      className="spawn-word"
                      aria-pressed={provider === p.value}
                      onClick={() => setProvider(p.value)}
                      title={
                        availability === 'installed'
                          ? `Found: ${providerDetection.find((d) => d.provider === p.value)?.resolvedPath}`
                          : availability === 'missing'
                            ? 'Not found on PATH'
                            : 'Availability unknown'
                      }
                    >
                      {p.label}
                      {p.missing ? (
                        <span className="spawn-badge spawn-danger">NOT INSTALLED</span>
                      ) : p.beta ? (
                        <span className="spawn-badge">BETA</span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            )}

            <div className="spawn-words spawn-options" role="group" aria-label="Launch options">
              {primaryRows.map((row) => (
                <div
                  key={row.key}
                  className="spawn-option"
                  role="group"
                  aria-label={row.label}
                  title={row.title}
                >
                  {row.control}
                </div>
              ))}
              <button
                type="button"
                className="spawn-word spawn-more"
                aria-label="Advanced options"
                aria-expanded={advancedOpen}
                aria-controls="spawn-advanced"
                onClick={toggleAdvanced}
              >
                more…{' '}
                <ChevronDown
                  size={12}
                  aria-hidden
                  style={{ transform: advancedOpen ? 'rotate(180deg)' : undefined }}
                />
              </button>
            </div>

            {deviations.length > 0 && (
              <div className="spawn-deviations" aria-label="Advanced overrides">
                {deviations.map((d) => (
                  <span key={d.key}>{d.label}</span>
                ))}
              </div>
            )}

            {advancedOpen && (
              <div id="spawn-advanced" className="spawn-advanced">
                <div className="spawn-advanced-row">
                  <label htmlFor="spawn-name" style={quietLabel}>
                    name
                  </label>
                  <input
                    id="spawn-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    onKeyDown={keySubmit}
                    placeholder={`name it (optional) · ${placeholderName}`}
                    spellCheck={false}
                    style={{ ...inlineInput, width: '100%' }}
                  />
                </div>
                {eligibleProfiles.length > 0 && (
                  <div className="spawn-advanced-row">
                    <span id="spawn-profile-label" style={quietLabel}>
                      profile
                    </span>
                    <div className="spawn-words" role="group" aria-labelledby="spawn-profile-label">
                      {eligibleProfiles.map((p) => (
                        <button
                          key={p.id}
                          className="spawn-word"
                          aria-pressed={profileId === p.id}
                          onClick={() => setProfileId(p.id)}
                          title={profileChipTitle(p, provider)}
                        >
                          {p.name}
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                {eligibleLaunchPlugins.length > 0 && (
                  <div className="spawn-advanced-row">
                    <label htmlFor="launch-integration" style={quietLabel}>
                      Launch integration
                    </label>
                    <div>
                      <select
                        id="launch-integration"
                        value={launchIntegrationId}
                        onChange={(e) => setLaunchIntegrationId(e.target.value)}
                        style={rowSelect}
                      >
                        <option value="">None</option>
                        {eligibleLaunchPlugins.map((pl) => (
                          <option key={pl.id} value={pl.id}>
                            {pl.name || pl.id}
                          </option>
                        ))}
                      </select>
                      <div className="spawn-hint">
                        Applies to this session and its resumes. The selected plugin must be ready
                        before launch.
                      </div>
                    </div>
                  </div>
                )}
                {advRows
                  .filter((row) => row.key !== 'binary' || !missingProvider)
                  .map((row) => (
                    <div key={row.key} className="spawn-advanced-row" title={row.title}>
                      <span style={quietLabel}>{row.label}</span>
                      <div style={{ minWidth: 0 }}>{row.control}</div>
                    </div>
                  ))}
              </div>
            )}

            {bypassSelected && (
              <div className="spawn-hint spawn-danger">
                {isClaude
                  ? 'Dangerous — bypasses all approval prompts (--dangerously-skip-permissions).'
                  : 'Dangerous — auto-approves every command and file change, no prompts.'}
              </div>
            )}

            <div id="spawn-availability" role="status" className="spawn-status">
              <div className="spawn-status-line">
                <span
                  aria-hidden
                  className={`spawn-dot ${missingProvider || runtimeStatus.blocked || repoInfo?.directory === 'invalid' ? 'spawn-dot-error' : ''}`}
                />
                {cwd.trim() && !targetHub && (
                  <span className="spawn-project-identity">
                    <ProjectMark cwd={cwd.trim()} projects={projects} size={12} />
                    {resolveProject(cwd.trim(), projects)?.label}
                    {!projects?.[projectKey(cwd.trim())] && <span> · unregistered folder</span>}
                  </span>
                )}
                {repoInfo?.branch && <span>{repoInfo.branch}</span>}
                {repoInfo?.directory === 'invalid' && (
                  <span className="spawn-danger">Directory unavailable</span>
                )}
                {targetHub && <span>Remote folder unverified</span>}
                {!targetHub && !repoInfo && <span>Folder unverified</span>}
                <span>{runtimeStatus.detail.split('. ')[0]}</span>
                <span>
                  {targetHub
                    ? 'Remote provider availability unknown'
                    : missingProvider
                      ? `${providerLabel} is not installed.`
                      : providerAvailability(detection, provider) === 'installed'
                        ? `${providerLabel} CLI found.`
                        : `${providerLabel} availability is unknown.`}
                </span>
              </div>
              {!['unchecked', 'unsupported', 'responding'].includes(readiness.status.state) && (
                <div className="spawn-readiness-feedback">{readiness.detail}</div>
              )}
              <details className="spawn-status-details">
                <summary>Status details</summary>
                <div id="spawn-folder-status">
                  {folderDetail}
                  <button className="spawn-word" onClick={() => setFolderCheck((n) => n + 1)}>
                    Check folder again
                  </button>
                </div>
                <div id="spawn-runtime-status">
                  {runtimeStatus.detail}
                  <button className="spawn-word" onClick={() => void runtimeStatus.refresh()}>
                    Check runtime again
                  </button>
                </div>
                <div>
                  {targetHub
                    ? 'Provider availability on the selected machine is unknown.'
                    : missingProvider
                      ? `${providerLabel} is not installed. Install its CLI, set a binary override, or choose an installed provider.`
                      : providerAvailability(detection, provider) === 'installed'
                        ? `${providerLabel} CLI found.`
                        : `${providerLabel} availability is unknown. You can try dispatching or check again.`}
                </div>
                {['unchecked', 'unsupported', 'responding'].includes(readiness.status.state) && (
                  <div>{readiness.detail}</div>
                )}
              </details>
              <button
                className="spawn-word"
                onClick={() => {
                  refreshDetection();
                  void readiness.refresh();
                }}
              >
                Check again
              </button>
            </div>

            {missingProvider && (
              <div className="spawn-missing">
                <div className="spawn-hint spawn-danger">
                  Not found on PATH — set a custom path or install the CLI
                </div>
                {rows.find((row) => row.key === 'binary')?.control}
              </div>
            )}
            {error && (
              <div role="alert" ref={errorRef} tabIndex={-1} className="spawn-error">
                {error}
              </div>
            )}

            <div className="spawn-actions">
              <button
                onClick={submit}
                disabled={!canSubmit}
                className="spawn-start"
                aria-describedby={`${hasTaskHandoff ? 'spawn-task-help ' : ''}spawn-availability spawn-runtime-status spawn-folder-status`}
              >
                {busy
                  ? 'Starting…'
                  : error
                    ? hasTaskHandoff
                      ? 'Retry dispatch'
                      : 'Retry launch'
                    : hasTaskHandoff
                      ? 'Dispatch agent'
                      : 'Start agent'}
              </button>
              <button onClick={onCancel} disabled={busy} className="spawn-word">
                Cancel
              </button>
              <div className="spawn-shortcuts spawn-hint">
                {hasTaskHandoff ? (
                  <>
                    <kbd>⌘/ctrl+enter</kbd> dispatch
                  </>
                ) : (
                  <>
                    <kbd>↵</kbd> start
                  </>
                )}{' '}
                · <kbd>esc</kbd> cancel
              </div>
            </div>
          </div>
        </fieldset>
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
  fontSize: '0.66rem',
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
  borderRadius: 'var(--wks-radius-sm)',
  padding: '3px 2px',
  fontSize: '0.72rem',
  fontWeight: 400,
  fontFamily: 'inherit',
  color: 'var(--wks-text-primary)',
  cursor: 'pointer',
  maxWidth: '100%',
  textOverflow: 'ellipsis',
};

/** Worktree isolation toggle in Advanced. */
const segGroup: React.CSSProperties = {
  display: 'inline-flex',
  gap: 2,
  padding: 2,
  border: '1px solid var(--wks-border-input)',
  borderRadius: 'var(--wks-radius-pill)',
};

/** Worktree isolation choice. */
const segBtn = (active: boolean): React.CSSProperties => ({
  fontSize: '0.72rem',
  fontWeight: 600,
  fontFamily: 'inherit',
  cursor: 'pointer',
  padding: '3px 10px',
  borderRadius: 'var(--wks-radius-pill)',
  border: 'none',
  background: active ? 'var(--wks-accent-bg)' : 'transparent',
  color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-muted)',
  transition: 'background-color 0.15s, color 0.15s',
});

/** Low-chrome mono text input — underline only. */
const inlineInput: React.CSSProperties = {
  background: 'transparent',
  border: 'none',
  outline: 'none',
  borderBottom: '1px solid var(--wks-border-input)',
  fontFamily: 'var(--wks-font-mono)',
  fontSize: '0.72rem',
  color: 'var(--wks-text-primary)',
  padding: '2px 2px 4px',
  boxSizing: 'border-box',
};

/** Small ghost button (Browse…) — flat, rounds via the composer hover class. */
const ghostBtnSmall: React.CSSProperties = {
  fontSize: '0.72rem',
  fontFamily: 'inherit',
  fontWeight: 600,
  cursor: 'pointer',
  background: 'transparent',
  color: 'var(--wks-text-muted)',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm)',
  padding: '4px 9px',
  whiteSpace: 'nowrap',
  flexShrink: 0,
};

export default SpawnAgentDialog;
