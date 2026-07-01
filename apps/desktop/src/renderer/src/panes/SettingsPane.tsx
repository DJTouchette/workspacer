import React, { useState, useMemo, useRef, useCallback, useEffect } from 'react';
import { useConfig } from '../hooks/useConfig';
import AppearanceSection from '../components/settings/AppearanceSection';
import LayoutSection from '../components/settings/LayoutSection';
import TerminalSection from '../components/settings/TerminalSection';
import KeybindingsSection from '../components/settings/KeybindingsSection';
import NotificationsSection from '../components/settings/NotificationsSection';
import SessionSection from '../components/settings/SessionSection';
import BrowserSection from '../components/settings/BrowserSection';
import EditorSection from '../components/settings/EditorSection';
import AppsSection from '../components/settings/AppsSection';
import ClaudeProfilesSection from '../components/settings/ClaudeProfilesSection';
import SupervisorSection from '../components/settings/SupervisorSection';
import PluginsSection from '../components/settings/PluginsSection';

interface SettingsPaneProps {
  title: string;
}

// ── Section registry ─────────────────────────────────────────────────────────

interface SectionDef {
  key: string;
  label: string;
  group: string;
  keywords: string[];
}

// Groups determine the sidebar hierarchy. Sections within each group appear
// together under a group label.
const GROUPS = ['Agents & AI', 'Workspace', 'Tools', 'System'] as const;

const SECTIONS: SectionDef[] = [
  // Agents & AI
  {
    key: 'session',
    label: 'Session',
    group: 'Agents & AI',
    keywords: [
      'session', 'agent', 'provider', 'claude', 'codex', 'opencode', 'pi', 'binary',
      'path', 'tool', 'resume', 'restore', 'auto', 'composer', 'send', 'button', 'view',
      'gui', 'terminal', 'default', 'spawn', 'model', 'diff', 'font', 'scale',
    ],
  },
  {
    key: 'profiles',
    label: 'Claude Profiles',
    group: 'Agents & AI',
    keywords: [
      'profile', 'claude', 'model', 'api', 'permissions', 'mcp', 'config', 'key',
    ],
  },
  {
    key: 'supervisor',
    label: 'Supervisor',
    group: 'Agents & AI',
    keywords: [
      'supervisor', 'fleet', 'agent', 'summarize', 'digest', 'model', 'coordinator',
      'poll', 'notify', 'background',
    ],
  },
  // Workspace
  {
    key: 'appearance',
    label: 'Appearance',
    group: 'Workspace',
    keywords: [
      'appearance', 'theme', 'color', 'corner', 'border', 'font', 'dark', 'light',
      'radius', 'accent', 'style', 'palette',
    ],
  },
  {
    key: 'layout',
    label: 'Layout',
    group: 'Workspace',
    keywords: [
      'layout', 'pane', 'tab', 'split', 'gap', 'width', 'view', 'mode', 'position',
      'sidebar', 'peek', 'tabs', 'spatial', 'stacked',
    ],
  },
  {
    key: 'keybindings',
    label: 'Keybindings',
    group: 'Workspace',
    keywords: [
      'keybinding', 'keyboard', 'shortcut', 'hotkey', 'vim', 'leader', 'bind',
      'prefix', 'chord', 'ctrl', 'alt', 'key',
    ],
  },
  // Tools
  {
    key: 'terminal',
    label: 'Terminal',
    group: 'Tools',
    keywords: [
      'terminal', 'shell', 'bash', 'pwsh', 'powershell', 'zsh', 'fish', 'console',
      'font', 'scrollback', 'cursor', 'blink',
    ],
  },
  {
    key: 'editor',
    label: 'Editor',
    group: 'Tools',
    keywords: [
      'editor', 'file', 'open', 'codemirror', 'vim', 'nvim', 'code', 'command',
    ],
  },
  {
    key: 'browser',
    label: 'Browser',
    group: 'Tools',
    keywords: [
      'browser', 'homepage', 'bookmark', 'hibernate', 'web', 'url', 'tab',
    ],
  },
  {
    key: 'apps',
    label: 'Apps',
    group: 'Tools',
    keywords: [
      'app', 'url', 'launch', 'custom', 'icon', 'link', 'launcher',
    ],
  },
  // System
  {
    key: 'notifications',
    label: 'Notifications',
    group: 'System',
    keywords: [
      'notification', 'alert', 'sound', 'done', 'notify', 'badge', 'attention',
    ],
  },
  {
    key: 'plugins',
    label: 'Plugins',
    group: 'System',
    keywords: [
      'plugin', 'extension', 'vim', 'editor', 'language', 'addon', 'install',
    ],
  },
];

