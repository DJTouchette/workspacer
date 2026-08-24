/**
 * A worker's structured result, rendered as a card instead of a JSON dump.
 *
 * When a dispatch carries a `resultSchema` (spawn_agent), the worker ends its
 * turn with a fenced `wks-result` block; supervisorNudge validates it and the
 * finish wake carries the object (main/shared/fleetMessages `result`). This is
 * where a manager reads it — inside the fleet card's worker row.
 *
 * ## Two layers, because the schema is arbitrary
 *
 * The keys are authored per dispatch, so this component renders by value
 * SHAPE, not by key (see structuredResultFields): booleans and numbers become
 * chips in a scannable top strip, arrays of paths become FileLinks, long
 * arrays collapse, long strings clamp, nested objects become key/value rows.
 * A field nobody anticipated therefore still renders, labelled from its key —
 * dropping it would be worse than the raw JSON this replaces.
 *
 * On top of that, the conventional keys get the treatment they earn:
 * `merged` a yes/no badge (the first thing anyone asks), `commit` the mono SHA
 * the Review pane's history rows use, `filesChanged` a count with the list on
 * demand, and `caveats` its own band that is NEVER behind a fold — a caveat
 * nobody reads is the whole reason this card is worth building.
 *
 * Degradation is a feature: a result too large for the wake arrives truncated
 * (and so unparseable), and a worker can botch the contract entirely. Both
 * render — the bytes that did arrive, or the reason none did — never an empty
 * card and never a thrown render.
 */
import React, { useState } from 'react';
import { AlertTriangle, Check, ChevronRight, ClipboardList, X } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import { Surface } from '../Surface';
import { CopyTextButton } from './CopyTextButton';
import { FileLink } from './FileLink';
import {
  ARRAY_COLLAPSE_MIN,
  ARRAY_PREVIEW,
  CAVEATS_CLAMP,
  TEXT_CLAMP,
  buildResultView,
  emptyLabel,
  fieldsInSlot,
  formatNumber,
  itemText,
  shortCommit,
  type ResultField,
} from './structuredResultFields';

/** Most paths a hover tooltip previews before it says "+N more". */
const TOOLTIP_PEEK = 10;

/** Body text follows the transcript's own scale, like the wake's reply excerpt. */
const BODY_FONT = 'calc(0.76rem * var(--claude-gui-font-scale, 1))';

/** Section/overline label — the same treatment the fleet card's badge uses. */
const Overline: React.FC<{ children: React.ReactNode; color?: string }> = ({ children, color }) => (
  <span
    style={{
      fontSize: '0.6rem',
      fontWeight: 700,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: color ?? colors.muted,
    }}
  >
    {children}
  </span>
);

/** Chevron toggle used by every collapsible section, so they all read alike. */
const Disclosure: React.FC<{
  open: boolean;
  onToggle: () => void;
  label: React.ReactNode;
  title?: string;
}> = ({ open, onToggle, label, title }) => (
  <button
    type="button"
    onClick={onToggle}
    title={title}
    aria-expanded={open}
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 4,
      padding: 0,
      border: 'none',
      background: 'transparent',
      cursor: 'pointer',
      fontFamily: 'inherit',
      fontSize: '0.66rem',
      fontWeight: 600,
      color: colors.muted,
      textAlign: 'left',
    }}
  >
    <ChevronRight
      size={11}
      aria-hidden
      style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform 0.15s' }}
    />
    {label}
  </button>
);

const chipStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  padding: '2px 7px',
  borderRadius: 'var(--wks-radius-pill)',
  fontSize: '0.66rem',
  whiteSpace: 'nowrap',
  maxWidth: '100%',
  minWidth: 0,
};

/** A yes/no answer. `merged` is the one everyone looks for first, but any
 *  boolean a schema invents gets the same badge. */
