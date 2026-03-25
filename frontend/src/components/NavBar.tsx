import React from 'react';
import { PaneConfig, PaneType } from '../types/pane';
import { useConfig } from '../hooks/useConfig';

interface NavBarProps {
  panes: PaneConfig[];
  activePaneId: string;
  onPaneClick: (id: string) => void;
  onAddPane?: () => void;
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

        {/* Add pane button */}
        {onAddPane && (
          <button
            onClick={onAddPane}
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
            title="New terminal (Ctrl+T)"
          >
            +
          </button>
        )}
      </div>
    </nav>
  );
};

export default NavBar;
