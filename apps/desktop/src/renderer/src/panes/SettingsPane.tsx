import React, { useState, useMemo, useRef, useCallback, useEffect } from 'react';
import { useConfig } from '../hooks/useConfig';
import { fuzzyScoreAny } from '../lib/fuzzy';
import { SETTINGS_SECTION_EVENT, consumePendingSettingsSection } from '../lib/settingsBus';
import { Settings as SettingsIcon } from '../components/icons';
import { useIsSmallScreen } from '../hooks/useMediaQuery';
import AppearanceSection from '../components/settings/AppearanceSection';
import LayoutSection from '../components/settings/LayoutSection';
import TerminalSection from '../components/settings/TerminalSection';
import KeybindingsSection from '../components/settings/KeybindingsSection';
import NotificationsSection from '../components/settings/NotificationsSection';
import UpdatesSection from '../components/settings/UpdatesSection';
import CliSection from '../components/settings/CliSection';
import SessionSection from '../components/settings/SessionSection';
import BrowserSection from '../components/settings/BrowserSection';
import EditorSection from '../components/settings/EditorSection';
import AppsSection from '../components/settings/AppsSection';
import ClaudeProfilesSection from '../components/settings/ClaudeProfilesSection';
import SupervisorSection from '../components/settings/SupervisorSection';
import ModelPricingSection from '../components/settings/ModelPricingSection';
import PluginsSection from '../components/settings/PluginsSection';
import ToolsSection from '../components/settings/ToolsSection';

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
      'session',
      'agent',
      'provider',
      'claude',
      'codex',
      'opencode',
      'pi',
      'binary',
      'path',
      'tool',
      'resume',
      'restore',
      'auto',
      'composer',
      'send',
      'button',
      'view',
      'gui',
      'terminal',
      'default',
      'spawn',
      'model',
      'diff',
      'font',
      'scale',
    ],
  },
  {
    key: 'profiles',
    label: 'Claude Profiles',
    group: 'Agents & AI',
    keywords: ['profile', 'claude', 'model', 'api', 'permissions', 'mcp', 'config', 'key'],
  },
  {
    key: 'supervisor',
    label: 'Supervisor',
    group: 'Agents & AI',
    keywords: [
      'supervisor',
      'fleet',
      'agent',
      'summarize',
      'digest',
      'model',
      'coordinator',
      'poll',
      'notify',
      'background',
    ],
  },
  {
    key: 'pricing',
    label: 'Model pricing',
    group: 'Agents & AI',
    keywords: [
      'pricing',
      'price',
      'rate',
      'cost',
      'token',
      'model',
      'context',
      'window',
      'override',
      'usage',
      'claude',
      'codex',
      'fable',
      'opus',
      'sonnet',
    ],
  },
  // Workspace
  {
    key: 'appearance',
    label: 'Appearance',
    group: 'Workspace',
    keywords: [
      'appearance',
      'theme',
      'color',
      'corner',
      'border',
      'font',
      'dark',
      'light',
      'radius',
      'accent',
      'style',
      'palette',
    ],
  },
  {
    key: 'layout',
    label: 'Layout',
    group: 'Workspace',
    keywords: [
      'layout',
      'pane',
      'tab',
      'split',
      'gap',
      'width',
      'view',
      'mode',
      'position',
      'sidebar',
      'peek',
      'tabs',
      'focus',
      'fleet',
    ],
  },
  {
    key: 'keybindings',
    label: 'Keybindings',
    group: 'Workspace',
    keywords: [
      'keybinding',
      'keyboard',
      'shortcut',
      'hotkey',
      'vim',
      'leader',
      'bind',
      'prefix',
      'chord',
      'ctrl',
      'alt',
      'key',
    ],
  },
  // Tools
  {
    key: 'terminal',
    label: 'Terminal',
    group: 'Tools',
    keywords: [
      'terminal',
      'shell',
      'bash',
      'pwsh',
      'powershell',
      'zsh',
      'fish',
      'console',
      'font',
      'scrollback',
      'cursor',
      'blink',
    ],
  },
  {
    key: 'editor',
    label: 'Editor',
    group: 'Tools',
    keywords: ['editor', 'file', 'open', 'codemirror', 'vim', 'nvim', 'code', 'command'],
  },
  {
    key: 'browser',
    label: 'Browser',
    group: 'Tools',
    keywords: ['browser', 'homepage', 'bookmark', 'hibernate', 'web', 'url', 'tab'],
  },
  {
    key: 'apps',
    label: 'Apps',
    group: 'Tools',
    keywords: ['app', 'url', 'launch', 'custom', 'icon', 'link', 'launcher'],
  },
  // System
  {
    key: 'notifications',
    label: 'Notifications',
    group: 'System',
    keywords: ['notification', 'alert', 'sound', 'done', 'notify', 'badge', 'attention'],
  },
  {
    key: 'updates',
    label: 'Updates',
    group: 'System',
    keywords: ['update', 'auto', 'upgrade', 'version', 'release', 'download', 'install', 'channel'],
  },
  {
    key: 'tools',
    label: 'System Tools',
    group: 'System',
    keywords: [
      'tools',
      'dependencies',
      'git',
      'tailscale',
      'binary',
      'path',
      'missing',
      'install',
      'external',
      'requirements',
    ],
  },
  {
    key: 'cli',
    label: 'Command Line',
    group: 'System',
    keywords: ['cli', 'command', 'terminal', 'path', 'install', 'serve', 'headless', 'server'],
  },
  {
    key: 'plugins',
    label: 'Plugins',
    group: 'System',
    keywords: ['plugin', 'extension', 'vim', 'editor', 'language', 'addon', 'install'],
  },
];

