import React, { useCallback, useMemo, useRef, useState } from 'react';
import { AgentWorkspace } from '../types/pane';
import { BrandMark } from '../components/Brand';
import { ArrowUp } from 'lucide-react';
import { ArrowRight } from '../components/icons';
import { GUIDE_AGENT_NAME, GUIDE_PRESETS } from '../lib/guide';

/**
 * The Workspacer Guide pane — a chat-styled front door for "how do I use this
 * app?" questions. The opening bubbles are scripted (free); the moment the user
 * picks a preset or submits a question, a REAL Claude agent spawns with the
 * workspacer MCP facade (triage tier: observe + UI navigation) and the view
 * jumps into its live conversation. Repeat questions reuse the running guide
 * instead of spawning another.
 */

export interface GuidePaneProps {
  /** The current fleet — used to find an already-running guide agent. */
  agents: AgentWorkspace[];
  /** Spawn the guide agent and auto-send it the question. Returns agent id. */
  spawnGuide: (question: string) => Promise<string>;
  /** Navigate the app to a given agent workspace by its AgentWorkspace.id. */
  onJumpToAgent: (agentId: string) => void;
}

interface Bubble {
  role: 'guide' | 'user' | 'status';
  text: string;
}

const OPENING_BUBBLES: Bubble[] = [
  {
    role: 'guide',
    text:
      "Hi! I'm the Workspacer guide — a live agent with tools that can see this app " +
      'and open things for you. Ask me how anything works, or pick a question below ' +
      'and I can show you around.',
  },
];

const GuideBubble: React.FC<{ bubble: Bubble }> = ({ bubble }) => {
  if (bubble.role === 'status') {
    return (
      <div
        style={{
          alignSelf: 'center',
          fontSize: '0.7rem',
          color: 'var(--wks-text-faint)',
          padding: '2px 0',
        }}
      >
        {bubble.text}
      </div>
    );
  }
  const isUser = bubble.role === 'user';
  return (
    <div
      style={{
        display: 'flex',
        gap: 8,
        alignItems: 'flex-end',
        alignSelf: isUser ? 'flex-end' : 'flex-start',
        maxWidth: '85%',
      }}
    >
      {!isUser && (
        <span style={{ flexShrink: 0, marginBottom: 2 }}>
          <BrandMark size={18} />
        </span>
      )}
      <div
        style={{
          padding: '9px 12px',
          borderRadius: 'var(--wks-radius-lg)',
          fontSize: '0.8rem',
          lineHeight: 1.5,
          background: isUser ? 'var(--wks-accent)' : 'var(--wks-bg-surface)',
          color: isUser ? 'var(--wks-text-on-accent)' : 'var(--wks-text-primary)',
        }}
      >
        {bubble.text}
      </div>
    </div>
  );
};

