import React, { useState, useMemo } from 'react';
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

const SECTION_KEYWORDS: Record<string, string[]> = {
  appearance: ['appearance', 'theme', 'color', 'corner', 'border', 'font', 'dark', 'light'],
  layout: ['layout', 'pane', 'tab', 'split', 'gap', 'width', 'view'],
  terminal: ['terminal', 'shell', 'bash', 'pwsh', 'powershell', 'zsh', 'fish', 'console'],
  keybindings: ['keybinding', 'keyboard', 'shortcut', 'hotkey', 'vim', 'leader', 'bind'],
  notifications: ['notification', 'alert', 'sound', 'done', 'notify'],
  session: ['session', 'resume', 'restore', 'auto', 'composer', 'send', 'button', 'claude', 'view'],
  browser: ['browser', 'homepage', 'bookmark', 'hibernate', 'web'],
  editor: ['editor', 'file', 'open'],
  apps: ['app', 'url', 'launch', 'custom'],
  profiles: ['profile', 'claude', 'model', 'api'],
  supervisor: ['supervisor', 'fleet', 'agent', 'summarize', 'digest', 'model', 'notify'],
  plugins: ['plugin', 'extension', 'vim', 'editor', 'language', 'addon'],
};

function sectionVisible(key: string, q: string): boolean {
  if (!q) return true;
  const lower = q.toLowerCase();
  return SECTION_KEYWORDS[key]?.some(kw => kw.includes(lower)) ?? true;
}

const SettingsPane: React.FC<SettingsPaneProps> = ({ title }) => {
  const { config, save } = useConfig();
  const [search, setSearch] = useState('');

  const q = search.trim();
  const show = useMemo(() => ({
    appearance: sectionVisible('appearance', q),
    layout: sectionVisible('layout', q),
    terminal: sectionVisible('terminal', q),
    keybindings: sectionVisible('keybindings', q),
    notifications: sectionVisible('notifications', q),
    session: sectionVisible('session', q),
    browser: sectionVisible('browser', q),
    editor: sectionVisible('editor', q),
    apps: sectionVisible('apps', q),
    profiles: sectionVisible('profiles', q),
    supervisor: sectionVisible('supervisor', q),
    plugins: sectionVisible('plugins', q),
  }), [q]);

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: '"Hanken Grotesk", Inter, system-ui, sans-serif',
        fontSize: '12px',
        overflow: 'auto',
        padding: '22px 26px',
      }}
    >
      <div style={{ width: '100%', maxWidth: 760, margin: '0 auto' }}>
      <h2 style={{ fontSize: '1.05rem', fontWeight: 700, margin: '0 0 14px 0', color: 'var(--wks-text-primary)', letterSpacing: '-0.01em' }}>
        Settings
      </h2>

      {/* Section filter — rounded pill with a search glyph (mockup). */}
      <div
        style={{
          display: 'flex', alignItems: 'center', gap: '9px',
          padding: '0 13px', marginBottom: '24px',
          background: 'var(--wks-bg-raised)',
          border: '1px solid var(--wks-border-subtle)',
          borderRadius: '11px',
          transition: 'border-color 0.15s',
        }}
        onFocus={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent-glow)'; }}
        onBlur={(e) => { (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)'; }}
      >
        <span aria-hidden style={{ color: 'var(--wks-text-faint)', fontSize: '0.85rem', flexShrink: 0 }}>⌕</span>
        <input
          type="search"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Filter settings…"
          spellCheck={false}
          style={{
            flex: 1,
            height: '38px',
            padding: 0,
            fontSize: '0.8rem',
            fontFamily: 'inherit',
            backgroundColor: 'transparent',
            color: 'var(--wks-text-primary)',
            border: 'none',
            outline: 'none',
            boxSizing: 'border-box',
          }}
        />
      </div>

      {/* Appearance section */}
      {show.appearance && <AppearanceSection config={config} save={save} />}

      {/* Layout section */}
      {show.layout && <LayoutSection config={config} save={save} />}

      {/* Terminal section */}
      {show.terminal && <TerminalSection config={config} save={save} />}

      {/* Keybindings section */}
      {show.keybindings && <KeybindingsSection config={config} save={save} />}

      {/* Notifications section */}
      {show.notifications && <NotificationsSection config={config} save={save} />}

      {/* Session section */}
      {show.session && <SessionSection config={config} save={save} />}

      {/* Browser section */}
      {show.browser && <BrowserSection config={config} save={save} />}

      {/* Editor section */}
      {show.editor && <EditorSection config={config} save={save} />}

      {/* Apps section */}
      {show.apps && <AppsSection config={config} save={save} />}

      {/* Claude Profiles section */}
      {show.profiles && <ClaudeProfilesSection />}

      {/* Supervisor section */}
      {show.supervisor && <SupervisorSection config={config} save={save} />}

      {/* Plugins section (settings contributed by installed plugins) */}
      {show.plugins && <PluginsSection />}

      {/* No-match state */}
      {q && !Object.values(show).some(Boolean) && (
        <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', textAlign: 'center', marginTop: '24px' }}>
          No sections match &ldquo;{q}&rdquo;
        </div>
      )}
      </div>
    </div>
  );
};

export default SettingsPane;
