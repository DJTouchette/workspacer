import React, { useEffect, useMemo, useRef } from 'react';
import { BookOpen, Sparkles, Bot, Plug, Blocks, Wrench, Command, PieChart } from 'lucide-react';
import { useClaudeSession } from '../hooks/useClaudeSession';
import { claudeColors as colors } from '../components/claude-shared';
import { FileLink } from '../components/claude/FileLink';
import type { ContextItemInfo } from '../types/claudeSession';

interface ContextPaneProps {
  title: string;
  isActive: boolean;
  /** The claudemon session whose context inventory this pane itemizes. */
  contextSessionId?: string;
  /** The target agent's display name (shown in the header). */
  contextAgentName?: string;
  /** Section id to scroll into view on open (inspector chip deep-link). */
  contextFocus?: string;
}

/** Section ids — the deep-link targets the inspector's Usage chips use. */
const SECTIONS = ['memory', 'skills', 'agents', 'mcp', 'plugins', 'tools', 'commands'] as const;
type SectionId = (typeof SECTIONS)[number];

const SECTION_ICONS: Record<SectionId, React.ComponentType<{ size?: number | string }>> = {
  memory: BookOpen,
  skills: Sparkles,
  agents: Bot,
  mcp: Plug,
  plugins: Blocks,
  tools: Wrench,
  commands: Command,
};

const fmtTokens = (n?: number): string =>
  n === undefined ? '' : n >= 1000 ? `~${(n / 1000).toFixed(1)}k tok` : `~${n} tok`;

const fmtBytes = (n?: number): string =>
  n === undefined ? '' : n >= 1024 ? `${(n / 1024).toFixed(1)} KB` : `${n} B`;

/** Subtotal of the known token estimates in a list (undefined if none known). */
const subtotal = (items: ContextItemInfo[]): number | undefined => {
  const known = items.filter((i) => i.estTokens !== undefined);
  if (!known.length) return undefined;
  return known.reduce((sum, i) => sum + (i.estTokens ?? 0), 0);
};

/** Fixed-width mini meter: item share of the section's largest entry. Single
 *  hue (accent) since it encodes magnitude only; the unfilled track is a
 *  lighter step of the same hue so the column reads as one family. */
const ShareBar: React.FC<{ value: number; max: number }> = ({ value, max }) => (
  <span
    aria-hidden
    style={{
      width: 56,
      height: 4,
      flexShrink: 0,
      borderRadius: 2,
      background: 'color-mix(in srgb, var(--wks-accent) 12%, transparent)',
      overflow: 'hidden',
      display: 'inline-block',
    }}
  >
    <span
      style={{
        display: 'block',
        width: `${max > 0 ? Math.max(4, (value / max) * 100) : 0}%`,
        height: '100%',
        borderRadius: '0 2px 2px 0',
        background: 'color-mix(in srgb, var(--wks-accent) 60%, transparent)',
      }}
    />
  </span>
);

const ItemRow: React.FC<{ item: ContextItemInfo; cwd?: string; maxTokens: number }> = ({
  item,
  cwd,
  maxTokens,
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '5px 0',
      fontSize: '0.74rem',
    }}
  >
    {item.status && (
      <span
        aria-hidden
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          flexShrink: 0,
          background: item.status === 'connected' ? colors.success : colors.warning,
        }}
      />
    )}
    <span style={{ color: colors.text, minWidth: 0, overflowWrap: 'anywhere' }}>
      {item.path ? (
        <FileLink path={item.path} cwd={cwd} style={{ color: colors.text }}>
          {item.name}
        </FileLink>
      ) : (
        item.name
      )}
    </span>
    {item.status && (
      <span style={{ fontSize: '0.67rem', color: colors.muted, flexShrink: 0 }}>{item.status}</span>
    )}
    {item.source && (
      <span
        style={{
          fontSize: '0.66rem',
          color: colors.mutedDim,
          flexShrink: 0,
          border: `1px solid ${colors.borderSubtle}`,
          borderRadius: 'var(--wks-radius-pill, 99px)',
          padding: '0 6px',
          lineHeight: 1.6,
        }}
      >
        {item.source}
      </span>
    )}
    <span style={{ flex: 1 }} />
    {item.estTokens !== undefined && (
      <>
        <ShareBar value={item.estTokens} max={maxTokens} />
        <span
          title={fmtBytes(item.bytes)}
          style={{
            color: colors.muted,
            fontVariantNumeric: 'tabular-nums',
            whiteSpace: 'nowrap',
            width: 74,
            textAlign: 'right',
            flexShrink: 0,
            fontSize: '0.7rem',
          }}
        >
          {fmtTokens(item.estTokens)}
        </span>
      </>
    )}
  </div>
);

