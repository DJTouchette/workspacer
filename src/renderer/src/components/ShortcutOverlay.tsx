import React, { useEffect } from 'react';

interface ShortcutOverlayProps {
  visible: boolean;
  onClose: () => void;
  mode?: 'default' | 'vim';
  leader?: string;
}

const defaultShortcuts: [string, string][] = [
  ['', 'Navigation'],
  ['Ctrl+1-9', 'Jump to tab'],
  ['Ctrl+Alt+Left/Right', 'Previous / next tab (wraps)'],
  ['Alt+Arrow', 'Navigate split panes'],
  ['', ''],
  ['', 'Tabs & Panes'],
  ['Ctrl+T', 'New terminal tab'],
  ['Ctrl+B', 'New browser tab'],
  ['Ctrl+D', 'Split current tab'],
  ['Ctrl+W', 'Close pane / tab'],
  ['Ctrl+Shift+1-9', 'Move tab to position'],
  ['F2', 'Rename tab'],
  ['', ''],
  ['', 'Tools'],
  ['Ctrl+K', 'Command palette (apps + panes)'],
  ['Ctrl+S', 'Save session'],
  ['Ctrl+,', 'Open settings'],
  ['Ctrl+/', 'Toggle this help'],
  ['Esc', 'Cancel (in Claude GUI mode)'],
];

function vimShortcutList(leader: string): [string, string][] {
  const l = leader || 'ctrl';
  return [
    [`${l} \u2192 1-9`, 'Jump to tab'],
    [`${l} \u2192 h / l`, 'Prev / next tab'],
    [`${l} \u2192 H / L`, 'Move tab left / right'],
    [`${l} \u2192 n`, 'New terminal tab'],
    [`${l} \u2192 b`, 'New browser tab'],
    [`${l} \u2192 d`, 'Split current tab'],
    [`${l} \u2192 q`, 'Close tab'],
    [`${l} \u2192 r`, 'Rename tab'],
    [`${l} \u2192 ?`, 'Toggle help'],
    [`${l} \u2192 s`, 'Save session'],
    ['', ''],
    ['Direct shortcuts:', ''],
    ['Ctrl+T / Ctrl+B', 'New terminal / browser tab'],
    ['Ctrl+D', 'Split current tab'],
    ['Ctrl+K', 'Launch app'],
    ['Ctrl+W', 'Close pane / tab'],
    ['Ctrl+S', 'Save session'],
    ['Ctrl+,', 'Open settings'],
    ['Ctrl+/', 'Toggle this help'],
  ];
}

const ShortcutOverlay: React.FC<ShortcutOverlayProps> = ({ visible, onClose, mode = 'default', leader = 'ctrl' }) => {
  useEffect(() => {
    if (!visible) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [visible, onClose]);

  if (!visible) return null;

  const entries = mode === 'vim' ? vimShortcutList(leader) : defaultShortcuts;
  const title = mode === 'vim' ? 'Keyboard Shortcuts (Vim Mode)' : 'Keyboard Shortcuts';

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: 'var(--wks-overlay)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 1000,
        transition: 'none',
      }}
      onClick={onClose}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-bg-surface)',
          border: '1px solid var(--wks-border-input)',
          borderRadius: '6px',
          padding: '12px 16px',
          minWidth: '280px',
          maxWidth: '400px',
          transition: 'none',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          style={{
            fontSize: '0.75rem',
            fontWeight: 600,
            color: 'var(--wks-text-secondary)',
            marginBottom: '8px',
            borderBottom: '1px solid var(--wks-border)',
            paddingBottom: '6px',
          }}
        >
          {title}
        </div>
        <table
          style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontSize: '0.75rem',
          }}
        >
          <tbody>
            {entries.map(([key, desc], i) => {
              // Empty row = spacer
              if (!key && !desc) {
                return <tr key={i}><td colSpan={2} style={{ height: '6px' }} /></tr>;
              }
              // Label row (no desc)
              if (!desc) {
                return (
                  <tr key={i}>
                    <td colSpan={2} style={{
                      padding: '4px 0 2px',
                      color: 'var(--wks-text-faint)',
                      fontSize: '0.6rem',
                      fontWeight: 600,
                    }}>
                      {key}
                    </td>
                  </tr>
                );
              }
              return (
                <tr key={i}>
                  <td
                    style={{
                      padding: '2px 12px 2px 0',
                      color: 'var(--wks-text-tertiary)',
                      fontFamily: 'monospace',
                      fontSize: '0.65rem',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {key}
                  </td>
                  <td
                    style={{
                      padding: '2px 0',
                      color: 'var(--wks-text-muted)',
                    }}
                  >
                    {desc}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default ShortcutOverlay;