const BooleanChip: React.FC<{ field: ResultField }> = ({ field }) => {
  const yes = field.value === true;
  const tint = yes ? colors.success : colors.muted;
  return (
    <span
      title={`${field.label}: ${yes ? 'yes' : 'no'}`}
      style={{
        ...chipStyle,
        fontWeight: 600,
        color: tint,
        background: `color-mix(in srgb, ${tint} 12%, transparent)`,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}
    >
      {yes ? (
        <Check size={11} strokeWidth={2.25} aria-hidden />
      ) : (
        <X size={11} strokeWidth={2.25} aria-hidden />
      )}
      {field.label}
      <span style={{ opacity: 0.75, fontWeight: 500 }}>{yes ? 'yes' : 'no'}</span>
    </span>
  );
};

/** A count. Value first — "8 tests fixed" scans; "tests fixed: 8" does not. */
const NumberChip: React.FC<{ field: ResultField }> = ({ field }) => (
  <span
    title={`${field.label}: ${String(field.value)}`}
    style={{
      ...chipStyle,
      color: colors.text,
      border: `1px solid ${colors.borderSubtle}`,
      overflow: 'hidden',
      textOverflow: 'ellipsis',
    }}
  >
    <span
      style={{
        fontFamily: 'var(--wks-font-mono)',
        fontWeight: 600,
        color: colors.textBright,
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {formatNumber(field.value as number)}
    </span>
    {field.label}
  </span>
);

/** A SHA, in the Review pane's own mono treatment for a commit. Clicking
 *  copies it — there is no cross-pane deep link to a commit's diff to send it
 *  to, and a chip that pretended otherwise would advertise an action it can't
 *  perform. */
const CommitChip: React.FC<{ field: ResultField }> = ({ field }) => {
  const [copied, setCopied] = useState(false);
  const full = String(field.value).trim();
  const copy = async (): Promise<void> => {
    try {
      await navigator.clipboard?.writeText(full);
    } catch {
      return;
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };
  return (
    <button
      type="button"
      onClick={() => void copy()}
      title={copied ? 'Copied' : `${field.label} ${full} — click to copy`}
      aria-label={`Copy ${field.label} ${full}`}
      style={{
        ...chipStyle,
        border: `1px solid ${colors.borderSubtle}`,
        background: 'transparent',
        cursor: 'pointer',
        color: colors.muted,
        fontFamily: 'var(--wks-font-mono)',
        fontSize: '0.62rem',
      }}
    >
      <span style={{ color: colors.textBright, fontWeight: 600 }}>{shortCommit(full)}</span>
      {field.key !== 'commit' && <span>{field.label}</span>}
      {copied && <Check size={10} strokeWidth={2.25} aria-hidden />}
    </button>
  );
};

/** Long prose, clamped with an inline expander. The head always shows. */
const ClampedText: React.FC<{ text: string; limit: number; color?: string }> = ({
  text,
  limit,
  color,
}) => {
  const [open, setOpen] = useState(false);
  const long = text.length > limit;
  const shown = !long || open ? text : `${text.slice(0, limit).trimEnd()}…`;
  return (
    <span style={{ color: color ?? colors.text, wordBreak: 'break-word' }}>
      {shown}
      {long && (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          style={{
            marginLeft: 6,
            padding: 0,
            border: 'none',
            background: 'transparent',
            cursor: 'pointer',
            fontFamily: 'inherit',
            fontSize: '0.66rem',
            fontWeight: 600,
            color: colors.accent,
          }}
        >
          {open ? 'less' : 'more'}
        </button>
      )}
    </span>
  );
};

/** A nested object: one key/value row per entry, one level deep. Deeper
 *  structure falls back to its JSON — still shown, never dropped. */
const ObjectRows: React.FC<{ value: Record<string, unknown> }> = ({ value }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
    {Object.entries(value).map(([k, v]) => (
      <div key={k} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', minWidth: 0 }}>
        <span style={{ fontSize: '0.66rem', color: colors.muted, flexShrink: 0 }}>{k}</span>
        <span
          style={{
            fontSize: BODY_FONT,
            color: colors.text,
            wordBreak: 'break-word',
            minWidth: 0,
          }}
        >
          <ClampedText text={itemText(v)} limit={TEXT_CLAMP} />
        </span>
      </div>
    ))}
  </div>
);

const isRowObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** A list: shown whole when short, head + "+N more" when long. Items that are
 *  themselves objects get the key/value treatment rather than their JSON —
 *  an array of records is a shape a per-dispatch schema reaches for often. */
const StringList: React.FC<{ items: unknown[] }> = ({ items }) => {
  const [open, setOpen] = useState(false);
  const collapsible = items.length >= ARRAY_COLLAPSE_MIN;
  const shown = collapsible && !open ? items.slice(0, ARRAY_PREVIEW) : items;
  // Record items are multi-line, so they need air between them to read as
  // separate items; one-line items would only look sparse with it.
  const gap = items.some(isRowObject) ? 7 : 3;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap }}>
      {shown.map((item, i) => (
        <div
          key={i}
          style={{
            display: 'flex',
            gap: 6,
            fontSize: BODY_FONT,
            lineHeight: 1.5,
            color: colors.text,
            minWidth: 0,
          }}
        >
          <span aria-hidden style={{ color: colors.mutedDim, flexShrink: 0 }}>
            ·
          </span>
          <span style={{ wordBreak: 'break-word', minWidth: 0, flex: 1 }}>
            {isRowObject(item) ? (
              <ObjectRows value={item} />
            ) : (
              <ClampedText text={itemText(item)} limit={TEXT_CLAMP} />
            )}
          </span>
        </div>
      ))}
      {collapsible && (
        <Disclosure
          open={open}
          onToggle={() => setOpen((v) => !v)}
          label={open ? 'show fewer' : `+${items.length - ARRAY_PREVIEW} more`}
        />
      )}
    </div>
  );
};

/** File paths: the count is the headline, the list is on demand — and each
 *  path is a FileLink, the app's one path affordance. */
const PathList: React.FC<{ field: ResultField; cwd?: string }> = ({ field, cwd }) => {
  const [open, setOpen] = useState(false);
  const paths = field.value as string[];
  // A tooltip is a peek, not the list: a worker that touched 200 files would
  // otherwise hand the pointer a 200-line hover.
  const peek = paths.slice(0, TOOLTIP_PEEK);
  const title =
    paths.length > TOOLTIP_PEEK
      ? `${peek.join('\n')}\n+${paths.length - TOOLTIP_PEEK} more`
      : peek.join('\n');
  return (
    <div>
      <Disclosure
        open={open}
        onToggle={() => setOpen((v) => !v)}
        title={title}
        label={
          <span>
            <span style={{ color: colors.textBright, fontVariantNumeric: 'tabular-nums' }}>
              {paths.length}
            </span>{' '}
            <span style={{ textTransform: 'uppercase', letterSpacing: '0.08em' }}>
              {field.label}
            </span>
          </span>
        }
      />
      {open && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
          {paths.map((p, i) => (
            <FileLink
              key={`${p}-${i}`}
              path={p}
              cwd={cwd}
              style={{ fontSize: '0.68rem', wordBreak: 'break-all' }}
            >
              {p}
            </FileLink>
          ))}
        </div>
      )}
    </div>
  );
};