function matchesQuery(section: SectionDef, q: string): boolean {
  if (!q) return true;
  const lower = q.toLowerCase();
  return (
    section.label.toLowerCase().includes(lower) ||
    section.keywords.some((kw) => kw.includes(lower))
  );
}

// ── Sidebar nav ───────────────────────────────────────────────────────────────

interface NavProps {
  sections: SectionDef[];
  active: string;
  onSelect: (key: string) => void;
}

const NAV_GROUP_LABEL: React.CSSProperties = {
  fontSize: '0.62rem',
  fontWeight: 700,
  textTransform: 'uppercase',
  letterSpacing: '0.09em',
  color: 'var(--wks-text-disabled)',
  padding: '14px 12px 4px',
};

const NAV_ITEM_BASE: React.CSSProperties = {
  display: 'block',
  width: '100%',
  textAlign: 'left',
  padding: '6px 12px',
  fontSize: '0.82rem',
  fontFamily: 'inherit',
  fontWeight: 400,
  border: 'none',
  borderRadius: '6px',
  cursor: 'pointer',
  transition: 'background 0.1s, color 0.1s',
  background: 'transparent',
};

function Sidebar({ sections, active, onSelect }: NavProps) {
  const byGroup: Record<string, SectionDef[]> = {};
  for (const s of sections) {
    if (!byGroup[s.group]) byGroup[s.group] = [];
    byGroup[s.group].push(s);
  }

  return (
    <nav style={{ width: 172, flexShrink: 0, paddingTop: 8 }}>
      {GROUPS.map((group) => {
        const items = byGroup[group];
        if (!items?.length) return null;
        return (
          <div key={group}>
            <div style={NAV_GROUP_LABEL}>{group}</div>
            {items.map((s) => {
              const isActive = s.key === active;
              return (
                <button
                  key={s.key}
                  onClick={() => onSelect(s.key)}
                  style={{
                    ...NAV_ITEM_BASE,
                    color: isActive ? 'var(--wks-accent-text, var(--wks-accent))' : 'var(--wks-text-secondary)',
                    background: isActive ? 'var(--wks-accent-bg, rgba(99,102,241,0.12))' : 'transparent',
                    fontWeight: isActive ? 600 : 400,
                  }}
                >
                  {s.label}
                </button>
              );
            })}
          </div>
        );
      })}
    </nav>
  );
}

// ── Main pane ─────────────────────────────────────────────────────────────────