const GuidePane: React.FC<GuidePaneProps> = ({ agents, spawnGuide, onJumpToAgent }) => {
  const [bubbles, setBubbles] = useState<Bubble[]>(OPENING_BUBBLES);
  const [question, setQuestion] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  // An already-running guide: repeat questions go straight to it.
  const runningGuide = useMemo(
    () => agents.find((a) => a.name === GUIDE_AGENT_NAME && a.sessionId),
    [agents],
  );

  const submit = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      if (!trimmed || busy) return;
      setBusy(true);
      setError('');
      setBubbles((prev) => [
        ...prev,
        { role: 'user', text: trimmed },
        {
          role: 'status',
          text: runningGuide ? 'Sending to your guide…' : 'Starting your guide agent…',
        },
      ]);
      try {
        if (runningGuide?.sessionId) {
          await window.electronAPI.claudeMessage(runningGuide.sessionId, trimmed);
          onJumpToAgent(runningGuide.id);
        } else {
          const newId = await spawnGuide(trimmed);
          onJumpToAgent(newId);
        }
        setQuestion('');
        // Trim the status line; the real conversation continues in the agent pane.
        setBubbles((prev) => prev.filter((b) => b.role !== 'status'));
      } catch (err) {
        setBubbles((prev) => prev.filter((b) => b.role !== 'status'));
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setBusy(false);
      }
    },
    [busy, runningGuide, spawnGuide, onJumpToAgent],
  );

  const canSend = question.trim().length > 0 && !busy;

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        boxSizing: 'border-box',
      }}
    >
      <div
        style={{
          maxWidth: 'min(640px, 100%)',
          margin: '0 auto',
          minHeight: '100%',
          display: 'flex',
          flexDirection: 'column',
          padding: '24px 20px 20px',
          boxSizing: 'border-box',
          gap: 16,
        }}
      >
        {/* Header */}
        <div>
          <div style={{ fontSize: '1.05rem', fontWeight: 700, marginBottom: 4 }}>
            Workspacer Guide
          </div>
          <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
            Ask how anything works — a live agent answers, and can open the app&rsquo;s surfaces to
            show you.
          </div>
        </div>

        {/* Bubbles */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, flex: 1 }}>
          {bubbles.map((b, i) => (
            <GuideBubble key={i} bubble={b} />
          ))}
        </div>

        {/* Preset chips */}
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
          {GUIDE_PRESETS.map((preset) => (
            <button
              key={preset.id}
              onClick={() => void submit(preset.prompt)}
              disabled={busy}
              title={preset.prompt}
              style={{
                padding: '4px 11px',
                borderRadius: 'var(--wks-radius-pill)',
                border: '1px solid var(--wks-accent)',
                background: 'transparent',
                color: 'var(--wks-accent)',
                fontSize: '0.7rem',
                fontWeight: 600,
                fontFamily: 'inherit',
                cursor: busy ? 'default' : 'pointer',
                opacity: busy ? 0.5 : 1,
                transition: 'background 0.1s ease, color 0.1s ease',
              }}
              onMouseEnter={(e) => {
                if (busy) return;
                const el = e.currentTarget;
                el.style.background = 'var(--wks-accent)';
                el.style.color = 'var(--wks-text-on-accent)';
              }}
              onMouseLeave={(e) => {
                const el = e.currentTarget;
                el.style.background = 'transparent';
                el.style.color = 'var(--wks-accent)';
              }}
            >
              {preset.label}
            </button>
          ))}
        </div>

        {/* Usage note — the honest fine print. */}
        <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
          The guide is a real Claude agent on your account — answers consume usage like any other
          session. Nothing runs until you ask.
        </div>

        {error && (
          <div
            style={{
              fontSize: '0.72rem',
              color: 'var(--wks-error)',
              padding: '6px 10px',
              borderRadius: 'var(--wks-radius-sm)',
              border: '1px solid var(--wks-error)',
              background: 'color-mix(in srgb, var(--wks-error) 7%, transparent)',
            }}
          >
            {error}
          </div>
        )}

        {/* Composer */}
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <input
            ref={inputRef}
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void submit(question);
              }
            }}
            placeholder="Ask anything about Workspacer…"
            disabled={busy}
            style={{
              flex: 1,
              padding: '9px 12px',
              borderRadius: 'var(--wks-radius-md)',
              border: '1px solid var(--wks-border-input)',
              background: 'var(--wks-bg-input)',
              color: 'var(--wks-text-primary)',
              fontSize: '0.8rem',
              fontFamily: 'inherit',
              outline: 'none',
              opacity: busy ? 0.6 : 1,
            }}
            onFocus={(e) => {
              e.currentTarget.style.borderColor = 'var(--wks-accent)';
            }}
            onBlur={(e) => {
              e.currentTarget.style.borderColor = 'var(--wks-border-input)';
            }}
          />
          <button
            onClick={() => void submit(question)}
            disabled={!canSend}
            title="Ask the guide"
            aria-label="Ask the guide"
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              width: 34,
              height: 34,
              borderRadius: 'var(--wks-radius-md)',
              border: 'none',
              background: canSend ? 'var(--wks-accent)' : 'var(--wks-border-subtle)',
              color: canSend ? 'var(--wks-text-on-accent)' : 'var(--wks-text-faint)',
              cursor: canSend ? 'pointer' : 'default',
              transition: 'background 0.15s ease',
            }}
          >
            <ArrowUp size={14} strokeWidth={2} />
          </button>
        </div>

        {/* Running-guide shortcut */}
        {runningGuide && (
          <button
            onClick={() => onJumpToAgent(runningGuide.id)}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 5,
              alignSelf: 'flex-start',
              padding: 0,
              border: 'none',
              background: 'none',
              fontFamily: 'inherit',
              fontSize: '0.7rem',
              fontWeight: 600,
              color: 'var(--wks-accent)',
              cursor: 'pointer',
            }}
          >
            Your guide is running — open its chat <ArrowRight size={11} strokeWidth={2.25} />
          </button>
        )}
      </div>
    </div>
  );
};

export default GuidePane;
