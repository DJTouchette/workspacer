import { useInlineChatState } from './ChatUiScope';
/**
 * The inline response card: a `wks-html-card` fence rendered as a bounded,
 * sandboxed frame with its declared actions in TRUSTED chrome beside it.
 *
 * Shape of the thing, top to bottom:
 *
 *   ┌─ header ─────────────────────── trusted app DOM (title, notes) ─┐
 *   │  <iframe sandbox="allow-scripts" srcDoc=…>  ← untrusted, opaque │
 *   └─ actions ───────────────────── trusted app DOM (real buttons) ──┘
 *
 * The action buttons are deliberately NOT inside the frame. A button the model
 * drew would need a postMessage channel to reach the host, and postMessage
 * carries no user-activation across frames — the host could never tell a real
 * click from a script that fired on load. Rendering them out here makes that
 * question moot: the only thing that crosses the boundary in this feature is a
 * bounded height number, in one direction.
 *
 * Gating is by CONTEXT, in two independent parts, and both default to "no":
 *  - `HtmlCardAllowedContext` — only an ASSISTANT text bubble turns this on, so
 *    a user message, a tool result, a library preview or a command's output
 *    that happens to contain the fence renders as an ordinary code block.
 *  - `HtmlCardHostContext` — the owning chat surface's live session/pane/cwd.
 *    Without it the original fence remains readable and no frame mounts.
 *
 * Reading the gate inside the component, rather than parameterising
 * `parseMarkdownBlocks`, is what keeps markdown.tsx's module-level parse cache
 * valid — the same reason `MarkdownFileCwd` is a context there.
 */
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { claudeColors as colors } from '../claude-shared';
import {
  parseHtmlCard,
  type HtmlCardAction,
  type HtmlCardEnvelope,
} from '../../../../main/shared/htmlCard';
import {
  buildCardDocument,
  cardNonce,
  hostThemeVars,
  CARD_SANDBOX,
} from '../../lib/htmlCard/cardShell';
import {
  CARD_HEIGHT_MESSAGE,
  CARD_MAX_HEIGHT,
  CARD_MIN_HEIGHT,
} from '../../lib/htmlCard/cardRuntime';
import {
  actionAvailable,
  performCardAction,
  type HtmlCardHost,
} from '../../lib/htmlCard/cardActions';
import { DiffView } from './DiffView';
import type { HtmlCardDiffResult } from '../../../../main/shared/htmlCardDiff';
import { Surface } from '../Surface';
import { LayoutPanelTop, FileDiff, MessageSquarePlus, UserRound } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

const HtmlCardAllowedContext = createContext(false);
/** Turned on around an assistant bubble's rendered markdown, nowhere else. */
export const HtmlCardAllowedProvider = HtmlCardAllowedContext.Provider;

export const HtmlCardHostContext = createContext<HtmlCardHost | null>(null);
/** Provided by the owning chat pane with its OWN live session/pane/cwd. */
export const HtmlCardHostProvider = HtmlCardHostContext.Provider;

const ACTION_ICON: Record<HtmlCardAction['kind'], LucideIcon> = {
  open_worker: UserRound,
  view_diff: FileDiff,
  fill_composer: MessageSquarePlus,
};

/** The verb the button's tooltip promises, so nothing a card labels can imply
 *  more than the host will actually do. A prefill in particular must read as a
 *  prefill: the label is the model's, this sentence is ours. */
const ACTION_EFFECT: Record<HtmlCardAction['kind'], string> = {
  open_worker: 'Opens that agent in a viewer pane.',
  view_diff: 'Shows an exact HEAD-to-working-file snapshot inside this project.',
  fill_composer: 'Puts this text in the composer for you to read and send. Nothing is sent.',
};

