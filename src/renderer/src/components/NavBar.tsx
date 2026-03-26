import React, { useState, useRef, useEffect } from 'react';
import { PaneType, TabConfig } from '../types/pane';
import { useConfig } from '../hooks/useConfig';

interface NavBarProps {
  tabs: TabConfig[];
  activeTabId: string;
  onTabClick: (id: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string) => void;
}

const typeLabels: Record<PaneType, string> = {
  terminal: '>_',
  browser: '\u{1F310}',
  notes: '\u{1F4DD}',
  agent: '\u{1F916}',
  claude: '\u2666',
  settings: '\u2699',
};

const NavBar: React.FC<NavBarProps> = ({ tabs, activeTabId, onTabClick, onAddTab }) => {
  const { config } = useConfig();
  const navHeight = config.ui.navBarHeight || 28;
  const shells = config.terminal.shells || [];
  const [showMenu, setShowMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showMenu) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showMenu]);

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
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                padding: '2px 8px',
                margin: 0,
                width: 'auto',
                height: '20px',
                lineHeight: '1',
                border: 'none',
                borderRadius: '4px',
                cursor: 'pointer',
                fontSize: '0.65rem',
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
              <span style={{ fontSize: '0.65rem' }}>
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
                width: '20px',
                height: '20px',
                lineHeight: '1',
                border: 'none',
                borderRadius: '4px',
                cursor: 'pointer',
                fontSize: '0.75rem',
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
                  top: `${config.ui.navBarHeight || 28}px`,
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
                  if (folder) onAddTab('claude', undefined, undefined, folder);
                }} />
                <MenuButton label="Browser" onClick={() => { setShowMenu(false); onAddTab('browser'); }} />
                <MenuButton label="Notes" onClick={() => { setShowMenu(false); onAddTab('notes'); }} />

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
    </nav>
  );
};

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