/** Fuzzy score of a query against a section's label + keywords; -Infinity = no
 *  match. Fuzzy means "kbd" finds Keybindings and "permode" finds permission
 *  keywords — characters must appear in order but not adjacently. */
function sectionScore(section: SectionDef, q: string): number {
  return fuzzyScoreAny(q, [section.label, ...section.keywords]);
}

// ── Sidebar nav ───────────────────────────────────────────────────────────────

interface NavProps {
  sections: SectionDef[];
  active: string;
  onSelect: (key: string) => void;
}

const NAV_GROUP_LABEL: React.CSSProperties = {
  fontSize: '0.58rem',
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: '0.08em',
  color: 'var(--wks-text-faint)',
  padding: '14px 12px 5px',
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
                    color: isActive
                      ? 'var(--wks-accent-text, var(--wks-accent))'
                      : 'var(--wks-text-secondary)',
                    background: isActive
                      ? 'var(--wks-accent-bg, rgba(99,102,241,0.12))'
                      : 'transparent',
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
  const isSmallScreen = useIsSmallScreen();
  const [search, setSearch] = useState('');
  const [activeKey, setActiveKey] = useState('session');
  const searchRef = useRef<HTMLInputElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  const q = search.trim();

  // Fuzzy-filter and rank: best match first while searching; registry order
  // otherwise (stable sort keeps ties in registry order).
  const visibleSections = useMemo(() => {
    if (!q) return SECTIONS;
    return SECTIONS.map((s) => ({ s, score: sectionScore(s, q) }))
      .filter((x) => x.score > -Infinity)
      .sort((a, b) => b.score - a.score)
      .map((x) => x.s);
  }, [q]);

  // When search filters sections, keep activeKey valid.
  useEffect(() => {
    if (q && visibleSections.length > 0 && !visibleSections.find((s) => s.key === activeKey)) {
      setActiveKey(visibleSections[0].key);
    }
  }, [q, visibleSections, activeKey]);

  // Deep-link into a section (settingsBus): consume a request that fired while
  // this pane was mounting, then follow live requests while it's open. Defer
  // the scroll a frame so the section anchors exist on the mount path.
  useEffect(() => {
    const jump = (key: string | null) => {
      if (!key || !SECTIONS.some((s) => s.key === key)) return;
      setSearch('');
      setActiveKey(key);
      requestAnimationFrame(() => {
        const el = contentRef.current?.querySelector(`#settings-${key}`);
        if (el) el.scrollIntoView({ block: 'start' });
      });
    };
    jump(consumePendingSettingsSection());
    const onSection = (e: Event) => jump((e as CustomEvent<{ key?: string }>).detail?.key ?? null);
    window.addEventListener(SETTINGS_SECTION_EVENT, onSection);
    return () => window.removeEventListener(SETTINGS_SECTION_EVENT, onSection);
  }, []);

  // Press / to focus the search box.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (
        e.key === '/' &&
        document.activeElement?.tagName !== 'INPUT' &&
        document.activeElement?.tagName !== 'TEXTAREA'
      ) {
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
      case 'session':
        return <SessionSection config={config} save={save} />;
      case 'profiles':
        return <ClaudeProfilesSection />;
      case 'supervisor':
        return <SupervisorSection config={config} save={save} />;
      case 'pricing':
        return <ModelPricingSection />;
      case 'appearance':
        return <AppearanceSection config={config} save={save} />;
      case 'layout':
        return <LayoutSection config={config} save={save} />;
      case 'keybindings':
        return <KeybindingsSection config={config} save={save} />;
      case 'terminal':
        return <TerminalSection config={config} save={save} />;
      case 'editor':
        return <EditorSection config={config} save={save} />;
      case 'browser':
        return <BrowserSection config={config} save={save} />;
      case 'apps':
        return <AppsSection config={config} save={save} />;
      case 'notifications':
        return <NotificationsSection config={config} save={save} />;
      case 'updates':
        return <UpdatesSection config={config} save={save} />;
      case 'tools':
        return <ToolsSection />;
      case 'cli':
        return <CliSection />;
      case 'plugins':
        return <PluginsSection />;
      default:
        return null;
    }
  };

  return (
    <div
      style={{
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        overflow: 'hidden',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: '"Hanken Grotesk", Inter, system-ui, sans-serif',
        fontSize: '0.85rem',
      }}
    >
      {/* Soft accent glow behind the hero — same decoration as the spawn
          dialog and the Overview/Usage panes. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: -420,
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

      {/* ── Hero: badge + title + search ─────────────────────────────────── */}
      <div
        style={{
          position: 'relative',
          textAlign: 'center',
          padding: '28px 20px 18px',
          flexShrink: 0,
          animation: 'wks-fade-in 0.25s ease-out',
        }}
      >
        <div
          style={{
            width: 52,
            height: 52,
            margin: '0 auto',
            borderRadius: '50%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            border: '1px solid var(--wks-border-input)',
            background: 'color-mix(in srgb, var(--wks-accent) 5%, transparent)',
            color: 'var(--wks-accent-text, var(--wks-text-primary))',
          }}
        >
          <SettingsIcon size={22} strokeWidth={1.7} />
        </div>
        <h2
          style={{
            fontSize: '1.05rem',
            fontWeight: 650,
            margin: '12px 0 0',
            color: 'var(--wks-text-primary)',
            letterSpacing: '-0.01em',
          }}
        >
          Settings
        </h2>

        {/* Search */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            maxWidth: 420,
            margin: '14px auto 0',
            padding: '0 14px',
            background: 'var(--wks-bg-raised)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 'var(--wks-radius-pill, 999px)',
            transition: 'border-color 0.15s',
          }}
          onFocusCapture={(e) => {
            (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
          }}
          onBlurCapture={(e) => {
            (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
          }}
        >
          <span
            aria-hidden
            style={{
              color: 'var(--wks-text-faint)',
              fontSize: '0.9rem',
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ⌕
          </span>
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
              onClick={() => {
                setSearch('');
                searchRef.current?.focus();
              }}
              title="Clear"
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                padding: '0 2px',
                color: 'var(--wks-text-faint)',
                fontSize: '1rem',
                lineHeight: 1,
                flexShrink: 0,
              }}
            >
              ×
            </button>
          )}
          {!search && (
            <kbd
              style={{
                fontSize: '0.65rem',
                padding: '1px 5px',
                background: 'var(--wks-bg-elevated)',
                border: '1px solid var(--wks-border)',
                borderRadius: 3,
                color: 'var(--wks-text-faint)',
                flexShrink: 0,
              }}
            >
              /
            </kbd>
          )}
        </div>
      </div>

      {/* Phones: the 192px sidebar would eat half the viewport — swap it for
          a horizontally scrollable chip row above the content. */}
      {isSmallScreen && (
        <div
          style={{
            display: 'flex',
            gap: 6,
            overflowX: 'auto',
            flexShrink: 0,
            padding: '2px 14px 10px',
            scrollbarWidth: 'none',
          }}
        >
          {visibleSections.map((s) => {
            const active = s.key === activeKey;
            return (
              <button
                key={s.key}
                onClick={() => handleNavSelect(s.key)}
                style={{
                  flexShrink: 0,
                  padding: '5px 12px',
                  fontSize: '0.72rem',
                  fontFamily: 'inherit',
                  fontWeight: 600,
                  cursor: 'pointer',
                  borderRadius: 'var(--wks-radius-pill, 999px)',
                  border: '1px solid',
                  borderColor: active
                    ? 'color-mix(in srgb, var(--wks-accent) 45%, transparent)'
                    : 'var(--wks-border-subtle)',
                  background: active
                    ? 'var(--wks-accent-bg, rgba(99,102,241,0.14))'
                    : 'transparent',
                  color: active
                    ? 'var(--wks-accent-text, var(--wks-accent))'
                    : 'var(--wks-text-secondary)',
                }}
              >
                {s.label}
              </button>
            );
          })}
        </div>
      )}

      {/* Body: sidebar + content */}
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        {/* Left sidebar (desktop only — phones use the chip row above) */}
        {!isSmallScreen && (
          <div
            style={{
              width: 192,
              flexShrink: 0,
              borderRight: '1px solid var(--wks-border-subtle)',
              overflowY: 'auto',
              padding: '4px 8px 24px',
            }}
          >
            <Sidebar sections={visibleSections} active={activeKey} onSelect={handleNavSelect} />
          </div>
        )}

        {/* Content area */}
        <div
          ref={contentRef}
          style={{
            flex: 1,
            overflowY: 'auto',
            padding: isSmallScreen ? '14px 16px' : '24px 32px',
          }}
        >
          <div style={{ maxWidth: 680 }}>
            {visibleSections.length === 0 && q ? (
              <div
                style={{
                  marginTop: 48,
                  textAlign: 'center',
                  fontSize: '0.85rem',
                  color: 'var(--wks-text-faint)',
                }}
              >
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
