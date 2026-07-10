import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { AppEntry } from '../hooks/useConfig';
import type { PaneType } from '../types/pane';
import type { PluginPane } from '../types/plugin';
import type { LibraryItem, LibraryAction } from '../types/library';
import { runLibraryItem } from '../lib/libraryBus';
import {
  PaneIcon,
  Globe,
  Puzzle,
  Blocks,
  Brain,
  Bot,
  Zap,
  BarChart3,
  LayoutGrid,
  FolderOpen,
  Plus,
  Smartphone,
  Columns3,
  RefreshCw,
  Settings,
  Sparkles,
  Star,
  IconSearch,
  type LucideIcon,
} from './icons';
import { shortcutFor } from '../lib/shortcuts';
import { useUiMode } from '../hooks/useUiMode';

// ── Unified palette item ──

/** Render a user-supplied icon string: a URL becomes a favicon-style image,
 *  anything else falls back to a thin-stroke lucide glyph (no emoji). */
function userIcon(raw: string | undefined, Fallback: LucideIcon): React.ReactNode {
  if (raw && /^https?:\/\//.test(raw)) {
    return (
      <img
        src={raw}
        width={16}
        height={16}
        style={{ borderRadius: 3, objectFit: 'contain' }}
        alt=""
      />
    );
  }
  return <Fallback size={16} strokeWidth={1.75} />;
}

export interface PaletteItem {
  id: string;
  name: string;
  description?: string;
  icon: React.ReactNode;
  category: 'action' | 'command' | 'app' | 'plugin' | 'library';
  /** For actions: the pane type to create */
  paneType?: PaneType;
  /** For actions: whether to prompt for folder (Claude) */
  pickFolder?: boolean;
  /** For apps: the URL to open */
  url?: string;
  /** For apps: the original AppEntry */
  app?: AppEntry;
  /** For plugins: the resolved pane to open */
  pluginPane?: PluginPane;
  /** For library: the prompt/skill to run */
  libraryItem?: LibraryItem;
  /** For commands: invoked directly on select (toggles, dialogs, etc.) */
  run?: () => void;
  /** Keybinding action id — used to show the shortcut badge on the row. */
  shortcut?: string;
}

// ── Built-in actions ──

export const builtInActions: PaletteItem[] = [
  {
    id: 'new-claude',
    name: 'New Claude Code',
    description: 'AI-powered coding assistant',
    icon: <PaneIcon type="claude" size={16} />,
    category: 'action',
    paneType: 'claude',
    pickFolder: true,
    shortcut: 'new-claude',
  },
  {
    id: 'new-terminal',
    name: 'New Terminal',
    description: 'Shell terminal',
    icon: <PaneIcon type="terminal" size={16} />,
    category: 'action',
    paneType: 'terminal',
    shortcut: 'new-terminal',
  },
  {
    id: 'new-browser',
    name: 'New Browser',
    description: 'Web browser tab',
    icon: <PaneIcon type="browser" size={16} />,
    category: 'action',
    paneType: 'browser',
    shortcut: 'new-browser',
  },
  {
    id: 'new-review',
    name: 'Review Changes',
    description: 'Git diff & status for this agent',
    icon: <PaneIcon type="review" size={16} />,
    category: 'action',
    paneType: 'review',
    shortcut: 'open-review',
  },
  {
    id: 'new-notes',
    name: 'Notes',
    description: 'Markdown scratchpad',
    icon: <PaneIcon type="notes" size={16} />,
    category: 'action',
    paneType: 'notes',
  },
  // The editor is provided by the workspacer.editor plugin, which contributes its
  // own "Editor" entry to the palette — no separate built-in action.
  {
    id: 'open-library',
    name: 'Library',
    description: 'Reusable prompts & skills',
    icon: <PaneIcon type="library" size={16} />,
    category: 'action',
    paneType: 'library',
    shortcut: 'library-picker',
  },
];

// ── Props ──

