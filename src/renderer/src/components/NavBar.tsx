import React, { useState, useRef, useEffect } from 'react';
import { PaneType, TabConfig } from '../types/pane';
import { useConfig } from '../hooks/useConfig';

interface NavBarProps {
  tabs: TabConfig[];
  activeTabId: string;
  onTabClick: (id: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
  onCloseTab?: (tabId: string) => void;
  onRenameTab?: (tabId: string) => void;
  onSplitTab?: (tabId: string, type: PaneType) => void;
  onMoveTab?: (tabId: string, toIndex: number) => void;
}

const typeLabels: Record<PaneType, string> = {
  terminal: '>_',
  browser: '\u{1F310}',
  notes: '\u{1F4DD}',
  agent: '\u{1F916}',
  claude: '\u2666',
  settings: '\u2699',
  dashboard: '\u{1F4CA}',
  tracker: '\u{1F4CB}',
  devops: '\u{1F527}',
  'agent-manager': '\u{1F916}',
  devdaemon: '\u26A1',
  inbox: '\u{1F4E5}',
};

interface SessionPickerState {
  folder: string;
  profileId?: string;
  sessions: { sessionId: string; timestamp: string; summary: string }[];
}

const NavBar: React.FC<NavBarProps> = ({ tabs, activeTabId, onTabClick, onAddTab, onCloseTab, onRenameTab, onSplitTab, onMoveTab }) => {
  const { config } = useConfig();
  const navHeight = Math.max(config.ui.navBarHeight || 34, 32);
  const shells = config.terminal.shells || [];
  const [showMenu, setShowMenu] = useState(false);
  const [tabContextMenu, setTabContextMenu] = useState<{ tabId: string; tabIdx: number; x: number; y: number } | null>(null);
  const [profilePickerState, setProfilePickerState] = useState<{ folder: string; profiles: any[] } | null>(null);
  const [sessionPickerState, setSessionPickerState] = useState<SessionPickerState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const tabMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showMenu && !tabContextMenu) return;
    const handler = (e: MouseEvent) => {
      if (showMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
      if (tabContextMenu && tabMenuRef.current && !tabMenuRef.current.contains(e.target as Node)) {
        setTabContextMenu(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showMenu, tabContextMenu]);

  return (
    <nav
      className="navbar"
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        height: `${navHeight}px`,
        display: 'flex',
        alignItems: 'center',
        backgroundColor: 'var(--wks-bg-input)',
        borderBottom: '1px solid var(--wks-border-subtle)',
        padding: '0 10px',
        zIndex: 100,
        userSelect: 'none',
        // @ts-ignore
        WebkitAppRegion: 'drag',
        appRegion: 'drag',
      }}
    >
      <div
        style={{
          fontWeight: 600,
          fontSize: '0.7rem',
          color: 'var(--wks-text-secondary)',
          marginRight: '16px',
        }}
      >
        Workspacer
      </div>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '2px',
          flex: 1,
          // @ts-ignore
          WebkitAppRegion: 'no-drag',
          appRegion: 'no-drag',
          overflow: 'hidden',
        }}
      >
        {tabs.map((tab, idx) => {
          const isActive = tab.id === activeTabId;
          const singlePane = tab.panes.length === 1;
          const firstPaneType = tab.panes[0]?.type ?? 'terminal';
          const hasHibernated = tab.panes.some((p) => p.hibernated);

          return (
            <button
              key={tab.id}
              onClick={() => onTabClick(tab.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setTabContextMenu({ tabId: tab.id, tabIdx: idx, x: e.clientX, y: e.clientY });
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                padding: '2px 10px',
                margin: 0,
                width: 'auto',
                height: '26px',
                lineHeight: '1',
                border: 'none',
                borderRadius: '4px',
                cursor: 'pointer',
                fontSize: '0.75rem',
                fontFamily: 'inherit',
                fontWeight: isActive ? 600 : 400,
                backgroundColor: isActive
                  ? 'var(--wks-bg-selected)'
                  : 'transparent',
                color: isActive
                  ? 'var(--wks-text-primary)'
                  : 'var(--wks-text-muted)',
                opacity: hasHibernated && !isActive ? 0.4 : 1,
                transition: 'none',
              }}
              onMouseEnter={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-tertiary)';
                }
              }}
              onMouseLeave={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
                }
              }}
              title={`${tab.title} (Ctrl+${idx + 1})`}
            >
              <span style={{ fontSize: '0.75rem' }}>
                {singlePane ? typeLabels[firstPaneType] : '\u25A3'}
              </span>
              <span>{tab.title}</span>
              {!singlePane && (
                <span style={{ fontSize: '0.5rem', opacity: 0.6 }}>
                  {tab.panes.length}
                </span>
              )}
            </button>
          );
        })}

        {onAddTab && (
          <div ref={menuRef} style={{ position: 'relative', display: 'inline-flex' }}>
            <button
              onClick={() => onAddTab('terminal')}
              onContextMenu={(e) => {
                e.preventDefault();
                setShowMenu((v) => !v);
              }}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                padding: 0,
                margin: '0 0 0 2px',
                width: '26px',
                height: '26px',
                lineHeight: '1',
                border: 'none',
                borderRadius: '4px',
                cursor: 'pointer',
                fontSize: '0.9rem',
                fontFamily: 'inherit',
                fontWeight: 400,
                backgroundColor: 'transparent',
                color: 'var(--wks-text-faint)',
                transition: 'none',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
              }}
              title="New terminal (Ctrl+T) | Right-click for more"
            >
              +
            </button>

            {showMenu && (
              <div
                style={{
                  position: 'fixed',
                  top: `${navHeight}px`,
                  backgroundColor: 'var(--wks-bg-surface)',
                  border: '1px solid var(--wks-border-input)',
                  borderRadius: '4px',
                  padding: '4px 0',
                  zIndex: 10000,
                  minWidth: '160px',
                  boxShadow: '0 4px 12px var(--wks-shadow)',
                }}
              >
                <div style={{ padding: '2px 8px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                  New Tab
                </div>
                <MenuButton label="Claude" onClick={async () => {
                  setShowMenu(false);
                  const folder = await window.electronAPI.pickFolder();
                  if (!folder) return;
                  // Check for profiles — show picker if more than 1
                  let profileId: string | undefined;
                  try {
                    const profiles = await window.electronAPI.claudeProfilesList();
                    if (profiles.length > 1) {
                      setProfilePickerState({ folder, profiles });
                      return;
                    }
                  } catch {}
                  // Check for existing sessions
                  try {
                    const sessions = await window.electronAPI.claudeListSessionsForDir(folder);
                    if (sessions.length > 0) {
                      setSessionPickerState({ folder, profileId, sessions });
                      return;
                    }
                  } catch {}
                  onAddTab('claude', undefined, undefined, folder);
                }} />
                <MenuButton label="Dashboard" onClick={() => { setShowMenu(false); onAddTab('dashboard'); }} />
                <MenuButton label="Tracker" onClick={() => { setShowMenu(false); onAddTab('tracker'); }} />
                <MenuButton label="Git & Pipelines" onClick={() => { setShowMenu(false); onAddTab('devops'); }} />
                <MenuButton label="Browser" onClick={() => { setShowMenu(false); onAddTab('browser'); }} />
                <MenuButton label="Notes" onClick={() => { setShowMenu(false); onAddTab('notes'); }} />
                <MenuButton label="Agent Manager" onClick={() => { setShowMenu(false); onAddTab('agent-manager'); }} />
                <MenuButton label="Daemon" onClick={() => { setShowMenu(false); onAddTab('devdaemon'); }} />

                <div style={{ height: '1px', backgroundColor: 'var(--wks-border)', margin: '4px 0' }} />

                <div style={{ padding: '2px 8px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                  Terminal
                </div>
                {shells.map((shell) => (
                  <MenuButton
                    key={shell.name}
                    label={shell.label}
                    onClick={() => {
                      setShowMenu(false);
                      onAddTab('terminal', shell.path, shell.label);
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}
      </div>
      {/* Profile Picker Modal */}
      {profilePickerState && (
        <div
          onClick={() => setProfilePickerState(null)}
          style={{
            position: 'fixed', inset: 0, zIndex: 3000,
            display: 'flex', justifyContent: 'center', paddingTop: '20vh',
            backgroundColor: 'rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-input)',
              borderRadius: 8, width: 320, padding: '12px 0', maxHeight: 300,
              boxShadow: '0 8px 32px var(--wks-shadow)',
            }}
          >
            <div style={{ padding: '0 16px 8px', fontSize: '0.75rem', fontWeight: 600, color: 'var(--wks-text-secondary)', borderBottom: '1px solid var(--wks-border)' }}>
              Select Claude Profile
            </div>
            {profilePickerState.profiles.map((p: any) => (
              <div
                key={p.id}
                onClick={async () => {
                  const { folder } = profilePickerState;
                  setProfilePickerState(null);
                  // Check for existing sessions before launching
                  try {
                    const sessions = await window.electronAPI.claudeListSessionsForDir(folder);
                    if (sessions.length > 0) {
                      setSessionPickerState({ folder, profileId: p.id, sessions });
                      return;
                    }
                  } catch {}
                  onAddTab?.('claude', undefined, undefined, folder, p.id);
                }}
                style={{
                  padding: '8px 16px', cursor: 'pointer', fontSize: '0.72rem',
                  display: 'flex', alignItems: 'center', gap: 8,
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
              >
                <span style={{ color: p.isDefault ? 'var(--wks-accent)' : 'var(--wks-text-disabled)', fontSize: '0.7rem' }}>
                  {p.isDefault ? '\u2666' : '\u25CB'}
                </span>
                <div style={{ flex: 1 }}>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 500 }}>{p.name}</div>
                  {p.extraArgs.length > 0 && (
                    <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)', fontFamily: 'monospace' }}>{p.extraArgs.join(' ')}</div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      {/* Session Picker Modal */}
      {sessionPickerState && (
        <div
          onClick={() => setSessionPickerState(null)}
          style={{
            position: 'fixed', inset: 0, zIndex: 3000,
            display: 'flex', justifyContent: 'center', paddingTop: '15vh',
            backgroundColor: 'rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              backgroundColor: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-input)',
              borderRadius: 8, width: 400, padding: '12px 0', maxHeight: 420,
              boxShadow: '0 8px 32px var(--wks-shadow)',
              display: 'flex', flexDirection: 'column',
            }}
          >
            <div style={{ padding: '0 16px 8px', fontSize: '0.75rem', fontWeight: 600, color: 'var(--wks-text-secondary)', borderBottom: '1px solid var(--wks-border)' }}>
              Claude Session
            </div>
            <div style={{ overflowY: 'auto', flex: 1 }}>
              {/* New session option */}
              <div
                onClick={() => {
                  const { folder, profileId } = sessionPickerState;
                  setSessionPickerState(null);
                  onAddTab?.('claude', undefined, undefined, folder, profileId);
                }}
                style={{
                  padding: '10px 16px', cursor: 'pointer', fontSize: '0.72rem',
                  display: 'flex', alignItems: 'center', gap: 8,
                  borderBottom: '1px solid var(--wks-border)',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
              >
                <span style={{ color: 'var(--wks-accent)', fontSize: '0.8rem' }}>+</span>
                <div>
                  <div style={{ color: 'var(--wks-text-primary)', fontWeight: 600 }}>New Session</div>
                  <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)' }}>Start a fresh conversation</div>
                </div>
              </div>

              {/* Existing sessions */}
              <div style={{ padding: '6px 16px 2px', fontSize: '0.55rem', color: 'var(--wks-text-disabled)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                Resume Session
              </div>
              {sessionPickerState.sessions.map((s) => (
                <div
                  key={s.sessionId}
                  onClick={() => {
                    const { folder, profileId } = sessionPickerState;
                    setSessionPickerState(null);
                    onAddTab?.('claude', undefined, undefined, folder, profileId, s.sessionId);
                  }}
                  style={{
                    padding: '8px 16px', cursor: 'pointer', fontSize: '0.72rem',
                    display: 'flex', alignItems: 'center', gap: 8,
                  }}
                  onMouseEnter={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
                  onMouseLeave={e => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
                >
                  <span style={{ color: 'var(--wks-text-disabled)', fontSize: '0.7rem' }}>{'\u25B6'}</span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ color: 'var(--wks-text-primary)', fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {s.summary}
                    </div>
                    <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-faint)', fontFamily: 'monospace' }}>
                      {formatSessionDate(s.timestamp)}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
      {/* Tab context menu */}
      {tabContextMenu && (
        <div
          ref={tabMenuRef}
          style={{
            position: 'fixed',
            top: tabContextMenu.y,
            left: tabContextMenu.x,
            backgroundColor: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: '4px',
            padding: '4px 0',
            zIndex: 10000,
            minWidth: '140px',
            boxShadow: '0 4px 12px var(--wks-shadow)',
          }}
        >
          <MenuButton label="Rename" onClick={() => { setTabContextMenu(null); onRenameTab?.(tabContextMenu.tabId); }} />
          <MenuButton label="Split — Terminal" onClick={() => { setTabContextMenu(null); onSplitTab?.(tabContextMenu.tabId, 'terminal'); }} />
          <MenuButton label="Split — Claude" onClick={() => { setTabContextMenu(null); onSplitTab?.(tabContextMenu.tabId, 'claude'); }} />
          <MenuButton label="Split — Browser" onClick={() => { setTabContextMenu(null); onSplitTab?.(tabContextMenu.tabId, 'browser'); }} />
          <div style={{ height: '1px', backgroundColor: 'var(--wks-border)', margin: '4px 0' }} />
          {tabContextMenu.tabIdx > 0 && (
            <MenuButton label="Move left" onClick={() => { setTabContextMenu(null); onMoveTab?.(tabContextMenu.tabId, tabContextMenu.tabIdx - 1); }} />
          )}
          {tabContextMenu.tabIdx < tabs.length - 1 && (
            <MenuButton label="Move right" onClick={() => { setTabContextMenu(null); onMoveTab?.(tabContextMenu.tabId, tabContextMenu.tabIdx + 1); }} />
          )}
          <div style={{ height: '1px', backgroundColor: 'var(--wks-border)', margin: '4px 0' }} />
          <MenuButton label="Close" onClick={() => { setTabContextMenu(null); onCloseTab?.(tabContextMenu.tabId); }} />
        </div>
      )}
    </nav>
  );
};

function formatSessionDate(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return 'just now';
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString();
  } catch {
    return iso;
  }
}

function MenuButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'block',
        width: '100%',
        padding: '4px 12px',
        margin: 0,
        border: 'none',
        borderRadius: 0,
        cursor: 'pointer',
        fontSize: '0.65rem',
        fontFamily: 'inherit',
        fontWeight: 400,
        backgroundColor: 'transparent',
        color: 'var(--wks-text-tertiary)',
        textAlign: 'left',
        height: 'auto',
        lineHeight: '1.4',
        transition: 'none',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)';
        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-tertiary)';
      }}
    >
      {label}
    </button>
  );
}

export default NavBar;
