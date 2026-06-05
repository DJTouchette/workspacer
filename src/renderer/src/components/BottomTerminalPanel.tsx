import React, { useState, useEffect } from 'react';
import TerminalPane from '../panes/TerminalPane';
import { SIDEBAR_WIDTH } from './SideBar';

interface Props {
  visible: boolean;
  onClose: () => void;
  /** Directory the terminal spawns in (captured at first open). */
  cwd?: string;
  /** Left edge (px) — follows the sidebar so it spans full width when collapsed. */
  left?: number;
}

const PANEL_H = '40vh';

/**
 * A VS Code-style integrated terminal that slides up from the bottom. The
 * terminal is mounted on first open and then kept mounted forever — toggling
 * only slides it off-screen — so the PTY keeps running and scrollback survives.
 * Toggled with Ctrl+` (and Esc closes).
 */
const BottomTerminalPanel: React.FC<Props> = ({ visible, onClose, cwd, left = SIDEBAR_WIDTH }) => {
  // Lazily mount the terminal the first time it's opened, then never unmount it
  // (unmounting would dispose xterm + drop the PTY).
  const [mounted, setMounted] = useState(false);
  useEffect(() => { if (visible) setMounted(true); }, [visible]);

  // No Escape-to-close here: Escape is a real key inside a terminal (vim, less,
  // readline). Toggle with Ctrl+` or the close button instead.

  return (
    <div
      aria-hidden={!visible}
      style={{
        position: 'fixed', left, right: 0, bottom: 0, height: PANEL_H,
        zIndex: 1700, display: 'flex', flexDirection: 'column',
        backgroundColor: 'var(--wks-bg-surface)',
        borderTop: '1px solid var(--wks-glass-border)',
        // Respect the theme's corner setting (0px when square) on the top edge.
        borderTopLeftRadius: 'var(--wks-radius-lg)',
        borderTopRightRadius: 'var(--wks-radius-lg)',
        boxShadow: '0 -8px 28px var(--wks-shadow)',
        transform: visible ? 'translateY(0)' : 'translateY(100%)',
        // Instant open/close — no slide animation.
        pointerEvents: visible ? 'auto' : 'none',
        overflow: 'hidden',
      }}
    >
      {/* No header/title bar — toggle with Ctrl+`. */}
      <div style={{ flex: 1, minHeight: 0 }}>
        {mounted && (
          <TerminalPane
            paneId="__bottom_terminal__"
            title="Terminal"
            isActive={visible}
            cwd={cwd}
          />
        )}
      </div>
    </div>
  );
};

export default BottomTerminalPanel;
