import React, { useEffect } from 'react';

interface ShortcutOverlayProps {
  visible: boolean;
  onClose: () => void;
}

const shortcuts: [string, string][] = [
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
  ['Ctrl+/', 'Toggle this help'],
];

const ShortcutOverlay: React.FC<ShortcutOverlayProps> = ({ visible, onClose }) => {
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
          minWidth: '260px',
          maxWidth: '340px',
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
          Keyboard Shortcuts
        </div>
        <table
          style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontSize: '0.75rem',
          }}
        >
          <tbody>
            {shortcuts.map(([key, desc]) => (
              <tr key={key}>
                <td
                  style={{
                    padding: '2px 12px 2px 0',
                    color: 'rgb(170, 170, 185)',
                    fontFamily: 'monospace',
                    fontSize: '0.7rem',
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
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default ShortcutOverlay;