interface CommandPaletteProps {
  visible: boolean;
  apps: AppEntry[];
  mode?: 'tab' | 'split';
  /** Active agent's working directory — folder-launches (e.g. New Claude)
   *  reuse it so they keep the workspace's directory context. */
  agentCwd?: string;
  onClose: () => void;
  onLaunchApp: (app: AppEntry) => void;
  onAddTab: (
    type: PaneType,
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
  ) => void;
  onSplitPane?: (type: PaneType, shell?: string, label?: string, cwd?: string) => void;
  pluginPanes?: PluginPane[];
  onOpenPlugin?: (pane: PluginPane) => void;
  onInstallPlugin?: () => void;
  onManagePlugins?: () => void;
  /** Reusable prompts + skills to surface in the palette. */
  libraryItems?: LibraryItem[];
  /** When 'library', the palette shows only library items (quick-picker mode). */
  restrictTo?: 'library';
  /** Open the Library pane (in the global Overview workspace). */
  onOpenLibrary?: () => void;
  /** Re-open the session picker to switch/start a named workspace session. */
  onSwitchSession?: () => void;
  /** Open the Analytics pane. */
  onOpenAnalytics?: () => void;
  /** Open the Agents pane (all agents as live cards, click to watch one). */
  onOpenAgents?: () => void;
  /** Open an Inspector pane for the currently-piloted agent (plan/flows/agents/files/usage). */
  onOpenInspector?: () => void;
  /** Open a Context pane itemizing the current agent's context window. */
  onOpenContext?: () => void;
  /** Open the layout-templates manager. */
  onOpenLayouts?: () => void;
  /** Open the remote-control (phone sharing) panel. */
  onOpenRemote?: () => void;
  /** Open the Ask pane (fleet supervisor question interface). */
  onOpenAskPane?: () => void;
  onSpawnFleetAgent?: () => void;
  /** Open a file in an Editor pane (prompts for a file first). */
  onOpenFile?: () => void;
  /** Resolved keybindings (config merged with defaults) — drives shortcut badges. */
  shortcuts?: Record<string, string>;
  /** Workspace prefix combo, for rendering 'prefix …' chord badges. */
  prefix?: string;
  /** Spawn a new agent (opens the spawn dialog). */
  onSpawnAgent?: () => void;
  /** Toggle the left sidebar. */
  onToggleSidebar?: () => void;
  /** Toggle the Triage Inbox drawer. */
  onToggleInbox?: () => void;
  /** Toggle the Fleet Deck overlay. */
  onToggleFleet?: () => void;
  /** Switch between the fleet / focus UI modes. */
  onToggleUiMode?: () => void;
  /** Save the current workspace session. */
  onSaveSession?: () => void;
  /** Open the Settings pane. */
  onOpenSettings?: () => void;
  /** Toggle the keyboard-shortcuts help overlay. */
  onToggleHelp?: () => void;
  /** Re-open the first-run welcome card. */
  onShowWelcome?: () => void;
  /** In-app update status; gates/labels the update command. */
  updateStatus?: { state: string; version?: string; percent?: number };
  /** Manually check the release feed now. */
  onCheckUpdates?: () => void;
  /** Restart into a downloaded update. */
  onInstallUpdate?: () => void;
}

