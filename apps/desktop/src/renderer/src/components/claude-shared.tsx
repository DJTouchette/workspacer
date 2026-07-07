/**
 * Shared Claude session UI components and utilities.
 * Used by ClaudePane for the full conversation GUI.
 */
import React from 'react';
import type { ClaudeSessionSnapshot, ToolCall } from '../types/claudeSession';
import { patchLineCounts } from '../lib/turnChanges';

// ── Color Palette ──

export const claudeColors = {
  bg: 'var(--wks-claude-bg)',
  bgSecondary: 'var(--wks-bg-hover)',
  bgToolbar: 'var(--wks-bg-surface)',
  userBubble: 'var(--wks-claude-user-bubble)',
  userBubbleBorder: 'var(--wks-claude-user-border)',
  muted: 'var(--wks-text-faint)',
  mutedDim: 'var(--wks-text-disabled)',
  accent: 'var(--wks-accent-text)',
  success: 'var(--wks-success)',
  error: 'var(--wks-error)',
  warning: 'var(--wks-warning)',
  purple: 'var(--wks-purple, #c084fc)',
  text: 'var(--wks-text-secondary)',
  textBright: 'var(--wks-text-primary)',
  border: 'var(--wks-claude-border)',
  borderSubtle: 'var(--wks-claude-border-subtle)',
  divider: 'var(--wks-claude-divider)',
};

// ── Badge colors & labels ──

export const badgeColors: Record<string, string> = {
  idle: claudeColors.success,
  thinking: claudeColors.warning,
  streaming: claudeColors.accent,
  waiting_input: 'var(--wks-purple, #c084fc)',
  waiting_approval: claudeColors.error,
};

export const badgeLabels: Record<string, string> = {
  idle: 'Idle',
  thinking: 'Thinking...',
  streaming: 'Streaming',
  waiting_input: 'Waiting for input',
  waiting_approval: 'Needs approval',
};

// ── CSS Keyframes (injected once) ──

const STYLE_ID = 'claude-pane-keyframes';

export function ensureKeyframes(): void {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = `
    @keyframes claudePulseDot {
      0%, 80%, 100% { opacity: 0.2; transform: scale(0.85); }
      40% { opacity: 1; transform: scale(1); }
    }
    @keyframes claudeSpinner {
      to { transform: rotate(360deg); }
    }
    @keyframes claudeFadeIn {
      from { opacity: 0; transform: translateY(4px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes claudeScrollBtn {
      from { opacity: 0; transform: translateY(8px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes claudeSlideUp {
      from { opacity: 0; transform: translateY(10px); }
      to { opacity: 1; transform: translateY(0); }
    }
  `;
  document.head.appendChild(style);
}

// ── StatusBadge ──

export function statusBadgeStyle(color: string): React.CSSProperties {
  return {
    display: 'inline-flex',
    alignItems: 'center',
    gap: 5,
    fontSize: '0.66rem',
    fontWeight: 600,
    letterSpacing: '0.02em',
    color: color,
    whiteSpace: 'nowrap',
  };
}

export const StatusBadge: React.FC<{
  session: ClaudeSessionSnapshot | null;
  approvalDismissed?: boolean;
}> = ({ session, approvalDismissed }) => {
  if (!session) return <span style={statusBadgeStyle('var(--wks-text-muted)')}>no session</span>;

  const state =
    session.ambientState === 'waiting_approval' && approvalDismissed
      ? 'thinking'
      : session.ambientState;

  const color = badgeColors[state] ?? '#555';
  const label = badgeLabels[state] ?? state;

  return (
    <span style={statusBadgeStyle(color)}>
      <span
        style={{
          display: 'inline-block',
          width: 7,
          height: 7,
          borderRadius: '50%',
          backgroundColor: color,
        }}
      />
      {label}
    </span>
  );
};

// ── Format tool call summary ──

