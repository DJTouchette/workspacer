import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { AppEntry } from '../hooks/useConfig';
import type { PaneType } from '../types/pane';
import type { PluginPane } from '../types/plugin';
import type { LibraryItem, LibraryAction } from '../types/library';
import { runLibraryItem } from '../lib/libraryBus';

// ── Unified palette item ──

export interface PaletteItem {
  id: string;
  name: string;
  description?: string;
  icon: string;
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
  { id: 'new-claude', name: 'New Claude Code', description: 'AI-powered coding assistant', icon: '\u2666', category: 'action', paneType: 'claude', pickFolder: true },
  { id: 'new-terminal', name: 'New Terminal', description: 'Shell terminal', icon: '>_', category: 'action', paneType: 'terminal' },
  { id: 'new-browser', name: 'New Browser', description: 'Web browser tab', icon: '\u{1F310}', category: 'action', paneType: 'browser' },
  { id: 'new-review', name: 'Review Changes', description: 'Git diff & status for this agent', icon: '\u{1F50D}', category: 'action', paneType: 'review' },
  { id: 'new-notes', name: 'Notes', description: 'Markdown scratchpad', icon: '\u{1F4DD}', category: 'action', paneType: 'notes' },
  { id: 'open-library', name: 'Library', description: 'Reusable prompts & skills', icon: '⚡', category: 'action', paneType: 'library' },
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
}

const CommandPalette: React.FC<CommandPaletteProps> = ({ visible, apps, mode = 'tab', onClose, onLaunchApp, onAddTab, onSplitPane, pluginPanes = [], onOpenPlugin, onInstallPlugin, onManagePlugins, libraryItems = [], restrictTo, onOpenLibrary, onSwitchSession }) => {
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
    icon: it.kind === 'skill' ? '\u{1F9E0}' : it.kind === 'agent' ? '\u{1F916}' : '⚡',
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
        icon: app.icon || '\u{1F310}',
        category: 'app' as const,
        url: app.url,
        app,
      })),
      ...pluginPanes.map((p) => ({
        id: `plugin-${p.type}`,
        name: p.title,
        description: p.pluginId,
        icon: p.icon || '\u{1F9E9}',
        category: 'plugin' as const,
        pluginPane: p,
      })),
      ...libItems,
    ];
  }, [apps, pluginPanes, libItems, restrictTo]);

  // Focus input and reset state when opening
  useEffect(() => {
    if (visible) {
      setQuery('');
      setSelectedIndex(0);
      setProfilePicker(null);
      setTimeout(() => inputRef.current?.focus(), 0);
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
      onClose();
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
  }, [filtered, selectedIndex, onClose, activateItem]);

  if (!visible) return null;

  // Profile picker sub-view
  if (profilePicker) {
    return (
      <div
        style={{ position: 'fixed', inset: 0, backgroundColor: 'var(--wks-overlay)', display: 'flex', justifyContent: 'center', paddingTop: '15vh', zIndex: 2000 }}
        onClick={() => { setProfilePicker(null); onClose(); }}
      >
        <div
          style={{ backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-input)', borderRadius: 8, width: 340, maxHeight: 320, overflow: 'hidden', boxShadow: '0 8px 32px var(--wks-shadow)', display: 'flex', flexDirection: 'column' }}
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
      onClick={onClose}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-bg-raised)',
          border: '1px solid var(--wks-border-input)',
          borderRadius: '8px',
          width: '440px',
          maxHeight: '420px',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          boxShadow: '0 8px 32px var(--wks-shadow)',
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
              borderRadius: '5px',
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

          {restrictTo !== 'library' && onSwitchSession && (
            <>
              <div style={{ padding: '6px 12px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                Session
              </div>
              <div
                onClick={() => { onSwitchSession(); }}
                style={{
                  display: 'flex', alignItems: 'center', gap: '10px',
                  padding: '7px 12px', margin: '0 4px', borderRadius: '5px', cursor: 'pointer',
                  color: 'var(--wks-text-muted)',
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
              >
                <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center', flexShrink: 0 }}>🗂️</span>
                <span style={{ fontSize: '0.78rem' }}>Switch session…</span>
              </div>
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
                padding: '7px 12px', margin: '0 4px', borderRadius: '5px', cursor: 'pointer',
                color: 'var(--wks-text-muted)',
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
            >
              <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center', flexShrink: 0 }}>🧰</span>
              <span style={{ fontSize: '0.78rem' }}>Manage plugins…</span>
            </div>
          )}

          {onInstallPlugin && (
            <div
              onClick={() => { onInstallPlugin(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: '10px',
                padding: '7px 12px', margin: '0 4px', borderRadius: '5px', cursor: 'pointer',
                color: 'var(--wks-text-muted)',
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
            >
              <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center', flexShrink: 0 }}>＋</span>
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
      borderRadius: '5px',
      cursor: 'pointer',
      backgroundColor: selected ? 'var(--wks-bg-selected)' : 'transparent',
    }}
  >
    <span style={{ fontSize: '0.85rem', width: '20px', textAlign: 'center', flexShrink: 0 }}>
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

export default CommandPalette;