/** One trusted action button. */
const CardActionButton: React.FC<{ action: HtmlCardAction; host: HtmlCardHost }> = ({
  action,
  host,
}) => {
  const [busy, setBusy] = useState(false);
  const [diff, setDiff] = useInlineChatState<Extract<HtmlCardDiffResult, { ok: true }> | null>(
    'html-diff',
    action,
    null,
  );
  const Icon = ACTION_ICON[action.kind];
  const run = useCallback(() => {
    setBusy(true);
    void performCardAction(action, host, setDiff).finally(() => setBusy(false));
  }, [action, host]);
  return (
    <>
      <button
        type="button"
        onClick={run}
        disabled={busy}
        title={ACTION_EFFECT[action.kind]}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 8px',
          borderRadius: 'var(--wks-radius-sm)',
          border: `1px solid ${colors.border}`,
          background: 'transparent',
          color: colors.text,
          cursor: busy ? 'progress' : 'pointer',
          fontSize: 'calc(0.72rem * var(--claude-gui-font-scale, 1))',
        }}
      >
        <Icon size={12} />
        {action.kind === 'fill_composer'
          ? 'Prefill: '
          : action.kind === 'view_diff'
            ? 'View diff: '
            : 'Open worker: '}
        {action.label}
      </button>
      {diff && (
        <div
          role="region"
          aria-label={`Diff: ${diff.path}`}
          style={{ width: '100%', maxHeight: 560, overflow: 'auto' }}
        >
          <button type="button" onClick={() => setDiff(null)}>
            Close diff
          </button>
          <p>{diff.path} — HEAD to working file snapshot</p>
          <DiffView oldStr={diff.before} newStr={diff.after} filePath={diff.path} />
        </div>
      )}
    </>
  );
};

/** The frame. Isolated in its own component so the `srcDoc` is rebuilt only
 *  when the card or the theme actually changes — a rebuild reloads the document
 *  and would throw away the user's filter/sort/disclosure state. */