/** One body field: label overline, then the value in its own shape. */
const BodyField: React.FC<{ field: ResultField; cwd?: string }> = ({ field, cwd }) => {
  if (field.kind === 'paths') return <PathList field={field} cwd={cwd} />;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
      <Overline>{field.label}</Overline>
      {field.kind === 'empty' && (
        <span style={{ fontSize: BODY_FONT, color: colors.muted }}>{emptyLabel(field.value)}</span>
      )}
      {field.kind === 'text' && (
        <span style={{ fontSize: BODY_FONT, lineHeight: 1.55 }}>
          <ClampedText text={String(field.value)} limit={TEXT_CLAMP} />
        </span>
      )}
      {(field.kind === 'strings' || field.kind === 'list') && (
        <StringList items={field.value as unknown[]} />
      )}
      {field.kind === 'object' && <ObjectRows value={field.value as Record<string, unknown>} />}
    </div>
  );
};

/** The band that must never be behind a fold. */
const CaveatsBand: React.FC<{ field: ResultField }> = ({ field }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      gap: 3,
      padding: '6px 10px',
      borderLeft: `2px solid ${colors.warning}`,
      minWidth: 0,
    }}
  >
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
      <span style={{ display: 'flex', color: colors.warning }}>
        <AlertTriangle size={11} strokeWidth={2} aria-hidden />
      </span>
      <Overline color={colors.warning}>{field.label}</Overline>
    </span>
    {field.kind === 'empty' ? (
      <span style={{ fontSize: BODY_FONT, color: colors.muted }}>
        {field.value === null || field.value === undefined ? 'not reported' : 'none reported'}
      </span>
    ) : (
      <span style={{ fontSize: BODY_FONT, lineHeight: 1.55, color: colors.textBright }}>
        <ClampedText text={itemText(field.value)} limit={CAVEATS_CLAMP} color={colors.textBright} />
      </span>
    )}
  </div>
);

