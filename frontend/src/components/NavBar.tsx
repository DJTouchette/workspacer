import React, { useState, useRef, useEffect } from 'react';
import { PaneConfig, PaneType } from '../types/pane';
import { useConfig } from '../hooks/useConfig';

interface NavBarProps {
  panes: PaneConfig[];
  activePaneId: string;
  onPaneClick: (id: string) => void;
  onAddPane?: (shell?: string, label?: string) => void;
}

const typeLabels: Record<PaneType, string> = {
  terminal: '>_',
  browser: '\u{1F310}',
  notes: '\u{1F4DD}',
  agent: '\u{1F916}',
};

const NavBar: React.FC<NavBarProps> = ({ panes, activePaneId, onPaneClick, onAddPane }) => {
  const { config } = useConfig();
  const navHeight = config.ui.navBarHeight || 28;
  const shells = config.terminal.shells || [];
  const [showShellMenu, setShowShellMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close menu on click outside
  useEffect(() => {
    if (!showShellMenu) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowShellMenu(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [showShellMenu]);

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
        backgroundColor: 'rgb(20, 20, 24)',
        borderBottom: '1px solid rgb(45, 45, 50)',
        padding: '0 10px',
        zIndex: 100,
        userSelect: 'none',
      }}
    >
      {/* App title */}
      <div
        style={{
          fontWeight: 600,
          fontSize: '0.7rem',
          color: 'rgb(200, 200, 210)',
          marginRight: '16px',
        }}
      >
        Workspacer
      </div>

      {/* Pane tabs */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '2px',
          flex: 1,
          overflow: 'hidden',
        }}
      >
        {panes.map((pane, idx) => {
          const isActive = pane.id === activePaneId;
          return (
            <button
              key={pane.id}
              onClick={() => onPaneClick(pane.id)}
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
                  ? 'rgb(45, 48, 60)'
                  : 'transparent',
                color: isActive
                  ? 'rgb(220, 220, 235)'
                  : 'rgb(140, 140, 155)',
                transition: 'none',
              }}
              onMouseEnter={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'rgb(38, 38, 44)';
                  (e.currentTarget as HTMLElement).style.color = 'rgb(180, 180, 195)';
                }
              }}
              onMouseLeave={(e) => {
                if (!isActive) {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  (e.currentTarget as HTMLElement).style.color = 'rgb(140, 140, 155)';
                }
              }}
              title={`${pane.title} (Ctrl+${idx + 1})`}
            >
              <span style={{ fontSize: '0.65rem' }}>{typeLabels[pane.type]}</span>
              <span>{pane.title}</span>
            </button>
          );
        })}

        {/* Add pane button — click for default, right-click or long text for shell picker */}
        {onAddPane && (
          <div ref={menuRef} style={{ position: 'relative', display: 'inline-flex' }}>
            <button
              onClick={() => onAddPane()}
              onContextMenu={(e) => {
                e.preventDefault();
                setShowShellMenu((v) => !v);
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
                color: 'rgb(120, 120, 135)',
                transition: 'none',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'rgb(38, 38, 44)';
                (e.currentTarget as HTMLElement).style.color = 'rgb(200, 200, 210)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                (e.currentTarget as HTMLElement).style.color = 'rgb(120, 120, 135)';
              }}
              title="New terminal (Ctrl+T) | Right-click for shell picker"
            >
              +
            </button>

            {/* Shell picker dropdown */}
            {showShellMenu && shells.length > 0 && (
              <div
                style={{
                  position: 'absolute',
                  top: '22px',
                  left: 0,
                  backgroundColor: 'rgb(30, 30, 33)',
                  border: '1px solid rgb(55, 55, 60)',
                  borderRadius: '4px',
                  padding: '4px 0',
                  zIndex: 200,
                  minWidth: '140px',
                }}
              >
                {shells.map((shell) => (
                  <button
                    key={shell.name}
                    onClick={() => {
                      setShowShellMenu(false);
                      onAddPane(shell.path, shell.label);
                    }}
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
                      color: 'rgb(180, 180, 190)',
                      textAlign: 'left',
                      height: 'auto',
                      lineHeight: '1.4',
                      transition: 'none',
                    }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLElement).style.backgroundColor = 'rgb(45, 48, 60)';
                      (e.currentTarget as HTMLElement).style.color = 'rgb(220, 220, 235)';
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                      (e.currentTarget as HTMLElement).style.color = 'rgb(180, 180, 190)';
                    }}
                  >
                    {shell.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </nav>
  );
};

export default NavBar;
