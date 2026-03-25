import React, { useEffect } from 'react';

interface ShortcutOverlayProps {
  visible: boolean;
  onClose: () => void;
  mode?: 'default' | 'vim';
  leader?: string;
}

const defaultShortcuts: [string, string][] = [
  ['Ctrl+1-9', 'Jump to pane'],
  ['Alt+Left/Right', 'Previous/Next pane'],
  ['Ctrl+T', 'New terminal'],
  ['Ctrl+B', 'New browser'],
  ['Ctrl+W', 'Close active pane'],
  ['Ctrl+Shift+Left', 'Shrink pane'],
  ['Ctrl+Shift+Right', 'Grow pane'],
  ['Ctrl+Shift+0', 'Reset pane width'],
  ['Ctrl+Shift+1-9', 'Move pane to position'],
  ['F2', 'Rename active pane'],
  ['Dbl-click title', 'Rename pane'],
  ['Drag header', 'Reorder pane'],
  ['Ctrl+,', 'Open settings'],
  ['Ctrl+/', 'Toggle this help'],
];

function vimShortcutList(leader: string): [string, string][] {
  const l = leader || 'ctrl';
  return [
    [`${l} \u2192 1-9`, 'Jump to pane'],
    [`${l} \u2192 h / l`, 'Prev / next pane'],
    [`${l} \u2192 H / L`, 'Move pane left / right'],
    [`${l} \u2192 n`, 'New terminal'],
    [`${l} \u2192 b`, 'New browser'],
    [`${l} \u2192 q`, 'Close pane'],
    [`${l} \u2192 r`, 'Rename pane'],
    [`${l} \u2192 + / -`, 'Grow / shrink pane'],
    [`${l} \u2192 =`, 'Reset pane width'],
    [`${l} \u2192 ?`, 'Toggle help'],
    ['', ''],
    ['Direct shortcuts:', ''],
    ['Ctrl+T / Ctrl+B', 'New terminal / browser'],
    ['Ctrl+W', 'Close pane'],
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
        backgroundColor: 'rgba(0, 0, 0, 0.6)',
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
          backgroundColor: 'rgb(30, 30, 33)',
          border: '1px solid rgb(55, 55, 60)',
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
            color: 'rgb(200, 200, 210)',
            marginBottom: '8px',
            borderBottom: '1px solid rgb(50, 50, 55)',
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
                      color: 'rgb(120, 120, 135)',
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
                      color: 'rgb(170, 170, 185)',
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
                      color: 'rgb(140, 140, 155)',
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
