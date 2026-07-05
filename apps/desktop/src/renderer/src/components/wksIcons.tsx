// Workspacer Icon Pack — 34 two-tone glyphs (24×24 grid, 2.2 stroke, round
// caps) purpose-built for the agent workspace: panes, agent status, actions,
// files/diff, and tools. Each icon is a stroke glyph in `currentColor` (so it
// tints with surrounding text) plus an optional "accent node" rendered in the
// theme accent (`--wks-accent`, overridable via the `accent` prop) for the
// two-tone look.
import React from 'react';

export interface WksIconProps {
  size?: number | string;
  strokeWidth?: number;
  className?: string;
  style?: React.CSSProperties;
  /** Color of the two-tone accent node/marks. Defaults to the theme accent. */
  accent?: string;
  /** Accepted for drop-in parity with lucide icons; the glyph tints via
   *  `currentColor` (set it through `style.color`), so this is ignored. */
  color?: string;
}

/** Build an icon component from a children-renderer that receives the accent. */
function glyph(render: (accent: string) => React.ReactNode) {
  const Icon: React.FC<WksIconProps> = ({
    size = 24,
    strokeWidth = 2.2,
    className,
    style,
    accent = 'var(--wks-accent)',
  }) => (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={style}
      aria-hidden="true"
    >
      {render(accent)}
    </svg>
  );
  return Icon;
}

// ── Navigation & panes ──────────────────────────────────────────────────────

export const IconInbox = glyph(() => (
  <>
    <path d="M22 12h-6l-2 3h-4l-2-3H2" />
    <path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
  </>
));

export const IconFleet = glyph((accent) => (
  <>
    <rect x="3" y="3" width="7" height="7" rx="1.6" />
    <rect x="14" y="3" width="7" height="7" rx="1.6" />
    <rect x="3" y="14" width="7" height="7" rx="1.6" />
    <rect x="14" y="14" width="7" height="7" rx="1.6" fill={accent} stroke="none" />
  </>
));

export const IconOverview = glyph((accent) => (
  <>
    <rect x="3" y="3" width="7" height="9" rx="1.6" />
    <rect x="14" y="3" width="7" height="5" rx="1.6" />
    <rect x="14" y="12" width="7" height="9" rx="1.6" />
    <rect x="3" y="16" width="7" height="5" rx="1.6" />
    <circle cx="6.5" cy="7.5" r="1.3" fill={accent} stroke="none" />
  </>
));

export const IconUsage = glyph((accent) => (
  <>
    <line x1="3" y1="21" x2="21" y2="21" />
    <line x1="6.5" y1="21" x2="6.5" y2="14" />
    <line x1="12" y1="21" x2="12" y2="10" />
    <line x1="17.5" y1="21" x2="17.5" y2="6" stroke={accent} />
  </>
));

export const IconSearch = glyph((accent) => (
  <>
    <circle cx="11" cy="11" r="7" />
    <line x1="21" y1="21" x2="16.65" y2="16.65" />
    <circle cx="11" cy="11" r="1.7" fill={accent} stroke="none" />
  </>
));

export const IconSettings = glyph((accent) => (
  <>
    <line x1="4" y1="7" x2="20" y2="7" />
    <line x1="4" y1="12" x2="20" y2="12" />
    <line x1="4" y1="17" x2="20" y2="17" />
    <circle cx="15" cy="7" r="2.1" fill={accent} stroke="none" />
    <circle cx="9" cy="12" r="2.1" fill={accent} stroke="none" />
    <circle cx="13" cy="17" r="2.1" fill={accent} stroke="none" />
  </>
));

export const IconAgent = glyph((accent) => (
  <>
    <path d="M12 3 20 7.5V16.5L12 21 4 16.5V7.5z" />
    <circle cx="12" cy="12" r="2.6" fill={accent} stroke="none" />
  </>
));

// ── Agent status ────────────────────────────────────────────────────────────