const CardFrame: React.FC<{ card: HtmlCardEnvelope; onRemoved: (r: string[]) => void }> = ({
  card,
  onRemoved,
}) => {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const [height, setHeight] = useState(CARD_MIN_HEIGHT);
  // Re-read the applied theme tokens whenever the app writes new ones. The
  // theme lands on documentElement's inline style (themes.applyTheme), so one
  // attribute observer covers every theme switch, including a custom theme.
  const [themeTick, setThemeTick] = useState(0);
  useEffect(() => {
    const observer = new MutationObserver(() => setThemeTick((n) => n + 1));
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['style'],
    });
    return () => observer.disconnect();
  }, []);

  const doc = useMemo(() => {
    const vars = hostThemeVars();
    const light = document.documentElement.style.colorScheme === 'light';
    return buildCardDocument(card, { nonce: cardNonce(), themeVars: vars, light });
    // themeTick is the dependency that matters here; `card` is the content.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [card, themeTick]);

  useEffect(() => onRemoved(doc.removed), [doc.removed, onRemoved]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      // event.source identity, NOT event.origin: every sandboxed srcDoc frame
      // reports its origin as the literal string "null", so origin cannot tell
      // two cards in one transcript apart. A frame only ever holds a reference
      // to its own contentWindow, so this comparison is the routing.
      if (!frameRef.current || event.source !== frameRef.current.contentWindow) return;
      const data = event.data as { type?: unknown; height?: unknown } | null;
      if (!data || data.type !== CARD_HEIGHT_MESSAGE) return;
      const raw = data.height;
      if (typeof raw !== 'number') return;
      if (!Number.isFinite(raw)) return;
      // Clamped, never trusted: a card that asks for 200000px gets the ceiling
      // and scrolls inside itself.
      setHeight(Math.min(CARD_MAX_HEIGHT, Math.max(CARD_MIN_HEIGHT, Math.ceil(raw))));
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, []);

  return (
    <iframe
      ref={frameRef}
      data-testid="wks-html-card-frame"
      // The frame is described by the card's own title, so a screen reader
      // announces it as a named embedded region rather than "frame".
      title={`Response card: ${card.title}`}
      sandbox={CARD_SANDBOX}
      srcDoc={doc.srcDoc}
      scrolling="auto"
      style={{
        display: 'block',
        width: '100%',
        height,
        border: 'none',
        background: 'transparent',
        colorScheme: 'normal',
      }}
    />
  );
};

/** A card that could not render, shown as the author's own fallback prose plus
 *  the raw block on demand. Degradation is the feature: the bytes that arrived,
 *  or the reason none did — never an empty space and never a thrown render. */
const CardRefusal: React.FC<{ reason: string; fallback?: string; raw: React.ReactNode }> = ({
  reason,
  fallback,
  raw,
}) => (
  <div
    style={{
      margin: '8px 0',
      border: `1px dashed ${colors.border}`,
      borderRadius: 'var(--wks-radius-md)',
      padding: '8px 10px',
    }}
  >
    {fallback && (
      <div style={{ whiteSpace: 'pre-wrap', lineHeight: 1.6, marginBottom: 6 }}>{fallback}</div>
    )}
    <details>
      <summary
        style={{
          cursor: 'pointer',
          color: colors.muted,
          fontSize: 'calc(0.72rem * var(--claude-gui-font-scale, 1))',
        }}
      >
        Card not shown — {reason}
      </summary>
      <div style={{ marginTop: 4 }}>{raw}</div>
    </details>
  </div>
);

/** The rendered card, chrome and all. */
export const HtmlResponseCard: React.FC<{ card: HtmlCardEnvelope; host: HtmlCardHost | null }> = ({
  card,
  host,
}) => {
  const [removed, setRemoved] = useState<string[]>([]);
  const onRemoved = useCallback((r: string[]) => setRemoved(r), []);
  if (!host?.sessionId) return <p style={{ whiteSpace: 'pre-wrap' }}>{card.fallback}</p>;
  const offered = host ? card.actions.filter((a) => actionAvailable(a, host)) : [];
  return (
    <Surface
      role="region"
      elevation="flat"
      data-testid="wks-html-card"
      aria-label={`Response card: ${card.title}`}
      style={{
        margin: '8px 0',
        borderRadius: 'var(--wks-radius-md)',
        overflow: 'hidden',
        overflowWrap: 'anywhere',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 10px',
          borderBottom: `1px solid ${colors.borderSubtle}`,
          color: colors.muted,
          fontSize: 'calc(0.66rem * var(--claude-gui-font-scale, 1))',
        }}
      >
        <LayoutPanelTop size={12} />
        <span style={{ color: colors.textBright, fontWeight: 600 }}>{card.title}</span>
      </header>
      <CardFrame card={card} onRemoved={onRemoved} />
      <details style={{ padding: '6px 10px' }}>
        <summary style={{ cursor: 'pointer' }}>Text alternative</summary>
        <p style={{ whiteSpace: 'pre-wrap' }}>{card.fallback}</p>
      </details>
      {(offered.length > 0 || removed.length > 0) && (
        <footer
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            alignItems: 'center',
            gap: 6,
            padding: '6px 10px',
            borderTop: `1px solid ${colors.borderSubtle}`,
          }}
        >
          {offered.map((action, index) => (
            <CardActionButton key={index} action={action} host={host!} />
          ))}
          {removed.length > 0 && (
            <span
              title={removed.join(', ')}
              style={{
                color: colors.mutedDim,
                fontSize: 'calc(0.66rem * var(--claude-gui-font-scale, 1))',
              }}
            >
              {removed.length} element{removed.length === 1 ? '' : 's'} removed
            </span>
          )}
        </footer>
      )}
    </Surface>
  );
};

/**
 * What `parseMarkdownBlocks` emits for a `wks-html-card` fence.
 *
 * `raw` is the fence body and `code` is the ordinary CodeBlock markdown.tsx
 * would have rendered — passed IN rather than imported, so this module and
 * markdown.tsx do not import each other.
 *
 * An UNCLOSED fence renders `code` verbatim. That is the whole streaming story:
 * a card is never mounted from a partial document, because a half-parsed
 * fragment in a live sandbox would flash a broken card on every token, and
 * "never partially execute" is the contract.
 */
export const HtmlCardFence: React.FC<{
  raw: string;
  closed: boolean;
  code: React.ReactNode;
}> = ({ raw, closed, code }) => {
  const allowed = useContext(HtmlCardAllowedContext);
  const host = useContext(HtmlCardHostContext);
  const parsed = useMemo(
    () => (closed && allowed ? parseHtmlCard(raw) : null),
    [raw, closed, allowed],
  );
  if (!parsed || !host?.sessionId) return <>{code}</>;
  if (!parsed.ok)
    return <CardRefusal reason={parsed.reason} fallback={parsed.fallback} raw={code} />;
  return <HtmlResponseCard card={parsed.card} host={host} />;
};