const Section: React.FC<{
  id: SectionId;
  title: string;
  note?: string;
  count: number;
  tokens?: number;
  focusRef?: React.RefObject<HTMLDivElement>;
  children: React.ReactNode;
}> = ({ id, title, note, count, tokens, focusRef, children }) => {
  if (count === 0) return null;
  const Icon = SECTION_ICONS[id];
  return (
    <div
      ref={focusRef}
      data-section={id}
      style={{
        marginBottom: 14,
        padding: '12px 16px 10px',
        borderRadius: 'var(--wks-radius-lg, 12px)',
        border: `1px solid ${colors.borderSubtle}`,
        background: 'var(--wks-bg-surface, rgba(255,255,255,0.02))',
        scrollMarginTop: 12,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
        <span
          aria-hidden
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 24,
            height: 24,
            borderRadius: 7,
            flexShrink: 0,
            color: colors.accent,
            background: 'color-mix(in srgb, var(--wks-accent) 9%, transparent)',
          }}
        >
          <Icon size={13} />
        </span>
        <span style={{ fontSize: '0.8rem', fontWeight: 650, color: colors.textBright }}>
          {title}
        </span>
        <span
          style={{
            fontSize: '0.66rem',
            color: colors.muted,
            background: 'rgba(255,255,255,0.06)',
            borderRadius: 'var(--wks-radius-pill, 99px)',
            padding: '1px 7px',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {count}
        </span>
        <span style={{ flex: 1 }} />
        {tokens !== undefined && (
          <span
            style={{
              fontSize: '0.7rem',
              color: colors.muted,
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {fmtTokens(tokens)}
          </span>
        )}
      </div>
      {note && (
        <div style={{ fontSize: '0.69rem', color: colors.mutedDim, margin: '0 0 6px 32px' }}>
          {note}
        </div>
      )}
      <div style={{ marginLeft: 32 }}>{children}</div>
    </div>
  );
};

/** Rows in a section, with per-row dividers and the section's max token count
 *  precomputed so every ShareBar shares one scale. */
const ItemList: React.FC<{ items: ContextItemInfo[]; cwd?: string }> = ({ items, cwd }) => {
  const maxTokens = items.reduce((m, i) => Math.max(m, i.estTokens ?? 0), 0);
  return (
    <>
      {items.map((i, idx) => (
        <div
          key={`${i.source ?? ''}/${i.path ?? i.name}`}
          style={idx > 0 ? { borderTop: `1px solid ${colors.borderSubtle}` } : undefined}
        >
          <ItemRow item={i} cwd={cwd} maxTokens={maxTokens} />
        </div>
      ))}
    </>
  );
};

/**
 * Itemizes what occupies one session's context window: the memory files,
 * skills, agents, MCP servers, plugins, builtin tools, and slash commands the
 * stream `init` frame reported loaded, with best-effort token estimates from
 * the files behind them. Opened from the inspector rail's Usage chips or the
 * command palette.
 *
 * The per-item numbers are estimates (file bytes / 4) — Claude doesn't expose
 * its real per-item accounting; only the overall context bar is authoritative.
 */
const ContextPane: React.FC<ContextPaneProps> = ({
  isActive,
  contextSessionId,
  contextAgentName,
  contextFocus,
}) => {
  const { session } = useClaudeSession({
    ptySessionId: contextSessionId ?? null,
    active: isActive,
  });
  const sl = session?.statusLine;
  const inv = sl?.capabilities?.inventory;
  const cwd = session?.cwd;

  // Deep-link: scroll the requested section into view once data is up.
  const sectionRefs = useRef<Partial<Record<SectionId, React.RefObject<HTMLDivElement>>>>({});
  for (const s of SECTIONS) sectionRefs.current[s] ??= React.createRef<HTMLDivElement>();
  useEffect(() => {
    if (!contextFocus || !inv) return;
    const ref = sectionRefs.current[contextFocus as SectionId];
    ref?.current?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }, [contextFocus, inv]);

  const usedTokens = useMemo(() => {
    if (sl?.contextUsedPct === undefined || !sl.contextWindowSize) return undefined;
    return Math.round((sl.contextUsedPct / 100) * sl.contextWindowSize);
  }, [sl?.contextUsedPct, sl?.contextWindowSize]);

  const estimatedTotal = useMemo(() => {
    if (!inv) return undefined;
    return subtotal([...inv.memoryFiles, ...inv.skills, ...inv.agents, ...inv.plugins]);
  }, [inv]);

  if (!contextSessionId) {
    return (
      <div style={emptyStyle}>
        This context pane lost its target — close it and reopen from the inspector.
      </div>
    );
  }

  const ctxPct = sl?.contextUsedPct;
  const barColor =
    ctxPct === undefined
      ? colors.muted
      : ctxPct >= 90
        ? colors.error
        : ctxPct >= 70
          ? colors.warning
          : colors.success;

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        overflowY: 'auto',
        background: 'var(--wks-bg-base)',
        padding: '22px 22px 40px',
        boxSizing: 'border-box',
        position: 'relative',
      }}
    >
      {/* Soft accent glow behind the header — pure decoration, brand family. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: -80,
          left: 0,
          right: 0,
          height: 280,
          background:
            'radial-gradient(ellipse 460px 240px at 50% 0%, color-mix(in srgb, var(--wks-accent) 7%, transparent) 0%, transparent 70%)',
          pointerEvents: 'none',
        }}
      />
      <div style={{ maxWidth: 640, margin: '0 auto', position: 'relative' }}>
        {/* Header: who + the authoritative context meter */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 2 }}>
          <span
            aria-hidden
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              width: 30,
              height: 30,
              borderRadius: '50%',
              flexShrink: 0,
              color: colors.accent,
              border: `1px solid ${colors.borderSubtle}`,
              background: 'color-mix(in srgb, var(--wks-accent) 7%, transparent)',
            }}
          >
            <PieChart size={15} />
          </span>
          <span style={{ fontSize: '0.95rem', fontWeight: 700, color: colors.textBright }}>
            Context window
          </span>
          {contextAgentName && (
            <span style={{ fontSize: '0.75rem', color: colors.muted }}>{contextAgentName}</span>
          )}
          <span style={{ flex: 1 }} />
          {sl?.modelDisplay && (
            <span
              style={{
                fontSize: '0.7rem',
                color: colors.muted,
                border: `1px solid ${colors.borderSubtle}`,
                borderRadius: 'var(--wks-radius-pill, 99px)',
                padding: '2px 9px',
              }}
            >
              {sl.modelDisplay}
            </span>
          )}
        </div>
        {inv?.claudeCodeVersion && (
          <div style={{ fontSize: '0.69rem', color: colors.mutedDim, margin: '4px 0 0 40px' }}>
            Claude Code v{inv.claudeCodeVersion}
          </div>
        )}

        {ctxPct !== undefined && (
          <div style={{ margin: '16px 0 6px' }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                justifyContent: 'space-between',
                fontSize: '0.74rem',
                color: colors.muted,
                marginBottom: 6,
              }}
            >
              <span>
                {usedTokens !== undefined && sl?.contextWindowSize
                  ? `${(usedTokens / 1000).toFixed(1)}k of ${(sl.contextWindowSize / 1000).toFixed(0)}k tokens`
                  : 'used'}
              </span>
              {/* Value in text ink; the dot + fill carry the severity color. */}
              <span
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 6,
                  color: colors.textBright,
                  fontWeight: 600,
                }}
              >
                <span
                  aria-hidden
                  style={{ width: 7, height: 7, borderRadius: '50%', background: barColor }}
                />
                {Math.round(ctxPct)}%
              </span>
            </div>
            {/* Meter: severity fill on a lighter step of the same hue. */}
            <div
              style={{
                height: 10,
                borderRadius: 5,
                background: `color-mix(in srgb, ${barColor} 14%, transparent)`,
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  width: `${Math.min(100, ctxPct)}%`,
                  height: '100%',
                  borderRadius: '5px 4px 4px 5px',
                  backgroundColor: barColor,
                  transition: 'width 0.4s ease, background-color 0.4s ease',
                }}
              />
            </div>
          </div>
        )}

        <div style={{ fontSize: '0.69rem', color: colors.mutedDim, margin: '10px 0 20px' }}>
          Per-item sizes are estimates from the files on disk (~4 chars per token). Claude does not
          report its exact per-item accounting; the bar above is the authoritative total.
          {estimatedTotal !== undefined &&
            ` File-backed items below: ${fmtTokens(estimatedTotal)}.`}
        </div>

        {!inv ? (
          <div style={{ ...emptyStyle, height: 'auto', padding: '30px 0' }}>
            {sl?.capabilities
              ? 'This session reported counts but no itemized inventory.'
              : 'No inventory yet — the itemized breakdown arrives with the first turn of a headless (stream-transport) Claude session. PTY and non-Claude sessions don’t report one.'}
          </div>
        ) : (
          <>
            <Section
              id="memory"
              title="Memories"
              count={inv.memoryFiles.length}
              tokens={subtotal(inv.memoryFiles)}
              focusRef={sectionRefs.current.memory}
            >
              <ItemList items={inv.memoryFiles} cwd={cwd} />
            </Section>

            <Section
              id="skills"
              title="Skills"
              note="Only a skill's name and description load up front; the body loads when invoked."
              count={inv.skills.length}
              tokens={subtotal(inv.skills)}
              focusRef={sectionRefs.current.skills}
            >
              <ItemList items={inv.skills} cwd={cwd} />
            </Section>

            <Section
              id="agents"
              title="Agents"
              note="Unsized entries are builtin agent types with no file behind them."
              count={inv.agents.length}
              tokens={subtotal(inv.agents)}
              focusRef={sectionRefs.current.agents}
            >
              <ItemList items={inv.agents} cwd={cwd} />
            </Section>

            <Section
              id="mcp"
              title="MCP servers"
              note="Each connected server's tool definitions occupy context; their size isn't reported."
              count={inv.mcpServers.length}
              focusRef={sectionRefs.current.mcp}
            >
              <ItemList items={inv.mcpServers} cwd={cwd} />
            </Section>

            <Section
              id="plugins"
              title="Plugins"
              count={inv.plugins.length}
              tokens={subtotal(inv.plugins)}
              focusRef={sectionRefs.current.plugins}
            >
              <ItemList items={inv.plugins} cwd={cwd} />
            </Section>

            <Section
              id="tools"
              title="Builtin tools"
              count={inv.tools.length}
              focusRef={sectionRefs.current.tools}
            >
              <NameCloud names={inv.tools} />
            </Section>

            <Section
              id="commands"
              title="Slash commands"
              count={inv.slashCommands.length}
              focusRef={sectionRefs.current.commands}
            >
              <NameCloud names={inv.slashCommands.map((c) => `/${c}`)} />
            </Section>
          </>
        )}
      </div>
    </div>
  );
};

/** Compact chip cloud for plain name lists (tools, slash commands). */
const NameCloud: React.FC<{ names: string[] }> = ({ names }) => (
  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 5, paddingTop: 2 }}>
    {names.map((n) => (
      <span
        key={n}
        style={{
          fontSize: '0.69rem',
          color: colors.muted,
          background: 'rgba(255,255,255,0.04)',
          border: `1px solid ${colors.borderSubtle}`,
          borderRadius: 'var(--wks-radius-pill, 99px)',
          padding: '2px 8px',
          whiteSpace: 'nowrap',
        }}
      >
        {n}
      </span>
    ))}
  </div>
);

const emptyStyle: React.CSSProperties = {
  height: '100%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: 24,
  textAlign: 'center',
  color: colors.mutedDim,
  fontSize: '0.75rem',
  background: 'var(--wks-bg-base)',
};

export default ContextPane;