const SettingsPane: React.FC<SettingsPaneProps> = () => {
  const { config, save } = useConfig();
  const [search, setSearch] = useState('');
  const [activeKey, setActiveKey] = useState('session');
  const searchRef = useRef<HTMLInputElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  const q = search.trim();

  const visibleSections = useMemo(
    () => SECTIONS.filter((s) => matchesQuery(s, q)),
    [q],
  );

  // When search filters sections, keep activeKey valid.
  useEffect(() => {
    if (q && visibleSections.length > 0 && !visibleSections.find((s) => s.key === activeKey)) {
      setActiveKey(visibleSections[0].key);
    }
  }, [q, visibleSections, activeKey]);

  // Press / to focus the search box.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === '/' && document.activeElement?.tagName !== 'INPUT' && document.activeElement?.tagName !== 'TEXTAREA') {
        e.preventDefault();
        searchRef.current?.focus();
      }
      if (e.key === 'Escape' && document.activeElement === searchRef.current) {
        setSearch('');
        searchRef.current?.blur();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  const handleNavSelect = useCallback((key: string) => {
    setActiveKey(key);
    // Scroll the content area to the matching section anchor.
    const el = contentRef.current?.querySelector(`#settings-${key}`);
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }, []);

  // Which section to show (when not searching: show all; nav just scrolls).
  // When searching: show only matching sections.
  const showAll = !q;

  const renderSection = (key: string) => {
    switch (key) {
      case 'session': return <SessionSection config={config} save={save} />;
      case 'profiles': return <ClaudeProfilesSection />;
      case 'supervisor': return <SupervisorSection config={config} save={save} />;
      case 'appearance': return <AppearanceSection config={config} save={save} />;
      case 'layout': return <LayoutSection config={config} save={save} />;
      case 'keybindings': return <KeybindingsSection config={config} save={save} />;
      case 'terminal': return <TerminalSection config={config} save={save} />;
      case 'editor': return <EditorSection config={config} save={save} />;
      case 'browser': return <BrowserSection config={config} save={save} />;
      case 'apps': return <AppsSection config={config} save={save} />;
      case 'notifications': return <NotificationsSection config={config} save={save} />;
      case 'plugins': return <PluginsSection />;
      default: return null;
    }
  };

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: '"Hanken Grotesk", Inter, system-ui, sans-serif',
        fontSize: '14px',
      }}
    >
      {/* Top bar: title + search */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        padding: '14px 20px',
        borderBottom: '1px solid var(--wks-border-subtle)',
        flexShrink: 0,
      }}>
        <h2 style={{ fontSize: '1.05rem', fontWeight: 700, margin: 0, color: 'var(--wks-text-primary)', letterSpacing: '-0.01em', flexShrink: 0 }}>
          Settings
        </h2>

        {/* Search */}
        <div
          style={{
            display: 'flex', alignItems: 'center', gap: 8,
            flex: 1, maxWidth: 360,
            padding: '0 12px',
            background: 'var(--wks-bg-raised)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: '8px',
            transition: 'border-color 0.15s',
          }}
          onFocusCapture={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)'; }}
          onBlurCapture={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)'; }}
        >
          <span aria-hidden style={{ color: 'var(--wks-text-faint)', fontSize: '0.9rem', flexShrink: 0, lineHeight: 1 }}>⌕</span>
          <input
            ref={searchRef}
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search settings…"
            title="Press / to focus"
            spellCheck={false}
            style={{
              flex: 1,
              height: '36px',
              padding: 0,
              fontSize: '0.85rem',
              fontFamily: 'inherit',
              backgroundColor: 'transparent',
              color: 'var(--wks-text-primary)',
              border: 'none',
              outline: 'none',
            }}
          />
          {search && (
            <button
              onClick={() => { setSearch(''); searchRef.current?.focus(); }}
              title="Clear"
              style={{
                background: 'none', border: 'none', cursor: 'pointer', padding: '0 2px',
                color: 'var(--wks-text-faint)', fontSize: '1rem', lineHeight: 1, flexShrink: 0,
              }}
            >×</button>
          )}
          {!search && (
            <kbd style={{
              fontSize: '0.65rem', padding: '1px 5px',
              background: 'var(--wks-bg-elevated)', border: '1px solid var(--wks-border)',
              borderRadius: 3, color: 'var(--wks-text-faint)', flexShrink: 0,
            }}>/</kbd>
          )}
        </div>
      </div>

      {/* Body: sidebar + content */}
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>

        {/* Left sidebar */}
        <div style={{
          width: 192,
          flexShrink: 0,
          borderRight: '1px solid var(--wks-border-subtle)',
          overflowY: 'auto',
          padding: '4px 8px 24px',
        }}>
          <Sidebar
            sections={visibleSections}
            active={activeKey}
            onSelect={handleNavSelect}
          />
        </div>

        {/* Content area */}
        <div
          ref={contentRef}
          style={{
            flex: 1,
            overflowY: 'auto',
            padding: '24px 32px',
          }}
        >
          <div style={{ maxWidth: 680 }}>
            {visibleSections.length === 0 && q ? (
              <div style={{
                marginTop: 48, textAlign: 'center',
                fontSize: '0.85rem', color: 'var(--wks-text-faint)',
              }}>
                No settings match &ldquo;{q}&rdquo;
              </div>
            ) : showAll ? (
              // No search — show all in logical order, nav scrolls to anchors.
              SECTIONS.map((s) => (
                <div key={s.key} id={`settings-${s.key}`} style={{ scrollMarginTop: 8 }}>
                  {renderSection(s.key)}
                </div>
              ))
            ) : (
              // Search active — show only matching sections.
              visibleSections.map((s) => (
                <div key={s.key} id={`settings-${s.key}`} style={{ scrollMarginTop: 8 }}>
                  {renderSection(s.key)}
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default SettingsPane;
