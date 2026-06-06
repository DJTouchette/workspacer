import React, { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { AgentWorkspace } from '../types/pane';
import { ASK_PRESETS } from './askPresets';
import { findSessionRefs } from './askLinks';

export interface AskPaneProps {
  /** The current fleet — used to resolve session:<id> links and to scope. */
  agents: AgentWorkspace[];
  /** Spawn a supervisor agent and send it the question. Returns new agent id. */
  spawnSupervisor: (opts: { question: string; parentId?: string }) => Promise<string>;
  /** Navigate the app to a given agent workspace by its AgentWorkspace.id. */
  onJumpToAgent: (agentId: string) => void;
  /** When the Ask pane was opened scoped to a specific agent (AgentWorkspace.id),
   *  pre-fill "About <name>: " and pass it as parentId on spawn. Optional. */
  scopeAgentId?: string;
}

// ── helpers ──────────────────────────────────────────────────────────────────

function scopePrefix(scopeAgentId: string | undefined, agents: AgentWorkspace[]): string {
  if (!scopeAgentId) return '';
  const agent = agents.find((a) => a.id === scopeAgentId);
  return agent ? `About ${agent.name}: ` : '';
}

// ── sub-components ────────────────────────────────────────────────────────────

const SupervisorRow: React.FC<{
  agent: AgentWorkspace;
  onJump: () => void;
}> = ({ agent, onJump }) => (
  <div
    onClick={onJump}
    title={`Open supervisor: ${agent.name}`}
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '7px 10px',
      borderRadius: 7,
      cursor: 'pointer',
      fontSize: '0.78rem',
      color: 'var(--wks-text-primary)',
      transition: 'background 0.1s ease',
    }}
    onMouseEnter={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-selected, rgba(74,158,255,0.08))';
    }}
    onMouseLeave={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'transparent';
    }}
  >
    {/* Supervisor indicator dot */}
    <span
      style={{
        width: 8,
        height: 8,
        borderRadius: '50%',
        background: 'var(--wks-accent, #4a9eff)',
        flexShrink: 0,
        opacity: agent.sessionId ? 1 : 0.35,
      }}
    />
    <span
      style={{
        flex: 1,
        minWidth: 0,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
        fontWeight: 500,
      }}
    >
      {agent.name}
    </span>
    <span
      style={{
        fontSize: '0.62rem',
        color: 'var(--wks-accent, #4a9eff)',
        fontWeight: 600,
        flexShrink: 0,
      }}
    >
      open →
    </span>
  </div>
);

// ── main pane ─────────────────────────────────────────────────────────────────