export const IconWorking = glyph((accent) => (
  <>
    <path d="M3 12h4l2.5 7 4-14 2.5 7H21" />
    <circle cx="3" cy="12" r="1.5" fill={accent} stroke="none" />
  </>
));

export const IconReviewing = glyph((accent) => (
  <>
    <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z" />
    <circle cx="12" cy="12" r="2.6" fill={accent} stroke="none" />
  </>
));

export const IconIdle = glyph(() => (
  <>
    <rect x="3" y="3" width="18" height="18" rx="4" />
    <line x1="10" y1="9" x2="10" y2="15" />
    <line x1="14" y1="9" x2="14" y2="15" />
  </>
));

export const IconQueued = glyph((accent) => (
  <>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l3 2" stroke={accent} />
  </>
));

export const IconError = glyph(() => (
  <>
    <path d="M10.29 4 2.5 18a2 2 0 0 0 1.71 3h15.58a2 2 0 0 0 1.71-3L13.71 4a2 2 0 0 0-3.42 0z" />
    <line x1="12" y1="9.5" x2="12" y2="13.5" />
    <circle cx="12" cy="17" r="1.05" fill="currentColor" stroke="none" />
  </>
));

export const IconDone = glyph((accent) => (
  <>
    <circle cx="12" cy="12" r="9" />
    <path d="M8 12.5l2.6 2.6L16 9.5" stroke={accent} />
  </>
));

// ── Actions ─────────────────────────────────────────────────────────────────

export const IconSpawn = glyph((accent) => (
  <>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 8v8M8 12h8" stroke={accent} />
  </>
));

export const IconRun = glyph((accent) => (
  <path d="M8 5.5v13l11-6.5z" fill={accent} stroke={accent} />
));

export const IconStop = glyph(() => <rect x="6" y="6" width="12" height="12" rx="2.5" />);

export const IconPause = glyph(() => (
  <>
    <rect x="7" y="5" width="3.4" height="14" rx="1" fill="currentColor" stroke="none" />
    <rect x="13.6" y="5" width="3.4" height="14" rx="1" fill="currentColor" stroke="none" />
  </>
));

export const IconRetry = glyph((accent) => (
  <>
    <path d="M21 12a9 9 0 1 1-2.64-6.36" stroke={accent} />
    <path d="M21 3.5v5h-5" stroke={accent} />
  </>
));

export const IconApprove = glyph((accent) => <path d="M5 13l4 4L19 7" stroke={accent} />);

export const IconReject = glyph(() => <path d="M6 6l12 12M18 6 6 18" />);

export const IconMerge = glyph((accent) => (
  <>
    <circle cx="6" cy="6" r="2.5" />
    <circle cx="6" cy="18" r="2.5" />
    <circle cx="18" cy="8" r="2.5" fill={accent} stroke="none" />
    <path d="M6 8.5v7" />
    <path d="M18 10.5c0 3.5-3 5-6 5H6" />
  </>
));

// ── Files & diff ────────────────────────────────────────────────────────────

export const IconFolder = glyph(() => (
  <path d="M4 8a2 2 0 0 1 2-2h3.2a2 2 0 0 1 1.4.6L12 8h6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" />
));

export const IconFile = glyph(() => (
  <>
    <path d="M14.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
    <path d="M14 3v6h5" />
  </>
));

export const IconAdded = glyph((accent) => (
  <>
    <path d="M14.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
    <path d="M14 3v6h5" />
    <path d="M12 12.5v5M9.5 15h5" stroke={accent} />
  </>
));

export const IconRemoved = glyph((accent) => (
  <>
    <path d="M14.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
    <path d="M14 3v6h5" />
    <path d="M9.5 15h5" stroke={accent} />
  </>
));

export const IconModified = glyph((accent) => (
  <>
    <path d="M14.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
    <path d="M14 3v6h5" />
    <circle cx="12" cy="15" r="1.8" fill={accent} stroke="none" />
  </>
));

