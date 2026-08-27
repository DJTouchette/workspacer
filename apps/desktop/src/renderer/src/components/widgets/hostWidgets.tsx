import React, { useEffect, useState } from 'react';
import { GitBranch, Gauge } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import type { WidgetSize } from '../../types/widget';
import { claudeColors as colors } from '../claude-shared';
import { GitClient, type GitStatus } from '../../lib/gitQueries';

/**
 * Built-in widgets — the host's own contributions to a project's widget board.
 *
 * These exist to prove the size vocabulary before third-party widgets rely on
 * it: if a class can't be made to read at 148px with data we already have, no
 * plugin author will manage it either. They render inline (plain React, no
 * webview), which is why they cost nothing next to a plugin widget's guest.
 *
 * A host widget must obey the same contract a plugin widget does: cheap,
 * glanceable, no continuous animation, and safe to unmount at any moment — the
 * board tears down whenever the rail closes.
 */

export interface HostWidgetProps {
  cwd: string;
  size: WidgetSize;
  /** The piloted session, when the board is shown beside one. May be null. */
  snapshot: ClaudeSessionSnapshot | null;
}

export interface HostWidgetDef {
  id: string;
  title: string;
  icon: React.ReactNode;
  /** Footprints this widget reads well at, smallest first. */
  sizes: WidgetSize[];
  Render: React.FC<HostWidgetProps>;
}

// --- shared bits -----------------------------------------------------------

/**
 * The tile's own layout. A small tile is one glanceable fact, so it centres —
 * the tile has no title bar to balance against (see WidgetCell), and content
 * pinned to the top-left of a 148px square just leaves a hole under it. Medium
 * and large carry a list, which has to start at the top to be readable.
 */
const Tile: React.FC<{ size: WidgetSize; children: React.ReactNode }> = ({ size, children }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100%',
      width: '100%',
      minWidth: 0,
      gap: 6,
      justifyContent: size === 'small' ? 'center' : 'flex-start',
      alignItems: size === 'small' ? 'center' : 'stretch',
      textAlign: size === 'small' ? 'center' : 'left',
    }}
  >
    {children}
  </div>
);

/** The big number every small widget leads with. */
const Stat: React.FC<{ value: React.ReactNode; label: string; tone?: string }> = ({
  value,
  label,
  tone,
}) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
    <div
      style={{
        // A display number, not chrome text: off the type scale on purpose, and
        // the same step the plugin widgets lead with so a board of host and
        // third-party tiles reads as one system.
        fontSize: '1.55rem',
        lineHeight: 1.05,
        fontWeight: 600,
        color: tone ?? colors.textBright,
        fontVariantNumeric: 'tabular-nums',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      {value}
    </div>
    <div
      style={{
        fontSize: '0.6rem',
        color: colors.mutedDim,
        textTransform: 'uppercase',
        letterSpacing: 0.4,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </div>
  </div>
);

const Empty: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: 11, color: colors.mutedDim, alignSelf: 'center', margin: 'auto' }}>
    {children}
  </div>
);

// --- git -------------------------------------------------------------------

const git = new GitClient();

/**
 * Branch, working-tree dirtiness, and upstream drift.
 *
 * Polls rather than subscribing: there is no git watcher on the host, and a
 * widget must not be the thing that introduces one. 15s is slow enough to be
 * invisible in a profile and fast enough that a commit shows up before you look
 * away. The poll stops the moment the widget unmounts (rail closed).
 */
