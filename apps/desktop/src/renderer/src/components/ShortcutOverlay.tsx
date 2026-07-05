import React, { useEffect } from 'react';
import { formatBinding, ACTION_SECTIONS } from '../lib/shortcuts';

interface ShortcutOverlayProps {
  visible: boolean;
  onClose: () => void;
  /** Workspace prefix combo, used to render 'prefix …' chords. */
  prefix?: string;
  shortcuts?: Record<string, string>;
}

const ShortcutOverlay: React.FC<ShortcutOverlayProps> = ({
  visible,
  onClose,
  prefix = 'ctrl+space',
  shortcuts = {},
}) => {
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
        backgroundColor: 'var(--wks-overlay)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 1000,
      }}
      onClick={onClose}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-md)',
          boxShadow:
            '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
          padding: '12px 16px',
          width: 'min(620px, 94vw)',
          boxSizing: 'border-box',
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
          Keyboard Shortcuts
        </div>

        <div
          style={{
            fontSize: '0.62rem',
            color: 'var(--wks-text-faint)',
            marginBottom: '10px',
          }}
        >
          Prefix is{' '}
          <code style={{ fontFamily: 'monospace', color: 'var(--wks-text-tertiary)' }}>
            {formatBinding(prefix)}
          </code>{' '}
          — press it, then the key.
        </div>

        {ACTION_SECTIONS.map((section) => (
          <div key={section.section} style={{ marginBottom: '10px' }}>
            <div
              style={{
                fontSize: '0.6rem',
                fontWeight: 600,
                color: 'var(--wks-text-faint)',
                padding: '4px 0 2px',
                textTransform: 'uppercase',
                letterSpacing: '0.03em',
              }}
            >
              {section.section}
            </div>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.72rem' }}>
              <tbody>
                {section.items.map(({ action, label }) => {
                  const combo = formatBinding(shortcuts[action] ?? action, prefix);

                  return (
                    <tr key={action}>
                      <td
                        style={{
                          padding: '2px 12px 2px 0',
                          color: 'var(--wks-text-tertiary)',
                          fontFamily: 'monospace',
                          fontSize: '0.65rem',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {combo}
                      </td>
                      <td style={{ padding: '2px 0', color: 'var(--wks-text-muted)' }}>{label}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ))}

        <div
          style={{
            fontSize: '0.6rem',
            color: 'var(--wks-text-faint)',
            textAlign: 'center',
            marginTop: '8px',
            borderTop: '1px solid var(--wks-border)',
            paddingTop: '6px',
          }}
        >
          Press Esc to close · Edit in Settings → Keybindings
        </div>
      </div>
    </div>
  );
};

export default ShortcutOverlay;
