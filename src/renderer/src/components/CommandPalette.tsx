import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { AppEntry } from '../hooks/useConfig';
import type { PaneType } from '../types/pane';
import type { PluginPane } from '../types/plugin';
import type { LibraryItem, LibraryAction } from '../types/library';
import { runLibraryItem } from '../lib/libraryBus';
import {
  PaneIcon, Globe, Puzzle, Blocks, Brain, Bot, Zap, BarChart3, LayoutGrid, FolderOpen, Plus,
  type LucideIcon,
} from './icons';

// ── Unified palette item ──

/** Render a user-supplied icon string: a URL becomes a favicon-style image,
 *  anything else falls back to a thin-stroke lucide glyph (no emoji). */
function userIcon(raw: string | undefined, Fallback: LucideIcon): React.ReactNode {
  if (raw && /^https?:\/\//.test(raw)) {
    return <img src={raw} width={16} height={16} style={{ borderRadius: 3, objectFit: 'contain' }} alt="" />;
  }
  return <Fallback size={16} strokeWidth={1.75} />;
}

export interface PaletteItem {
  id: string;
  name: string;
  description?: string;
  icon: React.ReactNode;
  category: 'action' | 'app' | 'plugin' | 'library';
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
}

// ── Built-in actions ──

export const builtInActions: PaletteItem[] = [
  { id: 'new-claude', name: 'New Claude Code', description: 'AI-powered coding assistant', icon: <PaneIcon type="claude" size={16} />, category: 'action', paneType: 'claude', pickFolder: true },
  { id: 'new-terminal', name: 'New Terminal', description: 'Shell terminal', icon: <PaneIcon type="terminal" size={16} />, category: 'action', paneType: 'terminal' },
  { id: 'new-browser', name: 'New Browser', description: 'Web browser tab', icon: <PaneIcon type="browser" size={16} />, category: 'action', paneType: 'browser' },
  { id: 'new-review', name: 'Review Changes', description: 'Git diff & status for this agent', icon: <PaneIcon type="review" size={16} />, category: 'action', paneType: 'review' },
  { id: 'new-notes', name: 'Notes', description: 'Markdown scratchpad', icon: <PaneIcon type="notes" size={16} />, category: 'action', paneType: 'notes' },
  { id: 'open-library', name: 'Library', description: 'Reusable prompts & skills', icon: <PaneIcon type="library" size={16} />, category: 'action', paneType: 'library' },
];

// ── Props ──

interface CommandPaletteProps {
  visible: boolean;
  apps: AppEntry[];
  mode?: 'tab' | 'split';
  onClose: () => void;
  onLaunchApp: (app: AppEntry) => void;
  onAddTab: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string) => void;
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
  /** Open the layout-templates manager. */
  onOpenLayouts?: () => void;
}

