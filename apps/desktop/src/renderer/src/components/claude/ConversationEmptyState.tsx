import React, { useEffect, useState } from 'react';
import { claudeColors as colors } from '../claude-shared';
import { AgentLogo } from '../agentLogos';
import type { AgentProvider } from '../../types/pane';

/** Spawn-dialog visual language for a pane's beginning screens: a soft accent
 *  glow behind a circular provider badge, then a title. Shared by the
 *  connecting / ready / failed empty states so they read as one family.
 *  The parent container must be `position: relative` (the glow anchors to it). */
export const AgentHero: React.FC<{
  provider: AgentProvider;
  title: React.ReactNode;
  titleColor?: string;
  dimLogo?: boolean;
}> = ({ provider, title, titleColor, dimLogo }) => (
  <>
    <div
      aria-hidden
      style={{
        position: 'absolute',
        top: -64,
        left: 0,
        right: 0,
        height: 340,
        background:
          'radial-gradient(ellipse 420px 300px at 50% 20%, color-mix(in srgb, var(--wks-accent) 8%, transparent) 0%, transparent 70%)',
        pointerEvents: 'none',
      }}
    />
    <div
      style={{
        position: 'relative',
        width: 64,
        height: 64,
        margin: '0 auto',
        borderRadius: '50%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        border: '1px solid var(--wks-border-input)',
        background: 'color-mix(in srgb, var(--wks-accent) 5%, transparent)',
        color: 'var(--wks-text-primary)',
        opacity: dimLogo ? 0.6 : 1,
      }}
    >
      <AgentLogo provider={provider} size={30} />
    </div>
    <div
      style={{
        position: 'relative',
        marginTop: 16,
        fontSize: '1.05rem',
        fontWeight: 650,
        letterSpacing: '-0.01em',
        color: titleColor ?? colors.textBright,
      }}
    >
      {title}
    </div>
  </>
);

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
  provider?: AgentProvider;
  model?: string;
  permissionMode?: string;
  transport?: 'pty' | 'stream';
  cwd?: string;
  /** Composer pre-fill the pane was spawned with (e.g. a handoff takeover
   *  message). When present the starter chips hide — picking one would
   *  clobber the prepared prompt. */
  initialPrompt?: string;
  onPick: (prompt: string) => void;
}> = ({ agentName, provider, model, permissionMode, transport, cwd, initialPrompt, onPick }) => {
  const dirName = cwd ? (cwd.replace(/\/+$/, '').split('/').pop() ?? cwd) : undefined;

  // Git peek: branch + dirty count, best-effort. Not a repo / no git → omit.
  const [git, setGit] = useState<{ branch: string | null; dirty: number } | null>(null);
  useEffect(() => {
    if (!cwd) return;
    let live = true;
    // Optional + try/catch: absent on older preloads / web polyfill / test mocks.
    try {
      window.electronAPI
        .gitStatus?.(cwd)
        ?.then((s) => {
          if (live) setGit({ branch: s.branch, dirty: s.files.length });
        })
        .catch(() => {});
    } catch {
      // not a repo / no bridge — the meta line simply omits the git peek
    }
    return () => {
      live = false;
    };
  }, [cwd]);

  // Handoff takeover: App.tsx pre-fills the composer with a fixed in-house
  // message pointing at the brief on disk.
  const isHandoff = /handoff brief at /i.test(initialPrompt ?? '');

  const meta = [
    model,
    permissionMode,
    transport === 'stream' ? 'headless' : undefined,
    dirName,
    git?.branch ? `⎇ ${git.branch}` : undefined,
    git && git.dirty > 0 ? `${git.dirty} changed file${git.dirty === 1 ? '' : 's'}` : undefined,
  ].filter(Boolean) as string[];

  return (
    <div
      style={{
        position: 'relative',
        textAlign: 'center',
        marginTop: 48,
        color: colors.mutedDim,
        animation: 'claudeFadeIn 0.2s ease-out',
      }}
    >
      <AgentHero provider={provider ?? 'claude'} title={`${agentName} is ready`} />
      {meta.length > 0 && (
        <div
          style={{
            position: 'relative',
            fontSize: '0.7rem',
            marginTop: 8,
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

      {isHandoff ? (
        <div
          style={{
            margin: '20px auto 0',
            maxWidth: 420,
            padding: '10px 14px',
            borderRadius: 'var(--wks-radius-md)',
            backgroundColor: 'var(--wks-accent-bg)',
            border: `1px solid ${colors.borderSubtle}`,
            fontSize: '0.72rem',
            color: colors.text,
            lineHeight: 1.5,
          }}
        >
          <div style={{ fontWeight: 600, color: colors.textBright, marginBottom: 2 }}>
            Taking over from a handoff
          </div>
          The composer is pre-filled to read the brief — press Enter to pick up where the previous
          agent left off.
        </div>
      ) : initialPrompt ? (
        <div style={{ fontSize: '0.72rem', marginTop: 20, color: colors.muted }}>
          A prompt is prepared in the composer — press Enter to send it.
        </div>
      ) : (
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
      )}

      <div style={{ fontSize: '0.62rem', marginTop: 18, color: colors.mutedDim }}>
        Enter to send · Shift+Enter for a newline · + to attach files
      </div>
    </div>
  );
};