const GitWidget: React.FC<HostWidgetProps> = ({ cwd, size }) => {
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!cwd) return;
    let alive = true;
    const tick = () => {
      git
        .status(cwd)
        .then((s) => {
          if (!alive) return;
          setStatus(s);
          setFailed(false);
        })
        .catch(() => {
          if (alive) setFailed(true);
        });
    };
    tick();
    const id = setInterval(tick, 15_000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [cwd]);

  if (failed) return <Empty>Not a git repo</Empty>;
  if (!status) return <Empty>…</Empty>;

  const dirty = status.files.length;
  const ahead = status.ahead ?? 0;
  const behind = status.behind ?? 0;

  return (
    <Tile size={size}>
      <Stat
        value={dirty === 0 ? 'Clean' : dirty}
        label={dirty === 0 ? 'working tree' : dirty === 1 ? 'changed file' : 'changed files'}
        tone={dirty === 0 ? colors.muted : undefined}
      />
      {/* The changed files, between the stat and the branch — the branch is the
          tile's footer and stays last, whatever is above it. */}
      {size !== 'small' && dirty > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, overflow: 'hidden' }}>
          {status.files.slice(0, size === 'large' ? 8 : 2).map((f) => (
            <div
              key={f.path}
              style={{
                fontSize: '0.66rem',
                color: colors.mutedDim,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                direction: 'rtl', // keep the filename visible when the path is long
                textAlign: 'left',
              }}
            >
              {f.path}
            </div>
          ))}
        </div>
      )}
      <div
        style={{
          // A small tile is centred as a block, so nothing inside it may claim
          // the leftover space — `auto` there would push the branch back to the
          // bottom edge and undo the centring.
          marginTop: size === 'small' ? undefined : 'auto',
          display: 'flex',
          alignItems: 'center',
          justifyContent: size === 'small' ? 'center' : 'flex-start',
          gap: 6,
          fontSize: '0.66rem',
          color: colors.mutedDim,
          minWidth: 0,
          maxWidth: '100%',
        }}
      >
        <GitBranch size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {status.branch ?? 'detached'}
        </span>
        {(ahead > 0 || behind > 0) && (
          <span style={{ flexShrink: 0, color: colors.text }}>
            {ahead > 0 && `↑${ahead}`}
            {behind > 0 && `↓${behind}`}
          </span>
        )}
      </div>
    </Tile>
  );
};

// --- usage -----------------------------------------------------------------

/** Context window pressure + spend for the session the board sits beside. */
const UsageWidget: React.FC<HostWidgetProps> = ({ size, snapshot }) => {
  const usage = snapshot?.usage ?? null;
  if (!usage) return <Empty>No active session</Empty>;

  // A null limit is UNKNOWN, not zero: no bar and no tone, rather than a
  // coloured meter against an invented denominator.
  const pct = usage.contextLimit
    ? Math.min(100, Math.round((usage.contextTokens / usage.contextLimit) * 100))
    : null;
  // Context pressure is the number worth colouring: past ~90% a compaction is
  // imminent and that changes what you do next.
  const tone = pct === null ? undefined : pct >= 90 ? '#e5534b' : pct >= 70 ? '#d29922' : undefined;

  return (
    <Tile size={size}>
      <Stat value={pct === null ? '—' : `${pct}%`} label="context used" tone={tone} />
      {pct !== null && (
        <div
          style={{
            width: '100%',
            height: 3,
            borderRadius: 'var(--wks-radius-pill)',
            background: 'var(--wks-bg-hover)',
            overflow: 'hidden',
          }}
        >
          <div style={{ width: `${pct}%`, height: '100%', background: tone ?? colors.muted }} />
        </div>
      )}
      <div
        style={{
          marginTop: size === 'small' ? undefined : 'auto',
          fontSize: '0.66rem',
          color: colors.mutedDim,
        }}
      >
        ${usage.costUSD.toFixed(2)}
        {size !== 'small' && usage.model ? ` · ${usage.model}` : ''}
      </div>
    </Tile>
  );
};

// --- registry --------------------------------------------------------------

export const HOST_WIDGETS: HostWidgetDef[] = [
  {
    id: 'git',
    title: 'Git',
    icon: <GitBranch size={13} strokeWidth={2} />,
    sizes: ['small', 'medium', 'large'],
    Render: GitWidget,
  },
  {
    id: 'usage',
    title: 'Usage',
    icon: <Gauge size={13} strokeWidth={2} />,
    sizes: ['small', 'medium'],
    Render: UsageWidget,
  },
];

export function hostWidget(id: string): HostWidgetDef | undefined {
  return HOST_WIDGETS.find((w) => w.id === id);
}
