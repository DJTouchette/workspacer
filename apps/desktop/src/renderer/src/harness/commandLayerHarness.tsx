/**
 * Command-layer chrome harness — every armed-state surface on one page, with
 * the REAL default+tmux keymap, for design review and screenshots without
 * launching Electron.
 *
 * Served by the normal Vite dev server: /command-layer-harness.html
 * (screenshots: spare-port vite + headless chromium, see deckHarness notes).
 *
 * Boxes (top to bottom):
 *   1. CommandStrip, compact — the just-armed strip
 *   2. CommandStrip, dwell-expanded HUD — the full grouped grid
 *   3. CommandStrip, inside the `g` submenu
 *   4. CommandStrip, compact with a pending-approval y/n hint
 *   5. FocusChip — all four faces
 *   6. PaneHints — badges over three fake panes
 *
 * Each strip lives in a `transform`ed box so its position:fixed anchors to
 * the box, not the page. `?theme=<id>` switches theme (default everforest).
 */
import React, { useEffect } from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';

const params = new URLSearchParams(window.location.search);

import { DEFAULT_CONFIG } from '../hooks/configDefaults';

// Minimal electronAPI stub BEFORE anything that touches it at module scope.
(window as any).electronAPI = new Proxy(
  {},
  {
    get: (_t, prop) => {
      if (prop === 'getConfig') return () => Promise.resolve(DEFAULT_CONFIG);
      return (..._args: unknown[]) => Promise.resolve(undefined);
    },
  },
);

import { applyTheme, resolveTheme } from '../themes';
import { DEFAULT_SHORTCUTS } from '../hooks/configDefaults';
import { KEYBINDING_PRESETS } from '../lib/keybindingPresets';
import CommandStrip from '../components/CommandStrip';
import { FocusChipView, type Face } from '../components/FocusChip';
import PaneHints from '../components/PaneHints';

const themeId = params.get('theme') ?? 'everforest';
const SHORTCUTS = { ...DEFAULT_SHORTCUTS, ...KEYBINDING_PRESETS.tmux.shortcuts };

const Box: React.FC<{ title: string; height?: number; children: React.ReactNode }> = ({
  title,
  height = 120,
  children,
}) => (
  <div style={{ padding: '10px 24px' }}>
    <div
      style={{
        fontFamily: 'var(--wks-font-mono)',
        fontSize: '0.68rem',
        color: 'var(--wks-text-faint)',
        padding: '0 0 6px 2px',
      }}
    >
      {title}
    </div>
    {/* transform makes this the containing block for fixed descendants. */}
    <div
      style={{
        position: 'relative',
        transform: 'translateZ(0)',
        height,
        border: '1px dashed var(--wks-border)',
        borderRadius: 'var(--wks-radius-md)',
        overflow: 'hidden',
        background: 'var(--wks-bg-base)',
      }}
    >
      {children}
    </div>
  </div>
);

const FakePane: React.FC<{ id: string; label: string }> = ({ id, label }) => (
  <div
    data-pane-id={id}
    style={{
      flex: 1,
      border: '1px solid var(--wks-border)',
      borderRadius: 'var(--wks-radius-md)',
      margin: 4,
      display: 'flex',
      alignItems: 'flex-start',
      padding: 10,
      color: 'var(--wks-text-faint)',
      fontFamily: 'var(--wks-font-mono)',
      fontSize: '0.7rem',
      background: 'var(--wks-bg-surface)',
    }}
  >
    {label}
  </div>
);

function Harness() {
  useEffect(() => {
    applyTheme(resolveTheme(themeId));
  }, []);

  return (
    <div style={{ width: 1600, paddingBottom: 24 }}>
      <Box title="CommandStrip — compact (just armed, tmux preset + layer verbs)" height={96}>
        <CommandStrip
          path={[]}
          prefix="ctrl+space"
          shortcuts={SHORTCUTS}
          hudDelayMs={999_999}
          attentionHint={null}
        />
      </Box>

      <Box title="CommandStrip — dwell-expanded HUD (the full grouped grid)" height={420}>
        <CommandStrip
          path={[]}
          prefix="ctrl+space"
          shortcuts={SHORTCUTS}
          hudDelayMs={1}
          attentionHint={null}
        />
      </Box>

      <Box title="CommandStrip — inside the g submenu (chat motions)" height={150}>
        <CommandStrip
          path={['g']}
          prefix="ctrl+space"
          shortcuts={SHORTCUTS}
          hudDelayMs={999_999}
          attentionHint={null}
        />
      </Box>

      <Box title="CommandStrip — compact with a pending approval (y/n context)" height={110}>
        <CommandStrip
          path={[]}
          prefix="ctrl+space"
          shortcuts={SHORTCUTS}
          hudDelayMs={999_999}
          attentionHint="Bash — rm -rf node_modules && npm install (workspacer)"
        />
      </Box>

      <Box title="FocusChip — all faces (bottom-right of each box)" height={70}>
        <div style={{ display: 'flex', height: '100%' }}>
          {(['insert', 'term', 'browse', 'app'] as Face[]).map((f) => (
            <div key={f} style={{ flex: 1, position: 'relative', transform: 'translateZ(0)' }}>
              <FocusChipView face={f} />
            </div>
          ))}
        </div>
      </Box>

      {/* PaneHints renders position:fixed badges from VIEWPORT rects, so it
          must not sit inside a transformed box — plain container here. */}
      <div style={{ padding: '10px 24px' }}>
        <div
          style={{
            fontFamily: 'var(--wks-font-mono)',
            fontSize: '0.68rem',
            color: 'var(--wks-text-faint)',
            padding: '0 0 6px 2px',
          }}
        >
          PaneHints — prefix d, next digit focuses (badges over live pane rects)
        </div>
        <div
          style={{
            display: 'flex',
            height: 220,
            border: '1px dashed var(--wks-border)',
            borderRadius: 'var(--wks-radius-md)',
            background: 'var(--wks-bg-base)',
          }}
        >
          <FakePane id="hx-1" label="claude — fixing tests" />
          <FakePane id="hx-2" label="terminal — npm run dev" />
          <FakePane id="hx-3" label="browser — localhost:5173" />
        </div>
        <PaneHintsWhenLaidOut />
      </div>
    </div>
  );
}

/** PaneHints measures DOM rects at render — defer one frame so the fake panes
 *  are laid out first. */
function PaneHintsWhenLaidOut() {
  const [ready, setReady] = React.useState(false);
  useEffect(() => {
    // setTimeout, not rAF: headless chromium's virtual-time screenshots do
    // not reliably advance animation frames.
    const t = setTimeout(() => setReady(true), 80);
    return () => clearTimeout(t);
  }, []);
  return ready ? <PaneHints paneIds={['hx-1', 'hx-2', 'hx-3']} /> : null;
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />);