export function formatToolSummary(tc: ToolCall): { call: string; result: string } {
  const fp = (p: string) => p?.split(/[/\\]/).pop() ?? '';

  switch (tc.name) {
    case 'Read': {
      const file = fp(tc.input?.file_path ?? '');
      const lines = tc.response ? String(tc.response).split('\n').length : 0;
      return { call: `Read(${file})`, result: lines > 0 ? `Read ${lines} lines` : '' };
    }
    case 'Edit':
    case 'MultiEdit': {
      const file = fp(tc.input?.file_path ?? '');
      const old = tc.input?.old_string ?? '';
      const nw = tc.input?.new_string ?? '';
      const added = nw ? nw.split('\n').length : 0;
      const removed = old ? old.split('\n').length : 0;
      const parts: string[] = [];
      if (removed > 0) parts.push(`-${removed}`);
      if (added > 0) parts.push(`+${added}`);
      return { call: `Edit(${file})`, result: parts.length ? parts.join(' ') + ' lines' : '' };
    }
    case 'Write': {
      const file = fp(tc.input?.file_path ?? '');
      const lines = tc.input?.content ? tc.input.content.split('\n').length : 0;
      return { call: `Write(${file})`, result: lines > 0 ? `${lines} lines` : '' };
    }
    // Codex file edits: `apply_patch` (opencode/pi: `patch`) carry the whole
    // unified patch under `diff`, so line counts come from its +/- lines.
    case 'apply_patch':
    case 'patch': {
      const file = fp(tc.input?.file_path ?? tc.input?.path ?? '');
      const counts = typeof tc.input?.diff === 'string' ? patchLineCounts(tc.input.diff) : null;
      const parts: string[] = [];
      if (counts?.removed) parts.push(`-${counts.removed}`);
      if (counts?.added) parts.push(`+${counts.added}`);
      return {
        call: `Edit(${file || 'patch'})`,
        result: parts.length ? parts.join(' ') + ' lines' : '',
      };
    }
    // Bash (claude) and its codex analogues: `shell` (app-server, argv array or
    // string) and `exec_command` (rollout, `cmd`).
    case 'Bash':
    case 'shell':
    case 'exec_command': {
      const raw = tc.input?.command ?? tc.input?.cmd ?? '';
      const flat = Array.isArray(raw) ? raw.join(' ') : String(raw);
      const cmd = flat.split('\n')[0].slice(0, 60);
      return { call: `${tc.name === 'Bash' ? 'Bash' : 'Shell'}(${cmd})`, result: '' };
    }
    case 'Grep':
    case 'Glob': {
      const pat = tc.input?.pattern ?? '';
      return { call: `Search(pattern: "${pat}")`, result: '' };
    }
    case 'web_search':
    case 'WebSearch': {
      const q = tc.input?.query ?? '';
      return { call: `Search("${q}")`, result: '' };
    }
    case 'Agent': {
      const desc = tc.input?.description ?? 'subagent';
      return { call: `Agent(${desc})`, result: '' };
    }
    case 'Workflow': {
      // input.script starts with `export const meta = { name: '...', ... }`
      const name =
        tc.input?.name ??
        /name:\s*['"`]([^'"`]+)['"`]/.exec(tc.input?.script ?? '')?.[1] ??
        'workflow';
      return { call: `Workflow(${name})`, result: '' };
    }
    default: {
      const vals = Object.values(tc.input ?? {});
      const firstStr = vals.find((v) => typeof v === 'string') as string | undefined;
      return { call: `${tc.name}(${firstStr?.slice(0, 40) ?? ''})`, result: '' };
    }
  }
}

// ── WorkLogEntry ──

export const WorkLogEntry: React.FC<{ tc: ToolCall }> = ({ tc }) => {
  const isRunning = tc.status === 'running';
  const isFailed = tc.status === 'failed';
  const iconColor = isRunning
    ? claudeColors.accent
    : isFailed
      ? claudeColors.error
      : claudeColors.success;
  const { call, result } = formatToolSummary(tc);

  return (
    <div style={{ padding: '1px 0' }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontSize: '0.75rem',
          lineHeight: 1.4,
        }}
      >
        {isRunning ? (
          <span
            style={{
              display: 'inline-block',
              width: 12,
              height: 12,
              border: `1.5px solid ${claudeColors.accent}`,
              borderTopColor: 'transparent',
              borderRadius: '50%',
              animation: 'claudeSpinner 0.8s linear infinite',
              flexShrink: 0,
            }}
          />
        ) : (
          <span
            style={{
              color: iconColor,
              fontSize: '0.72rem',
              width: 12,
              textAlign: 'center',
              flexShrink: 0,
            }}
          >
            {isFailed ? '\u2717' : '\u2713'}
          </span>
        )}
        <span
          style={{
            color: claudeColors.accent,
            fontFamily: 'var(--claude-mono-font, monospace)',
            fontSize: '0.72rem',
          }}
        >
          {call}
        </span>
      </div>
      {result && (
        <div
          style={{
            paddingLeft: 18,
            fontSize: '0.68rem',
            color: claudeColors.muted,
            lineHeight: 1.3,
          }}
        >
          {'\u23BF'} {result}
        </div>
      )}
    </div>
  );
};

// ── Approval helpers ──

/** Send approval response to Claude Code's interactive select menu */
export function sendApproval(ptyId: string, approve: boolean, write: (data: string) => void): void {
  if (approve) {
    write('\r');
  } else {
    write('\x1b[B\x1b[B\r');
  }
}

export function approvalBtnStyle(color: string): React.CSSProperties {
  return {
    fontSize: '0.7rem',
    fontWeight: 600,
    padding: '4px 16px',
    borderRadius: 6,
    border: `1px solid ${color}`,
    backgroundColor: 'transparent',
    color,
    cursor: 'pointer',
  };
}

// (The old three-dot StreamingDots indicator was replaced by the animated
// brand mark — see BrandSpinner in components/Brand.tsx.)
