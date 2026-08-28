import React, { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { AgentWorkspace, AgentProvider } from '../types/pane';
import { useConfig } from '../hooks/useConfig';
import { AgentLogo } from '../components/agentLogos';
import { ArrowRight } from '../components/icons';
import { ASK_PRESETS } from './askPresets';
import { useProviderDetection } from '../hooks/useProviderDetection';
import { visibleProviderOptions, NOT_INSTALLED_SUFFIX } from '../lib/providerAvailability';
import { SUPERVISOR_PROVIDERS } from '../lib/roleProviders';

export interface AskPaneProps {
  /** The current fleet — used to resolve session:<id> links and to scope. */
  agents: AgentWorkspace[];
  /** Spawn a triage-tier agent and send it the question (or, with no question,
   *  stage it to report on the fleet). Returns the new agent id. */
  spawnAskAgent: (opts: {
    question?: string;
    parentId?: string;
    provider?: AgentProvider;
  }) => Promise<string>;
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

// ── main pane ─────────────────────────────────────────────────────────────────

const AskPane: React.FC<AskPaneProps> = ({
  agents,
  spawnAskAgent,
  onJumpToAgent,
  scopeAgentId,
}) => {
  const prefix = useMemo(() => scopePrefix(scopeAgentId, agents), [scopeAgentId, agents]);
  const { config } = useConfig();

  const [question, setQuestion] = useState<string>(prefix);
  // The harness this launch runs on. It FOLLOWS Settings
  // (agents.managerProvider — the fleet harness) until you pick something here,
  // rather than snapshotting it at mount: config loads asynchronously, so a
  // `useState(config…)` initializer read 'claude' whenever this pane mounted
  // before the load landed (a pane restored at boot always did) and then never
  // caught up — Settings said codex and the launcher silently spawned claude.
  // `picked` is what makes an explicit choice stick.
  const configProvider: AgentProvider = config.agents?.managerProvider ?? 'claude';
  const [picked, setPicked] = useState<AgentProvider | null>(null);
  const provider = picked ?? configProvider;
  const setProvider = setPicked;
  // Offer only harnesses that are installed. The configured fleet harness and
  // the current pick stay listed even when missing — flagged, not hidden, so
  // "why can't I pick Codex any more" has an answer on screen.
  const { detection } = useProviderDetection();
  // One list with Settings (lib/roleProviders) — Pi ships no MCP client, so it
  // has no way to observe the fleet at all; this picker used to offer it while
  // the settings pane refused to.
  const visibleProviders = visibleProviderOptions(SUPERVISOR_PROVIDERS, detection, [
    provider,
    configProvider,
  ]);
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

  const submit = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      if (!trimmed || spawning) return;
      setSpawning(true);
      setError('');
      try {
        const newId = await spawnAskAgent({
          question: trimmed,
          parentId: scopeAgentId,
          provider,
        });
        setQuestion(prefix); // reset to prefix (or empty if no scope)
        onJumpToAgent(newId);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSpawning(false);
      }
    },
    [spawning, spawnAskAgent, scopeAgentId, onJumpToAgent, prefix, provider],
  );

  // Spawn a fleet agent directly — no question, it just starts its watch loop.
  const spawnDirect = useCallback(async () => {
    if (spawning) return;
    setSpawning(true);
    setError('');
    try {
      const newId = await spawnAskAgent({ parentId: scopeAgentId, provider });
      onJumpToAgent(newId);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSpawning(false);
    }
  }, [spawning, spawnAskAgent, scopeAgentId, provider, onJumpToAgent]);

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
          borderBottom: '1px solid var(--wks-border-subtle)',
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
            color: 'var(--wks-text-faint)',
            lineHeight: 1.5,
          }}
        >
          Dispatch an agent that inspects your fleet and answers.
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
                border: '1px solid var(--wks-accent)',
                background: 'transparent',
                color: 'var(--wks-accent)',
                fontSize: '0.7rem',
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
                el.style.background = 'var(--wks-accent)';
                el.style.color = 'var(--wks-text-on-accent)';
              }}
              onMouseLeave={(e) => {
                const el = e.currentTarget as HTMLButtonElement;
                el.style.background = 'transparent';
                el.style.color = 'var(--wks-accent)';
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
            borderRadius: 'var(--wks-radius-md)',
            border: '1px solid var(--wks-border-subtle)',
            background: 'var(--wks-bg-input)',
            color: 'var(--wks-text-primary)',
            fontSize: '0.82rem',
            fontFamily: 'inherit',
            lineHeight: 1.5,
            boxSizing: 'border-box',
            width: '100%',
            outline: 'none',
            opacity: spawning ? 0.6 : 1,
            transition: 'border-color 0.15s ease',
          }}
          onFocus={(e) => {
            (e.currentTarget as HTMLTextAreaElement).style.borderColor = 'var(--wks-accent)';
          }}
          onBlur={(e) => {
            (e.currentTarget as HTMLTextAreaElement).style.borderColor = 'var(--wks-border-subtle)';
          }}
        />

        {/* Which harness this launch runs on — follows Settings until picked. */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          {/* With a single installed harness there is nothing to pick. */}
          {visibleProviders.length > 1 && (
            <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-tertiary)' }}>Run on</span>
          )}
          {visibleProviders.length > 1 && (
            <div style={{ display: 'flex', gap: 4 }}>
              {visibleProviders.map((p) => {
                const active = provider === p.value;
                return (
                  <button
                    key={p.value}
                    onClick={() => setProvider(p.value)}
                    disabled={spawning}
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: 5,
                      padding: '4px 9px',
                      borderRadius: 6,
                      cursor: spawning ? 'default' : 'pointer',
                      fontSize: '0.7rem',
                      fontFamily: 'inherit',
                      fontWeight: 600,
                      border: active
                        ? '1px solid var(--wks-accent)'
                        : '1px solid var(--wks-border-input)',
                      background: active ? 'var(--wks-accent-bg)' : 'transparent',
                      color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                    }}
                  >
                    <AgentLogo
                      provider={p.value}
                      size={13}
                      style={{ flexShrink: 0, opacity: active ? 1 : 0.75 }}
                    />
                    {p.missing ? `${p.label}${NOT_INSTALLED_SUFFIX}` : p.label}
                  </button>
                );
              })}
            </div>
          )}
          {provider !== 'claude' && (
            <span style={{ fontSize: '0.64rem', color: 'var(--wks-text-faint)' }}>
              fleet tools via MCP facade · experimental
            </span>
          )}
        </div>

        {/* Error */}
        {error && (
          <div
            style={{
              fontSize: '0.72rem',
              color: 'var(--wks-error)',
              padding: '6px 10px',
              borderRadius: 6,
              border: '1px solid var(--wks-error)',
              background: 'color-mix(in srgb, var(--wks-error) 7%, transparent)',
            }}
          >
            {error}
          </div>
        )}

        {/* Actions: ask is primary; no-question watcher lives under Advanced. */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 10 }}>
          <details
            style={{
              position: 'relative',
              color: 'var(--wks-text-faint)',
              fontSize: '0.7rem',
              fontFamily: 'inherit',
            }}
          >
            <summary style={{ cursor: spawning ? 'default' : 'pointer', userSelect: 'none' }}>
              Advanced
            </summary>
            <button
              onClick={() => void spawnDirect()}
              disabled={spawning}
              title="Start an agent with no question"
              style={{
                position: 'absolute',
                right: 0,
                top: 22,
                zIndex: 3,
                whiteSpace: 'nowrap',
                padding: '7px 12px',
                borderRadius: 'var(--wks-radius-md)',
                border: '1px solid var(--wks-border-subtle)',
                background: 'var(--wks-bg-elevated)',
                color: spawning ? 'var(--wks-text-faint)' : 'var(--wks-text-secondary)',
                fontSize: '0.7rem',
                fontWeight: 600,
                fontFamily: 'inherit',
                cursor: spawning ? 'default' : 'pointer',
              }}
            >
              Start watcher only
            </button>
          </details>
          <button
            onClick={() => void submit(question)}
            disabled={!canSubmit}
            style={{
              padding: '8px 20px',
              borderRadius: 'var(--wks-radius-md)',
              border: 'none',
              background: canSubmit ? 'var(--wks-accent)' : 'var(--wks-border-subtle)',
              color: canSubmit ? 'var(--wks-text-on-accent)' : 'var(--wks-text-faint)',
              fontSize: '0.78rem',
              fontWeight: 700,
              fontFamily: 'inherit',
              cursor: canSubmit ? 'pointer' : 'default',
              transition: 'background 0.15s ease, color 0.15s ease',
              letterSpacing: '0.01em',
            }}
          >
            {spawning ? 'Dispatching…' : 'Ask'}
          </button>
        </div>
      </div>
    </div>
  );
};

export default AskPane;