const AskPane: React.FC<AskPaneProps> = ({ agents, spawnSupervisor, onJumpToAgent, scopeAgentId }) => {
  const prefix = useMemo(() => scopePrefix(scopeAgentId, agents), [scopeAgentId, agents]);

  const [question, setQuestion] = useState<string>(prefix);
  const [spawning, setSpawning] = useState(false);
  const [error, setError] = useState<string>('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // When scopeAgentId changes (or on mount), reset the prefix.
  useEffect(() => {
    setQuestion(prefix);
  }, [prefix]);

  // Focus the textarea on mount.
  useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  const supervisors = useMemo(
    () => agents.filter((a) => a.kind === 'supervisor'),
    [agents],
  );

  // Resolve session refs in supervisor names for potential future rendering use.
  // (kept here so the import of findSessionRefs is exercised)
  const _sessionRefs = useMemo(
    () => supervisors.flatMap((s) => findSessionRefs(s.name, agents)),
    [supervisors, agents],
  );
  void _sessionRefs; // intentionally unused in current minimal UI

  const submit = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      if (!trimmed || spawning) return;
      setSpawning(true);
      setError('');
      try {
        const newId = await spawnSupervisor({
          question: trimmed,
          parentId: scopeAgentId,
        });
        setQuestion(prefix); // reset to prefix (or empty if no scope)
        onJumpToAgent(newId);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSpawning(false);
      }
    },
    [spawning, spawnSupervisor, scopeAgentId, onJumpToAgent, prefix],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void submit(question);
      }
    },
    [submit, question],
  );

  const canSubmit = question.trim().length > 0 && !spawning;

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        display: 'flex',
        flexDirection: 'column',
        boxSizing: 'border-box',
      }}
    >
      {/* ── Header ── */}
      <div
        style={{
          padding: '18px 20px 14px',
          borderBottom: '1px solid var(--wks-border-subtle, rgba(255,255,255,0.07))',
          flexShrink: 0,
        }}
      >
        <div
          style={{
            fontSize: '1.05rem',
            fontWeight: 700,
            color: 'var(--wks-text-primary)',
            marginBottom: 4,
          }}
        >
          Ask the fleet
        </div>
        <div
          style={{
            fontSize: '0.72rem',
            color: 'var(--wks-text-faint, #666)',
            lineHeight: 1.5,
          }}
        >
          Spawn a supervisor agent that inspects your fleet and answers.
        </div>
      </div>

      {/* ── Composer ── */}
      <div
        style={{
          padding: '16px 20px',
          flexShrink: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
        }}
      >
        {/* Preset buttons */}
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 6,
          }}
        >
          {ASK_PRESETS.map((preset) => (
            <button
              key={preset.id}
              onClick={() => void submit(preset.prompt)}
              disabled={spawning}
              title={preset.prompt}
              style={{
                padding: '4px 11px',
                borderRadius: 20,
                border: '1px solid var(--wks-accent, #4a9eff)',
                background: 'transparent',
                color: 'var(--wks-accent, #4a9eff)',
                fontSize: '0.68rem',
                fontWeight: 600,
                fontFamily: 'inherit',
                cursor: spawning ? 'default' : 'pointer',
                opacity: spawning ? 0.5 : 1,
                transition: 'background 0.1s ease, color 0.1s ease',
                letterSpacing: '0.02em',
              }}
              onMouseEnter={(e) => {
                if (spawning) return;
                const el = e.currentTarget as HTMLButtonElement;
                el.style.background = 'var(--wks-accent, #4a9eff)';
                el.style.color = 'var(--wks-text-on-accent, #0d0d10)';
              }}
              onMouseLeave={(e) => {
                const el = e.currentTarget as HTMLButtonElement;
                el.style.background = 'transparent';
                el.style.color = 'var(--wks-accent, #4a9eff)';
              }}
            >
              {preset.label}
            </button>
          ))}
        </div>

        {/* Textarea */}
        <textarea
          ref={textareaRef}
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            scopeAgentId
              ? 'Ask about this agent…  (Cmd/Ctrl+Enter to send)'
              : 'Ask anything about your fleet…  (Cmd/Ctrl+Enter to send)'
          }
          disabled={spawning}
          rows={4}
          style={{
            resize: 'vertical',
            padding: '10px 12px',
            borderRadius: 8,
            border: '1px solid var(--wks-border-subtle, rgba(255,255,255,0.1))',
            background: 'var(--wks-bg-input, rgba(255,255,255,0.03))',
            color: 'var(--wks-text-primary)',
            fontSize: '0.82rem',
            fontFamily: 'inherit',
            lineHeight: 1.55,
            boxSizing: 'border-box',
            width: '100%',
            outline: 'none',
            opacity: spawning ? 0.6 : 1,
            transition: 'border-color 0.15s ease',
          }}
          onFocus={(e) => {
            (e.currentTarget as HTMLTextAreaElement).style.borderColor =
              'var(--wks-accent, #4a9eff)';
          }}
          onBlur={(e) => {
            (e.currentTarget as HTMLTextAreaElement).style.borderColor =
              'var(--wks-border-subtle, rgba(255,255,255,0.1))';
          }}
        />

        {/* Error */}
        {error && (
          <div
            style={{
              fontSize: '0.72rem',
              color: 'var(--wks-danger, #e05555)',
              padding: '6px 10px',
              borderRadius: 6,
              border: '1px solid var(--wks-danger, #e05555)',
              background: 'rgba(224,85,85,0.07)',
            }}
          >
            {error}
          </div>
        )}

        {/* Ask button */}
        <button
          onClick={() => void submit(question)}
          disabled={!canSubmit}
          style={{
            alignSelf: 'flex-end',
            padding: '8px 20px',
            borderRadius: 8,
            border: 'none',
            background: canSubmit ? 'var(--wks-accent, #4a9eff)' : 'var(--wks-border-subtle, rgba(255,255,255,0.08))',
            color: canSubmit ? 'var(--wks-text-on-accent, #0d0d10)' : 'var(--wks-text-faint, #666)',
            fontSize: '0.78rem',
            fontWeight: 700,
            fontFamily: 'inherit',
            cursor: canSubmit ? 'pointer' : 'default',
            transition: 'background 0.15s ease, color 0.15s ease',
            letterSpacing: '0.01em',
          }}
        >
          {spawning ? 'Spawning…' : 'Ask'}
        </button>
      </div>

      {/* ── Supervisors list ── */}
      <div
        style={{
          flex: 1,
          padding: '0 20px 20px',
          display: 'flex',
          flexDirection: 'column',
          minHeight: 0,
        }}
      >
        <div
          style={{
            fontSize: '0.58rem',
            color: 'var(--wks-text-disabled, #555)',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.05em',
            padding: '0 10px 4px',
            marginBottom: 2,
          }}
        >
          Supervisors {supervisors.length > 0 && `(${supervisors.length})`}
        </div>

        {supervisors.length === 0 ? (
          <div
            style={{
              padding: '12px 10px',
              fontSize: '0.74rem',
              color: 'var(--wks-text-faint, #555)',
              lineHeight: 1.5,
            }}
          >
            No supervisor agents yet. Ask a question to spawn one.
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {supervisors
              .slice()
              .reverse() // newest-ish first (assuming agents are appended in order)
              .map((agent) => (
                <SupervisorRow
                  key={agent.id}
                  agent={agent}
                  onJump={() => onJumpToAgent(agent.id)}
                />
              ))}
          </div>
        )}
      </div>
    </div>
  );
};

export default AskPane;