const CommandPalette: React.FC<CommandPaletteProps> = ({ visible, apps, mode = 'tab', onClose, onLaunchApp, onAddTab, onSplitPane, pluginPanes = [], onOpenPlugin, onInstallPlugin, onManagePlugins, libraryItems = [], restrictTo, onOpenLibrary, onSwitchSession, onOpenAnalytics, onOpenLayouts }) => {
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [profilePicker, setProfilePicker] = useState<{ folder: string; profiles: any[]; paneType: PaneType } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Library items as palette entries (always available; the only entries in
  // quick-picker mode).
  const libItems: PaletteItem[] = useMemo(() => libraryItems.map((it) => ({
    id: `lib-${it.scope}-${it.id}`,
    name: it.title,
    description: it.description || it.kind,
    icon: it.kind === 'skill' ? <Brain size={16} strokeWidth={1.75} /> : it.kind === 'agent' ? <Bot size={16} strokeWidth={1.75} /> : <Zap size={16} strokeWidth={1.75} />,
    category: 'library' as const,
    libraryItem: it,
  })), [libraryItems]);

  // Build unified item list: actions, then apps, then plugin panes, then library.
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
  }, [apps, pluginPanes, libItems, restrictTo]);

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
  const filtered = items.filter(item =>
    item.name.toLowerCase().includes(q) ||
    (item.description ?? '').toLowerCase().includes(q)
  );

  // Clamp selected index when results change
  useEffect(() => {
    if (selectedIndex >= filtered.length) {
      setSelectedIndex(Math.max(0, filtered.length - 1));
    }
  }, [filtered.length, selectedIndex]);

  const activateItem = useCallback(async (item: PaletteItem, libraryAction?: LibraryAction) => {
    if (item.id === 'open-library' && onOpenLibrary) {
      onOpenLibrary();
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
        const folder = await window.electronAPI.pickFolder();
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
        if (mode === 'split' && onSplitPane) {
          onSplitPane(item.paneType);
        } else {
          onAddTab(item.paneType);
        }
      }
    }
    onClose();
  }, [onLaunchApp, onAddTab, onSplitPane, onOpenPlugin, onClose, mode, onOpenLibrary]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      dismiss();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
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
            ? (e.metaKey || e.ctrlKey ? 'spawn' : e.altKey ? 'copy' : undefined)
            : undefined;
        activateItem(item, libAction);
      }
    }
  }, [filtered, selectedIndex, dismiss, activateItem]);

  if (!visible) return null;

  // Profile picker sub-view
  if (profilePicker) {
    return (
      <div
        style={{ position: 'fixed', inset: 0, backgroundColor: 'var(--wks-overlay)', display: 'flex', justifyContent: 'center', paddingTop: '15vh', zIndex: 2000 }}
        onClick={() => { setProfilePicker(null); dismiss(); }}
      >
        <div
          style={{ backgroundColor: 'var(--wks-glass-strong)', backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)', WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)', border: '1px solid var(--wks-glass-border)', borderRadius: 'var(--wks-radius-lg)', width: 340, maxHeight: 320, overflow: 'hidden', boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)', display: 'flex', flexDirection: 'column' }}
          onClick={e => e.stopPropagation()}
        >
          <div style={{ padding: '12px 16px 8px', fontSize: '0.75rem', fontWeight: 600, color: 'var(--wks-text-secondary)', borderBottom: '1px solid var(--wks-border)' }}>
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
                style={{ padding: '8px 16px', cursor: 'pointer', fontSize: '0.72rem', display: 'flex', alignItems: 'center', gap: 8 }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
              >
                <span style={{ color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)', fontSize: '0.7rem' }}>
                  {p.isDefault ? '\u2666' : '\u25CB'}
                </span>
                <div style={{ flex: 1 }}>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 500 }}>{p.name}</div>
                  {p.extraArgs?.length > 0 && (
                    <div style={{ fontSize: '0.55rem', color: 'var(--wks-text-faint)', fontFamily: 'monospace' }}>{p.extraArgs.join(' ')}</div>
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
  const actions = filtered.filter(i => i.category === 'action');
  const appItems = filtered.filter(i => i.category === 'app');
  const pluginItems = filtered.filter(i => i.category === 'plugin');
  const libraryFiltered = filtered.filter(i => i.category === 'library');

  return (
    <div
      style={{
        position: 'fixed',
        top: 0, left: 0, right: 0, bottom: 0,
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
          width: '440px',
          maxHeight: '420px',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          boxShadow: '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search input */}
        <div style={{ padding: '12px 12px 8px' }}>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => { setQuery(e.target.value); setSelectedIndex(0); }}
            onKeyDown={handleKeyDown}
            placeholder={restrictTo === 'library' ? 'Insert a prompt or skill…' : 'Search actions and apps...'}
            spellCheck={false}
            style={{
              width: '100%',
              height: '32px',
              padding: '0 12px',
              fontSize: '0.8rem',
              fontFamily: 'inherit',
              backgroundColor: 'var(--wks-bg-input)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-sm)',
              outline: 'none',
              boxSizing: 'border-box',
            }}
            onFocus={(e) => { e.currentTarget.style.borderColor = 'var(--wks-accent)'; }}
            onBlur={(e) => { e.currentTarget.style.borderColor = 'var(--wks-border-input)'; }}
          />
        </div>

        {/* Results */}
        <div style={{ overflow: 'auto', padding: '0 4px 8px' }}>
          {filtered.length === 0 && (
            <div style={{ padding: '12px', fontSize: '0.7rem', color: 'var(--wks-text-faint)', textAlign: 'center' }}>
              No results found
            </div>
          )}

          {actions.length > 0 && appItems.length > 0 && (
            <div style={{ padding: '4px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
              Panes
            </div>
          )}

          {actions.map(item => {
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

          {appItems.length > 0 && (
            <div style={{ padding: '6px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
              Apps
            </div>
          )}

          {appItems.map(item => {
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

          {restrictTo !== 'library' && (onSwitchSession || onOpenAnalytics || onOpenLayouts) && (
            <>
              <div style={{ padding: '6px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                Workspace
              </div>
              {onOpenAnalytics && <CommandRow icon={<BarChart3 size={16} strokeWidth={1.75} />} label="Analytics" onClick={onOpenAnalytics} />}
              {onOpenLayouts && <CommandRow icon={<LayoutGrid size={16} strokeWidth={1.75} />} label="Layouts…" onClick={onOpenLayouts} />}
              {onSwitchSession && <CommandRow icon={<FolderOpen size={16} strokeWidth={1.75} />} label="Switch session…" onClick={onSwitchSession} />}
            </>
          )}

          {(pluginItems.length > 0 || onInstallPlugin || onManagePlugins) && (
            <div style={{ padding: '6px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
              Plugins
            </div>
          )}

          {onManagePlugins && (
            <div
              onClick={() => { onManagePlugins(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: '10px',
                padding: '7px 12px', margin: '0 4px', borderRadius: 'var(--wks-radius-sm)', cursor: 'pointer',
                color: 'var(--wks-text-muted)',
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
            >
              <span style={{ display: 'flex', justifyContent: 'center', width: '20px', flexShrink: 0 }}><Blocks size={16} strokeWidth={1.75} /></span>
              <span style={{ fontSize: '0.78rem' }}>Manage plugins…</span>
            </div>
          )}

          {onInstallPlugin && (
            <div
              onClick={() => { onInstallPlugin(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: '10px',
                padding: '7px 12px', margin: '0 4px', borderRadius: 'var(--wks-radius-sm)', cursor: 'pointer',
                color: 'var(--wks-text-muted)',
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
            >
              <span style={{ display: 'flex', justifyContent: 'center', width: '20px', flexShrink: 0 }}><Plus size={16} strokeWidth={2} /></span>
              <span style={{ fontSize: '0.78rem' }}>Install from GitHub…</span>
            </div>
          )}

          {pluginItems.map(item => {
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
            <div style={{ padding: '6px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
              Prompts &amp; Skills
            </div>
          )}

          {libraryFiltered.map(item => {
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
          <div style={{ padding: '6px 14px', borderTop: '1px solid var(--wks-border)', fontSize: '0.58rem', color: 'var(--wks-text-faint)', display: 'flex', gap: 12 }}>
            <span><b>Enter</b> insert</span>
            <span><b>⌘/Ctrl+Enter</b> spawn</span>
            <span><b>Alt+Enter</b> copy</span>
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
}> = ({ item, selected, onActivate, onHover }) => (
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
    <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: '20px', flexShrink: 0, color: 'var(--wks-text-tertiary)' }}>
      {item.icon}
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontSize: '0.75rem', color: 'var(--wks-text-primary)', fontWeight: 500 }}>
        {item.name}
      </div>
      {item.description && (
        <div style={{
          fontSize: '0.6rem',
          color: 'var(--wks-text-faint)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}>
          {item.description}
        </div>
      )}
    </div>
  </div>
);

const CommandRow: React.FC<{ icon: React.ReactNode; label: string; onClick: () => void }> = ({ icon, label, onClick }) => (
  <div
    onClick={onClick}
    style={{
      display: 'flex', alignItems: 'center', gap: '10px',
      padding: '7px 12px', margin: '0 4px', borderRadius: 'var(--wks-radius-sm)', cursor: 'pointer',
      color: 'var(--wks-text-muted)',
    }}
    onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
    onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
  >
    <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: '20px', flexShrink: 0 }}>{icon}</span>
    <span style={{ fontSize: '0.78rem' }}>{label}</span>
  </div>
);

export default CommandPalette;