export const IconDiff = glyph((accent) => (
  <>
    <path d="M12 4v6M9 7h6" stroke={accent} />
    <path d="M9 16h6" stroke={accent} />
  </>
));

// ── Tools & integrations ────────────────────────────────────────────────────

export const IconTerminal = glyph((accent) => (
  <>
    <rect x="3" y="4" width="18" height="16" rx="2.5" />
    <path d="M7 9.5l3 2.5-3 2.5" stroke={accent} />
    <line x1="12.5" y1="15" x2="16" y2="15" stroke={accent} />
  </>
));

export const IconBranch = glyph((accent) => (
  <>
    <line x1="6" y1="9" x2="6" y2="15" />
    <circle cx="6" cy="18" r="2.5" />
    <circle cx="6" cy="6" r="2.5" />
    <circle cx="18" cy="6" r="2.5" fill={accent} stroke="none" />
    <path d="M18 8.5a9 9 0 0 1-9 9" />
  </>
));

export const IconCommit = glyph((accent) => (
  <>
    <line x1="3" y1="12" x2="9" y2="12" />
    <line x1="15" y1="12" x2="21" y2="12" />
    <circle cx="12" cy="12" r="3.2" fill={accent} stroke={accent} />
  </>
));

export const IconPlugin = glyph((accent) => (
  <>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M19.1 4.9 17 7M7 17l-2.1 2.1" />
    <circle cx="12" cy="12" r="1.6" fill={accent} stroke="none" />
  </>
));

export const IconModel = glyph((accent) => (
  <>
    <rect x="6" y="6" width="12" height="12" rx="2" />
    <rect x="9.5" y="9.5" width="5" height="5" rx="1" fill={accent} stroke="none" />
    <path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2" />
  </>
));

export const IconDeploy = glyph((accent) => (
  <>
    <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
    <path d="m3.3 7 8.7 5 8.7-5" />
    <path d="M12 22V12" />
    <circle cx="12" cy="12" r="1.5" fill={accent} stroke="none" />
  </>
));

export const IconNotify = glyph((accent) => (
  <>
    <path d="M6 8.5a6 6 0 0 1 12 0c0 7 3 8.5 3 8.5H3s3-1.5 3-8.5" />
    <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
    <circle cx="18" cy="5" r="2.4" fill={accent} stroke="none" />
  </>
));

// ── Registry ────────────────────────────────────────────────────────────────

/** Every glyph keyed by its pack name, for data-driven lookup. */
export const WKS_ICONS = {
  // navigation & panes
  inbox: IconInbox,
  fleet: IconFleet,
  overview: IconOverview,
  usage: IconUsage,
  search: IconSearch,
  settings: IconSettings,
  agent: IconAgent,
  // agent status
  working: IconWorking,
  reviewing: IconReviewing,
  idle: IconIdle,
  queued: IconQueued,
  error: IconError,
  done: IconDone,
  // actions
  spawn: IconSpawn,
  run: IconRun,
  stop: IconStop,
  pause: IconPause,
  retry: IconRetry,
  approve: IconApprove,
  reject: IconReject,
  merge: IconMerge,
  // files & diff
  folder: IconFolder,
  file: IconFile,
  added: IconAdded,
  removed: IconRemoved,
  modified: IconModified,
  diff: IconDiff,
  // tools & integrations
  terminal: IconTerminal,
  branch: IconBranch,
  commit: IconCommit,
  plugin: IconPlugin,
  model: IconModel,
  deploy: IconDeploy,
  notify: IconNotify,
} as const;

export type WksIconName = keyof typeof WKS_ICONS;

/** Render a pack glyph by name: `<WksIcon name="working" size={16} />`. */
export const WksIcon: React.FC<{ name: WksIconName } & WksIconProps> = ({ name, ...rest }) => {
  const Cmp = WKS_ICONS[name];
  return <Cmp {...rest} />;
};
