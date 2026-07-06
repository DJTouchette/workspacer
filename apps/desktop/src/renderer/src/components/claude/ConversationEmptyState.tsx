import React from 'react';
import { claudeColors as colors } from '../claude-shared';

/** Starter prompts: short chip label → full prompt dropped into the composer
 *  (not auto-sent — the user can edit before committing). */
const STARTERS: { label: string; prompt: string }[] = [
  {
    label: 'Orient me',
    prompt:
      'Give me a quick orientation: what is this codebase, how is it structured, and where does the interesting logic live?',
  },
  {
    label: 'What changed recently?',
    prompt: 'Summarize the recent git history — what has been worked on lately, by theme?',
  },
  {
    label: 'Find something to fix',
    prompt: 'Scan for bugs or code smells and propose the top 3 fixes, most impactful first.',
  },
  {
    label: 'Improve test coverage',
    prompt: 'Find an under-tested area of this codebase and add meaningful tests for it.',
  },
];

export const ConversationEmptyState: React.FC<{
  agentName: string;
  model?: string;
  permissionMode?: string;
  transport?: 'pty' | 'stream';
  cwd?: string;
  onPick: (prompt: string) => void;
}> = ({ agentName, model, permissionMode, transport, cwd, onPick }) => {
  const dirName = cwd ? (cwd.replace(/\/+$/, '').split('/').pop() ?? cwd) : undefined;
  const meta = [
    model,
    permissionMode,
    transport === 'stream' ? 'headless' : undefined,
    dirName,
  ].filter(Boolean) as string[];

  return (
    <div
      style={{
        textAlign: 'center',
        marginTop: 48,
        color: colors.mutedDim,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      <div style={{ fontSize: '1.6rem', marginBottom: 10, opacity: 0.35 }}>{'◆'}</div>
      <div style={{ fontSize: '0.85rem', color: colors.textBright, fontWeight: 600 }}>
        {agentName} is ready
      </div>
      {meta.length > 0 && (
        <div
          style={{
            fontSize: '0.68rem',
            marginTop: 6,
            color: colors.muted,
            display: 'flex',
            justifyContent: 'center',
            alignItems: 'center',
            gap: 6,
            flexWrap: 'wrap',
          }}
          title={cwd}
        >
          {meta.map((m, i) => (
            <React.Fragment key={m}>
              {i > 0 && <span style={{ color: colors.mutedDim }}>{'·'}</span>}
              <span>{m}</span>
            </React.Fragment>
          ))}
        </div>
      )}

      <div
        style={{
          display: 'flex',
          flexWrap: 'wrap',
          justifyContent: 'center',
          gap: 6,
          marginTop: 20,
          maxWidth: 460,
          marginLeft: 'auto',
          marginRight: 'auto',
        }}
      >
        {STARTERS.map((s) => (
          <button
            key={s.label}
            onClick={() => onPick(s.prompt)}
            title={s.prompt}
            style={{
              fontSize: '0.7rem',
              fontWeight: 500,
              padding: '5px 12px',
              borderRadius: 'var(--wks-radius-pill)',
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.03)',
              color: colors.text,
              cursor: 'pointer',
              fontFamily: 'inherit',
              transition: 'border-color 0.15s, color 0.15s',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.borderColor = 'var(--wks-border-active)';
              e.currentTarget.style.color = colors.textBright;
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.borderColor = colors.borderSubtle;
              e.currentTarget.style.color = colors.text;
            }}
          >
            {s.label}
          </button>
        ))}
      </div>

      <div style={{ fontSize: '0.62rem', marginTop: 18, color: colors.mutedDim }}>
        Enter to send · Shift+Enter for a newline · + to attach files
      </div>
    </div>
  );
};