const CommandPalette: React.FC<CommandPaletteProps> = ({
  visible,
  apps,
  mode = 'tab',
  agentCwd,
  onClose,
  onLaunchApp,
  onAddTab,
  onSplitPane,
  pluginPanes = [],
  onOpenPlugin,
  onInstallPlugin,
  onManagePlugins,
  libraryItems = [],
  restrictTo,
  onOpenLibrary,
  onSwitchSession,
  onOpenAnalytics,
  onOpenAgents,
  onOpenInspector,
  onOpenContext,
  onOpenLayouts,
  onOpenRemote,
  onOpenAskPane,
  onSpawnFleetAgent,
  onOpenFile,
  shortcuts = {},
  prefix = 'ctrl+space',
  onSpawnAgent,
  onToggleSidebar,
  onToggleInbox,
  onToggleFleet,
  onToggleUiMode,
  onSaveSession,
  onOpenSettings,
  onToggleHelp,
  onShowWelcome,
  updateStatus,
  onCheckUpdates,
  onInstallUpdate,
}) => {
  // Current UI mode — the toggle entry's label names the TARGET mode.
  const { mode: uiMode } = useUiMode();
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [profilePicker, setProfilePicker] = useState<{
    folder: string;
    profiles: any[];
    paneType: PaneType;
  } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Library items as palette entries (always available; the only entries in
  // quick-picker mode).
  const libItems: PaletteItem[] = useMemo(
    () =>
      libraryItems.map((it) => ({
        id: `lib-${it.scope}-${it.id}`,
        name: it.title,
        description: it.description || it.kind,
        icon:
          it.kind === 'skill' ? (
            <Brain size={16} strokeWidth={1.75} />
          ) : it.kind === 'agent' ? (
            <Bot size={16} strokeWidth={1.75} />
          ) : (
            <Zap size={16} strokeWidth={1.75} />
          ),
        category: 'library' as const,
        libraryItem: it,
      })),
    [libraryItems],
  );

  // Global commands (toggles & dialogs). Each is surfaced only when its
  // handler is wired, so the palette gives a clickable, searchable, shortcut-
  // labelled path to actions that were previously keyboard-only.
  const commandActions: PaletteItem[] = useMemo(() => {
    const out: PaletteItem[] = [];
    const add = (
      id: string,
      name: string,
      description: string,
      icon: React.ReactNode,
      run: (() => void) | undefined,
      shortcut?: string,
    ) => {
      if (run) out.push({ id, name, description, icon, category: 'command', run, shortcut });
    };
    // Note: agent spawning is surfaced via the prominent "New Claude Code"
    // action (which opens the standard spawn dialog), so there's no separate
    // "Spawn Agent" command here — that would be a redundant second entry.
    add(
      'cmd-ask-fleet',
      'Ask the Fleet',
      'Pose a question to a supervisor agent',
      <Brain size={16} strokeWidth={1.75} />,
      onOpenAskPane,
    );
    add(
      'cmd-spawn-fleet',
      'Spawn Fleet Agent',
      'Start a fleet supervisor watching all agents — no question needed',
      <Brain size={16} strokeWidth={1.75} />,
      onSpawnFleetAgent,
    );
    add(
      'cmd-toggle-sidebar',
      'Toggle Sidebar',
      'Show or hide the agent sidebar',
      <Columns3 size={16} strokeWidth={1.75} />,
      onToggleSidebar,
      'toggle-sidebar',
    );
    add(
      'cmd-toggle-inbox',
      'Toggle Inbox',
      'Open or close the triage inbox',
      <Star size={16} strokeWidth={1.75} />,
      onToggleInbox,
      'toggle-inbox',
    );
    add(
      'cmd-toggle-fleet',
      'Toggle Fleet Deck',
      'Cross-agent radar overlay',
      <LayoutGrid size={16} strokeWidth={1.75} />,
      onToggleFleet,
      'toggle-fleet',
    );
    add(
      'cmd-toggle-ui-mode',
      uiMode === 'focus' ? 'Switch to fleet mode' : 'Switch to focus mode',
      uiMode === 'focus'
        ? 'Bring back the full mission-control chrome'
        : 'Minimal chrome — rail sidebar, no inspector rail or Fleet Deck',
      <Columns3 size={16} strokeWidth={1.75} />,
      onToggleUiMode,
      'toggle-ui-mode',
    );
    add(
      'cmd-analytics',
      'Analytics',
      'Session usage & cost',
      <BarChart3 size={16} strokeWidth={1.75} />,
      onOpenAnalytics,
    );
    add(
      'cmd-agents',
      'Agents Monitor',
      'All agents as live cards — click one to watch it in a pane',
      <LayoutGrid size={16} strokeWidth={1.75} />,
      onOpenAgents,
    );
    add(
      'cmd-inspector',
      'Open Inspector Pane',
      'Plan · flows · agents · files · usage for the current agent, as a live pane',
      <Columns3 size={16} strokeWidth={1.75} />,
      onOpenInspector,
    );
    add(
      'cmd-context',
      'Context Window',
      'What occupies the current agent’s context — memories, skills, MCP, tools',
      <PaneIcon type="context" size={16} />,
      onOpenContext,
    );
    add(
      'cmd-layouts',
      'Layouts…',
      'Apply or save a layout template',
      <LayoutGrid size={16} strokeWidth={1.75} />,
      onOpenLayouts,
    );
    add(
      'cmd-switch-session',
      'Switch Session…',
      'Open another saved workspace session',
      <FolderOpen size={16} strokeWidth={1.75} />,
      onSwitchSession,
    );
    add(
      'cmd-save-session',
      'Save Session',
      'Persist the current workspace',
      <FolderOpen size={16} strokeWidth={1.75} />,
      onSaveSession,
      'save-session',
    );
    add(
      'cmd-remote',
      'Remote Control…',
      'Share this workspace to your phone',
      <Smartphone size={16} strokeWidth={1.75} />,
      onOpenRemote,
    );
    add(
      'cmd-manage-plugins',
      'Manage Plugins…',
      'Installed plugins & sidecars',
      <Blocks size={16} strokeWidth={1.75} />,
      onManagePlugins,
    );
    add(
      'cmd-install-plugin',
      'Install Plugin…',
      'Install a plugin from GitHub',
      <Plus size={16} strokeWidth={2} />,
      onInstallPlugin,
    );
    add(
      'cmd-settings',
      'Settings',
      'App preferences',
      <Settings size={16} strokeWidth={1.75} />,
      onOpenSettings,
      'settings',
    );
    add(
      'cmd-help',
      'Keyboard Shortcuts',
      'Show the shortcuts reference',
      <Brain size={16} strokeWidth={1.75} />,
      onToggleHelp,
      'toggle-help',
    );
    add(
      'cmd-welcome',
      'Show Welcome',
      'Replay the first-run welcome & orientation card',
      <Sparkles size={16} strokeWidth={1.75} />,
      onShowWelcome,
    );
    // Updates: one entry whose action tracks the updater state. Hidden where
    // there is no update feed (dev builds, the web mirror).
    if (updateStatus && updateStatus.state !== 'unsupported') {
      if (updateStatus.state === 'downloaded') {
        add(
          'cmd-update',
          `Restart to Update${updateStatus.version ? ` (v${updateStatus.version})` : ''}`,
          'A new version is downloaded — restart to apply it',
          <RefreshCw size={16} strokeWidth={1.75} />,
          onInstallUpdate,
        );
      } else {
        add(
          'cmd-update',
          'Check for Updates',
          'Look for a newer Workspacer release now',
          <RefreshCw size={16} strokeWidth={1.75} />,
          onCheckUpdates,
        );
      }
    }
    return out;
  }, [
    onOpenAskPane,
    onSpawnFleetAgent,
    onToggleSidebar,
    onToggleInbox,
    onToggleFleet,
    onToggleUiMode,
    uiMode,
    onOpenAnalytics,
    onOpenAgents,
    onOpenInspector,
    onOpenContext,
    onOpenLayouts,
    onSwitchSession,
    onSaveSession,
    onOpenRemote,
    onManagePlugins,
    onInstallPlugin,
    onOpenSettings,
    onToggleHelp,
    onShowWelcome,
    updateStatus,
    onCheckUpdates,
    onInstallUpdate,
  ]);

  // "Keyboard Shortcuts" is always pinned at the top (regardless of query) so
  // help is immediately discoverable. It is sourced from commandActions so it
  // still respects whether the handler is wired.
  const pinnedHelpItem: PaletteItem | null = useMemo(() => {
    if (restrictTo === 'library') return null;
    return commandActions.find((a) => a.id === 'cmd-help') ?? null;
  }, [commandActions, restrictTo]);

  // Build unified item list. Order MUST match the visual group order rendered
  // below (actions, apps, commands, plugins, library): keyboard nav advances
  // selectedIndex through this array while each row highlights by its index in
  // it, so any divergence makes Arrow keys jump rows instead of stepping.
  const items: PaletteItem[] = useMemo(() => {
    if (restrictTo === 'library') return libItems;
    return [
      ...builtInActions,
      ...apps.map((app, i) => ({
        id: `app-${i}`,
        name: app.name,
        description: app.url,
        icon: userIcon(app.icon, Globe),
        category: 'app' as const,
        url: app.url,
        app,
      })),
      ...commandActions.filter((a) => a.id !== 'cmd-help'),
      ...pluginPanes.map((p) => ({
        id: `plugin-${p.type}`,
        name: p.title,
        description: p.pluginId,
        icon: userIcon(p.icon, Puzzle),
        category: 'plugin' as const,
        pluginPane: p,
      })),
      ...libItems,
    ];
  }, [apps, pluginPanes, libItems, restrictTo, commandActions]);

  // Remember what had focus before we opened so we can hand it back — but only
  // when the palette is *dismissed* (Escape / click-away). When the user picks
  // an action, the thing it opens (a new pane / dialog) owns focus instead.
  const prevFocusRef = useRef<HTMLElement | null>(null);
  const dismissedRef = useRef(false);

  // Dismiss without performing an action — restores the prior focus on close.
  const dismiss = useCallback(() => {
    dismissedRef.current = true;
    onClose();
  }, [onClose]);

  useEffect(() => {
    if (visible) {
      prevFocusRef.current = document.activeElement as HTMLElement | null;
      dismissedRef.current = false;
      setQuery('');
      setSelectedIndex(0);
      setProfilePicker(null);
      setTimeout(() => inputRef.current?.focus(), 0);
    } else {
      const prev = prevFocusRef.current;
      const wasDismissed = dismissedRef.current;
      prevFocusRef.current = null;
      dismissedRef.current = false;
      // Only restore on a pure dismiss, and only if focus is otherwise stranded
      // (on <body> or the now-removed search input) — never yank it back from a
      // pane/dialog that an action just opened.
      if (!wasDismissed) return;
      requestAnimationFrame(() => {
        const active = document.activeElement;
        const stranded = !active || active === document.body || active === inputRef.current;
        if (stranded && prev && typeof prev.focus === 'function' && document.contains(prev)) {
          prev.focus();
        }
      });
    }
  }, [visible]);

  const q = query.toLowerCase();
  const filtered = items.filter(
    (item) =>
      item.name.toLowerCase().includes(q) || (item.description ?? '').toLowerCase().includes(q),
  );

  // Clamp selected index when results change
  useEffect(() => {
    if (selectedIndex >= filtered.length) {
      setSelectedIndex(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, selectedIndex]);

  const activateItem = useCallback(
    async (item: PaletteItem, libraryAction?: LibraryAction) => {
      if (item.run) {
        item.run();
        onClose();
        return;
      }
      // "New Claude Code" spawns an agent — route it through the SAME standard
      // spawn dialog as the sidebar + and the spawn hotkey, so spawning is
      // consistent regardless of where it's triggered from.
      if (item.id === 'new-claude' && onSpawnAgent) {
        onSpawnAgent();
        onClose();
        return;
      }
      if (item.id === 'open-library' && onOpenLibrary) {
        onOpenLibrary();
        onClose();
        return;
      }
      if (item.id === 'new-editor' && onOpenFile) {
        // Opening a file prompts for one (native dialog / host browser), so it
        // can't go through the standard onAddTab path.
        onOpenFile();
        onClose();
        return;
      }
      if (item.category === 'library' && item.libraryItem) {
        runLibraryItem(item.libraryItem, libraryAction);
        onClose();
        return;
      }
      if (item.category === 'plugin' && item.pluginPane) {
        onOpenPlugin?.(item.pluginPane);
      } else if (item.category === 'app' && item.app) {
        onLaunchApp(item.app);
      } else if (item.paneType) {
        if (item.pickFolder) {
          // In a workspace, reuse the active agent's directory (like New Terminal
          // already does) so launches keep their directory context. Only prompt
          // for a folder when there's no agent context (e.g. the Overview).
          const folder = agentCwd || (await window.electronAPI.pickFolder());
          if (!folder) return;
          // Check for profiles — show picker inline if multiple
          try {
            const profiles = await window.electronAPI.claudeProfilesList();
            if (profiles.length > 1) {
              setProfilePicker({ folder, profiles, paneType: item.paneType });
              return;
            }
          } catch {}
          if (mode === 'split' && onSplitPane) {
            onSplitPane(item.paneType, undefined, undefined, folder);
          } else {
            onAddTab(item.paneType, undefined, undefined, folder);
          }
        } else {
          // Carry the active agent's cwd explicitly so a new pane (e.g. a terminal)
          // lands in the agent's directory regardless of timing.
          if (mode === 'split' && onSplitPane) {
            onSplitPane(item.paneType, undefined, undefined, agentCwd || undefined);
          } else {
            onAddTab(item.paneType, undefined, undefined, agentCwd || undefined);
          }
        }
      }
      onClose();
    },
    [
      onLaunchApp,
      onAddTab,
      onSplitPane,
      onOpenPlugin,
      onClose,
      mode,
      onOpenLibrary,
      onSpawnAgent,
      agentCwd,
    ],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        dismiss();
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => (filtered.length === 0 ? 0 : Math.min(i + 1, filtered.length - 1)));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const item = filtered[selectedIndex];
        if (item) {
          // For library items: Enter = default, ⌘/Ctrl+Enter = spawn, Alt+Enter = copy.
          const libAction: LibraryAction | undefined =
            item.category === 'library'
              ? e.metaKey || e.ctrlKey
                ? 'spawn'
                : e.altKey
                  ? 'copy'
                  : undefined
              : undefined;
          activateItem(item, libAction);
        }
      }
    },
    [filtered, selectedIndex, dismiss, activateItem],
  );

  if (!visible) return null;

  // Profile picker sub-view
  if (profilePicker) {
    return (
      <div
        style={{
          position: 'fixed',
          inset: 0,
          backgroundColor: 'var(--wks-overlay)',
          display: 'flex',
          justifyContent: 'center',
          paddingTop: '15vh',
          zIndex: 2000,
        }}
        onClick={() => {
          setProfilePicker(null);
          dismiss();
        }}
      >
        <div
          style={{
            backgroundColor: 'var(--wks-glass-strong)',
            backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
            WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 'var(--wks-radius-lg)',
            width: 'min(340px, 94vw)',
            boxSizing: 'border-box',
            maxHeight: 320,
            overflow: 'hidden',
            boxShadow:
              '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
            display: 'flex',
            flexDirection: 'column',
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div
            style={{
              padding: '12px 16px 8px',
              fontSize: '0.75rem',
              fontWeight: 600,
              color: 'var(--wks-text-secondary)',
              borderBottom: '1px solid var(--wks-border)',
            }}
          >
            Select Claude Profile
          </div>
          <div style={{ overflow: 'auto', padding: '4px 0' }}>
            {profilePicker.profiles.map((p: any) => (
              <div
                key={p.id}
                onClick={() => {
                  const { folder, paneType } = profilePicker;
                  setProfilePicker(null);
                  onAddTab(paneType, undefined, undefined, folder, p.id);
                  onClose();
                }}
                style={{
                  padding: '8px 16px',
                  cursor: 'pointer',
                  fontSize: '0.72rem',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
              >
                <span
                  style={{
                    color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)',
                    fontSize: '0.7rem',
                  }}
                >
                  {p.isDefault ? '\u2666' : '\u25CB'}
                </span>
                <div style={{ flex: 1 }}>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 500 }}>{p.name}</div>
                  {p.extraArgs?.length > 0 && (
                    <div
                      style={{
                        fontSize: '0.55rem',
                        color: 'var(--wks-text-faint)',
                        fontFamily: 'var(--wks-font-mono)',
                      }}
                    >
                      {p.extraArgs.join(' ')}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    );
  }

  // Group filtered items by category for visual separation
  const actions = filtered.filter((i) => i.category === 'action');
  const commandItems = filtered.filter((i) => i.category === 'command');
  const appItems = filtered.filter((i) => i.category === 'app');
  const pluginItems = filtered.filter((i) => i.category === 'plugin');
  const libraryFiltered = filtered.filter((i) => i.category === 'library');

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: 'var(--wks-overlay)',
        display: 'flex',
        justifyContent: 'center',
        paddingTop: '15vh',
        zIndex: 2000,
      }}
      onClick={dismiss}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          width: 'min(440px, 94vw)',
          boxSizing: 'border-box',
          maxHeight: '420px',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          boxShadow:
            '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search input — leading pack search glyph */}
        <div style={{ padding: '12px 12px 8px', position: 'relative' }}>
          <IconSearch
            size={15}
            strokeWidth={2}
            accent="currentColor"
            style={{
              position: 'absolute',
              left: '22px',
              top: '50%',
              transform: 'translateY(-50%)',
              color: 'var(--wks-text-faint)',
              pointerEvents: 'none',
            }}
          />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder={
              restrictTo === 'library' ? 'Insert a prompt or skill…' : 'Search actions and apps...'
            }
            spellCheck={false}
            style={{
              width: '100%',
              height: '32px',
              padding: '0 12px 0 34px',
              fontSize: '0.8rem',
              fontFamily: 'inherit',
              backgroundColor: 'var(--wks-bg-input)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-sm)',
              outline: 'none',
              boxSizing: 'border-box',
            }}
            onFocus={(e) => {
              e.currentTarget.style.borderColor = 'var(--wks-accent)';
            }}
            onBlur={(e) => {
              e.currentTarget.style.borderColor = 'var(--wks-border-input)';
            }}
          />
        </div>

        {/* Results */}
        <div style={{ overflow: 'auto', padding: '0 4px 8px' }}>
          {/* Pinned shortcut-help entry — always visible at the top (outside the filtered list) */}
          {pinnedHelpItem && !restrictTo && (
            <PaletteRow
              item={pinnedHelpItem}
              shortcut={shortcutFor(pinnedHelpItem.shortcut, shortcuts, prefix)}
              selected={false}
              onActivate={() => activateItem(pinnedHelpItem)}
              onHover={() => {}}
            />
          )}

          {filtered.length === 0 && (
            <div
              style={{
                padding: '12px',
                fontSize: '0.7rem',
                color: 'var(--wks-text-faint)',
                textAlign: 'center',
              }}
            >
              {query ? (
                <>
                  No results for &ldquo;{query}&rdquo;
                  {onOpenSettings && (
                    <>
                      {' · '}
                      <span
                        role="button"
                        tabIndex={0}
                        onClick={() => {
                          onOpenSettings();
                          onClose();
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            onOpenSettings();
                            onClose();
                          }
                        }}
                        style={{
                          color: 'var(--wks-accent)',
                          cursor: 'pointer',
                          textDecoration: 'underline',
                        }}
                      >
                        Open Settings to add custom apps
                      </span>
                    </>
                  )}
                </>
              ) : (
                'No results found'
              )}
            </div>
          )}

          {actions.length > 0 && appItems.length > 0 && (
            <div
              style={{
                padding: '4px 12px 2px',
                fontSize: '0.55rem',
                color: 'var(--wks-text-disabled)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              Panes
            </div>
          )}

          {actions.map((item) => {
            const globalIdx = filtered.indexOf(item);
            return (
              <PaletteRow
                key={item.id}
                item={item}
                shortcut={shortcutFor(item.shortcut, shortcuts, prefix)}
                selected={globalIdx === selectedIndex}
                onActivate={() => activateItem(item)}
                onHover={() => setSelectedIndex(globalIdx)}
              />
            );
          })}

          {appItems.length > 0 && (
            <div
              style={{
                padding: '6px 12px 2px',
                fontSize: '0.55rem',
                color: 'var(--wks-text-disabled)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              Apps
            </div>
          )}

          {appItems.map((item) => {
            const globalIdx = filtered.indexOf(item);
            return (
              <PaletteRow
                key={item.id}
                item={item}
                selected={globalIdx === selectedIndex}
                onActivate={() => activateItem(item)}
                onHover={() => setSelectedIndex(globalIdx)}
              />
            );
          })}

          {commandItems.length > 0 && (
            <div
              style={{
                padding: '6px 12px 2px',
                fontSize: '0.55rem',
                color: 'var(--wks-text-disabled)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              Commands
            </div>
          )}

          {commandItems.map((item) => {
            const globalIdx = filtered.indexOf(item);
            return (
              <PaletteRow
                key={item.id}
                item={item}
                shortcut={shortcutFor(item.shortcut, shortcuts, prefix)}
                selected={globalIdx === selectedIndex}
                onActivate={() => activateItem(item)}
                onHover={() => setSelectedIndex(globalIdx)}
              />
            );
          })}

          {pluginItems.length > 0 && (
            <div
              style={{
                padding: '6px 12px 2px',
                fontSize: '0.55rem',
                color: 'var(--wks-text-disabled)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              Plugins
            </div>
          )}

          {pluginItems.map((item) => {
            const globalIdx = filtered.indexOf(item);
            return (
              <PaletteRow
                key={item.id}
                item={item}
                selected={globalIdx === selectedIndex}
                onActivate={() => activateItem(item)}
                onHover={() => setSelectedIndex(globalIdx)}
              />
            );
          })}

          {libraryFiltered.length > 0 && restrictTo !== 'library' && (
            <div
              style={{
                padding: '6px 12px 2px',
                fontSize: '0.55rem',
                color: 'var(--wks-text-disabled)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              Prompts &amp; Skills
            </div>
          )}

          {libraryFiltered.map((item) => {
            const globalIdx = filtered.indexOf(item);
            return (
              <PaletteRow
                key={item.id}
                item={item}
                selected={globalIdx === selectedIndex}
                onActivate={() => activateItem(item)}
                onHover={() => setSelectedIndex(globalIdx)}
              />
            );
          })}
        </div>

        {(restrictTo === 'library' || libraryFiltered.length > 0) && (
          <div
            style={{
              padding: '6px 14px',
              borderTop: '1px solid var(--wks-border)',
              fontSize: '0.58rem',
              color: 'var(--wks-text-faint)',
              display: 'flex',
              gap: 12,
            }}
          >
            <span>
              <b>Enter</b> insert
            </span>
            <span>
              <b>⌘/Ctrl+Enter</b> spawn
            </span>
            <span>
              <b>Alt+Enter</b> copy
            </span>
          </div>
        )}
      </div>
    </div>
  );
};

// ── Row component ──

const PaletteRow: React.FC<{
  item: PaletteItem;
  selected: boolean;
  onActivate: () => void;
  onHover: () => void;
  shortcut?: string;
}> = ({ item, selected, onActivate, onHover, shortcut }) => (
  <div
    onClick={onActivate}
    onMouseEnter={onHover}
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: '10px',
      padding: '7px 12px',
      margin: '0 4px',
      borderRadius: 'var(--wks-radius-sm)',
      cursor: 'pointer',
      backgroundColor: selected ? 'var(--wks-bg-selected)' : 'transparent',
    }}
  >
    <span
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: '20px',
        flexShrink: 0,
        color: 'var(--wks-text-tertiary)',
      }}
    >
      {item.icon}
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: '0.75rem', color: 'var(--wks-text-primary)', fontWeight: 500 }}>
        {item.name}
      </div>
      {item.description && (
        <div
          style={{
            fontSize: '0.6rem',
            color: 'var(--wks-text-faint)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {item.description}
        </div>
      )}
    </div>
    {shortcut && <Kbd>{shortcut}</Kbd>}
  </div>
);

/** Compact keyboard-shortcut chip shown on the right of a palette row. */
const Kbd: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span
    style={{
      flexShrink: 0,
      fontSize: '0.6rem',
      fontFamily: 'var(--claude-mono-font, monospace)',
      color: 'var(--wks-text-tertiary)',
      background: 'var(--wks-bg-input)',
      border: '1px solid var(--wks-border-input)',
      borderRadius: 4,
      padding: '1px 6px',
      whiteSpace: 'nowrap',
    }}
  >
    {children}
  </span>
);

export default CommandPalette;