/** The contract was asked for and did not arrive — say so, quietly but
 *  plainly. The prose report is still in the wake beside it. */
const MissingNotice: React.FC<{ reason: string }> = ({ reason }) => (
  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 6, minWidth: 0 }}>
    <span style={{ display: 'flex', color: colors.error, paddingTop: 1 }}>
      <AlertTriangle size={11} strokeWidth={2} aria-hidden />
    </span>
    <span style={{ minWidth: 0 }}>
      <Overline color={colors.error}>no structured result</Overline>
      <span
        style={{
          display: 'block',
          marginTop: 2,
          fontSize: BODY_FONT,
          lineHeight: 1.5,
          color: colors.text,
          wordBreak: 'break-word',
        }}
      >
        {reason}
      </span>
    </span>
  </div>
);

/**
 * The card. `json` is the validated object as the wake carried it (pretty
 * JSON); `error` is why one could not be read. Rendering either is additive —
 * the worker's prose report sits above it untouched.
 */
const StructuredResultCardInner: React.FC<{
  json?: string;
  error?: string;
  /** Worker cwd, so a relative path in the result resolves to a real file. */
  cwd?: string;
}> = ({ json, error, cwd }) => {
  if (!json && !error) return null;
  const view = json ? buildResultView(json) : { fields: [] };
  const summary = fieldsInSlot(view, 'summary');
  const caveats = fieldsInSlot(view, 'caveats');
  const body = fieldsInSlot(view, 'body');
  return (
    <Surface
      elevation="flat"
      radius="sm"
      className="wks-hover-host"
      tone={error ? colors.error : undefined}
      style={{ marginTop: 6, padding: '7px 10px 8px 10px', minWidth: 0 }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
        <span style={{ display: 'flex', color: colors.muted }}>
          <ClipboardList size={11} strokeWidth={2} aria-hidden />
        </span>
        <Overline>structured result</Overline>
        <span style={{ flex: 1 }} />
        {json && (
          <span className="wks-hover-actions">
            <CopyTextButton text={json} label="Copy the result JSON" />
          </span>
        )}
      </div>

      {error && (
        <div style={{ marginTop: 5 }}>
          <MissingNotice reason={error} />
        </div>
      )}

      {view.fallback && (
        <div style={{ marginTop: 5, display: 'flex', flexDirection: 'column', gap: 4 }}>
          <MissingNotice reason={`${view.fallback.reason} — showing it as it arrived`} />
          {view.fallback.text && (
            <pre
              style={{
                margin: 0,
                padding: '6px 8px',
                borderLeft: `2px solid ${colors.borderSubtle}`,
                fontFamily: 'var(--claude-mono-font, monospace)',
                fontSize: '0.66rem',
                lineHeight: 1.45,
                color: colors.text,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
                maxHeight: 220,
                overflow: 'auto',
              }}
            >
              {view.fallback.text}
            </pre>
          )}
        </div>
      )}

      {summary.length > 0 && (
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            alignItems: 'center',
            gap: 6,
            marginTop: 6,
            minWidth: 0,
          }}
        >
          {summary.map((f) =>
            f.kind === 'boolean' ? (
              <BooleanChip key={f.key} field={f} />
            ) : f.kind === 'number' ? (
              <NumberChip key={f.key} field={f} />
            ) : (
              <CommitChip key={f.key} field={f} />
            ),
          )}
        </div>
      )}

      {caveats.map((f) => (
        <div key={f.key} style={{ marginTop: 6 }}>
          <CaveatsBand field={f} />
        </div>
      ))}

      {body.length > 0 && (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 7,
            marginTop: 7,
            minWidth: 0,
          }}
        >
          {body.map((f) => (
            <BodyField key={f.key} field={f} cwd={cwd} />
          ))}
        </div>
      )}
    </Surface>
  );
};

export const StructuredResultCard = React.memo(StructuredResultCardInner);
