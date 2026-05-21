import React, { useState, useRef, useEffect } from 'react';
import { PaneType, TabConfig } from '../types/pane';

interface SideBarProps {
  tabs: TabConfig[];
  activeTabId: string;
  onTabClick: (id: string) => void;
  onAddTab?: (type: PaneType) => void;
  onCloseTab?: (tabId: string) => void;
}

const typeIcons: Record<PaneType, string> = {
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

export const SIDEBAR_WIDTH = 160;

const SideBar: React.FC<SideBarProps> = ({ tabs, activeTabId, onTabClick, onAddTab, onCloseTab }) => {
  const [contextMenu, setContextMenu] = useState<{ tabId: string; y: number } | null>(null);
  const cmRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!contextMenu) return;
    const handler = (e: MouseEvent) => {
      if (cmRef.current && !cmRef.current.contains(e.target as Node)) setContextMenu(null);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [contextMenu]);

  return (
    <div style={{
      position: 'fixed',
      top: 0, left: 0, bottom: 0,
      width: `${SIDEBAR_WIDTH}px`,
      display: 'flex',
      flexDirection: 'column',
      paddingTop: '8px',
      gap: '2px',
      backgroundColor: 'var(--wks-bg-input)',
      borderRight: '1px solid var(--wks-border-subtle)',
      zIndex: 100,
      userSelect: 'none',
      boxSizing: 'border-box',
    }}>
      {tabs.map((tab) => {
        const isActive = tab.id === activeTabId;
        const firstType = tab.panes[0]?.type ?? 'terminal';
        const icon = typeIcons[firstType] ?? '?';

        return (
          <button
            key={tab.id}
            onClick={() => onTabClick(tab.id)}
            onContextMenu={(e) => {
              e.preventDefault();
              setContextMenu({ tabId: tab.id, y: e.clientY });
            }}
            style={{
              width: 'calc(100% - 8px)',
              height: '30px',
              margin: '0 4px',
              padding: '0 8px',
              display: 'flex',
              alignItems: 'center',
              gap: '10px',
              border: 'none',
              borderRadius: '6px',
              cursor: 'pointer',
              fontSize: '0.75rem',
              fontFamily: 'inherit',
              fontWeight: isActive ? 600 : 400,
              backgroundColor: isActive ? 'var(--wks-bg-selected)' : 'transparent',
              color: isActive ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
              borderLeft: isActive ? '2px solid var(--wks-accent)' : '2px solid transparent',
              transition: 'none',
              textAlign: 'left',
              boxSizing: 'border-box',
            }}
            title={tab.title}
          >
            <span style={{ width: 16, display: 'inline-flex', justifyContent: 'center', flexShrink: 0 }}>
              {icon}
            </span>
            <span style={{
              flex: 1,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}>
              {tab.title}
            </span>
          </button>
        );
      })}

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Add button */}
      {onAddTab && (
        <button
          onClick={() => onAddTab('terminal')}
          style={{
            width: 'calc(100% - 8px)',
            height: '30px',
            margin: '0 4px 8px',
            padding: '0 8px',
            display: 'flex',
            alignItems: 'center',
            gap: '10px',
            border: 'none',
            borderRadius: '6px',
            cursor: 'pointer',
            fontSize: '0.75rem',
            fontFamily: 'inherit',
            backgroundColor: 'transparent',
            color: 'var(--wks-text-faint)',
            textAlign: 'left',
            boxSizing: 'border-box',
          }}
          title="New tab"
        >
          <span style={{ width: 16, display: 'inline-flex', justifyContent: 'center', fontSize: '0.85rem' }}>+</span>
          <span>New tab</span>
        </button>
      )}

      {/* Context menu */}
      {contextMenu && (
        <div
          ref={cmRef}
          style={{
            position: 'fixed',
            left: `${SIDEBAR_WIDTH + 4}px`,
            top: contextMenu.y,
            backgroundColor: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: '4px',
            padding: '4px 0',
            zIndex: 10000,
            minWidth: '100px',
            boxShadow: '0 4px 12px var(--wks-shadow)',
          }}
        >
          <SideMenuItem label="Close" onClick={() => { setContextMenu(null); onCloseTab?.(contextMenu.tabId); }} />
        </div>
      )}
    </div>
  );
};

function SideMenuItem({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'block', width: '100%', padding: '4px 12px', margin: 0,
        border: 'none', borderRadius: 0, cursor: 'pointer',
        fontSize: '0.65rem', fontFamily: 'inherit', fontWeight: 400,
        backgroundColor: 'transparent', color: 'var(--wks-text-tertiary)',
        textAlign: 'left', height: 'auto', lineHeight: '1.4',
      }}
      onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
      onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
    >
      {label}
    </button>
  );
}

export default SideBar;
