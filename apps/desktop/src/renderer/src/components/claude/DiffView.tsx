import React from 'react';
import { claudeColors as colors } from '../claude-shared';
import { langForPath } from '../../lib/diff/highlight';
import { useHighlight, renderLine } from './highlight';
import { inlineRows, splitRows } from '../../lib/diff/lineDiff';
import { useConfig } from '../../hooks/useConfig';
import { FileLink } from './FileLink';

/** How the GUI lays out a diff. See config.ui.diffView. */
export type DiffViewMode = 'stacked' | 'inline' | 'split';

const DEL_BG = 'rgba(248, 113, 113, 0.08)';
const ADD_BG = 'rgba(74, 222, 128, 0.08)';
const DEL_FG = 'rgb(248, 150, 150)';
const ADD_FG = 'rgb(150, 230, 170)';
const EMPTY_BG = 'rgba(255,255,255,0.02)';

const gutterStyle = (color: string): React.CSSProperties => ({
  color,
  userSelect: 'none',
  width: 36,
  minWidth: 36,
  textAlign: 'right',
  padding: '1px 6px 1px 0',
  fontSize: '0.65rem',
});
const markerStyle = (color: string): React.CSSProperties => ({
  color,
  userSelect: 'none',
  width: 16,
  minWidth: 16,
  textAlign: 'center',
  padding: '1px 0',
});
const contentStyle: React.CSSProperties = {
  padding: '1px 8px 1px 0',
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-all',
  flex: 1,
  minWidth: 0,
};

/**
 * Render an Edit/MultiEdit diff. Three layouts (config.ui.diffView):
 *  - stacked: every removed line then every added line (original look)
 *  - inline:  LCS-interleaved unified diff (shared lines shown once)
 *  - split:   side-by-side, removed left / added right, aligned
 */
export const DiffView: React.FC<{
  oldStr: string;
  newStr: string;
  filePath?: string;
  /** Session cwd — resolves relative paths for the header FileLink. */
  cwd?: string;
}> = ({ oldStr, newStr, filePath, cwd }) => {
  const { config } = useConfig();
  const mode: DiffViewMode = config.ui.diffView ?? 'stacked';
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const oldLines = oldStr ? oldStr.split('\n') : [];
  const newLines = newStr ? newStr.split('\n') : [];
  const addedCount = newLines.length;
  const removedCount = oldLines.length;
  // Syntax tokens per side, indexed by original line number; backgrounds stay
  // our add/del tints, foreground comes from the grammar (falls back to the
  // diff color until tokens load).
  const lang = langForPath(filePath ?? '');
  const oldTokens = useHighlight(oldStr, lang);
  const newTokens = useHighlight(newStr, lang);

  return (
    <div
      style={{
        margin: '6px 0',
        borderRadius: 'var(--wks-radius-sm)',
        overflow: 'hidden',
        border: `1px solid ${colors.borderSubtle}`,
        fontSize: '0.75rem',
        fontFamily: 'var(--claude-mono-font, monospace)',
      }}
    >
      {fileName && (
        <div
          style={{
            padding: '5px 12px',
            backgroundColor: 'rgba(255,255,255,0.03)',
            color: colors.text,
            fontSize: '0.72rem',
            fontWeight: 600,
            borderBottom: `1px solid ${colors.borderSubtle}`,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <FileLink path={filePath!} cwd={cwd}>
            {fileName}
          </FileLink>
          {removedCount > 0 && (
            <span style={{ color: colors.error, fontSize: '0.65rem', fontWeight: 400 }}>
              -{removedCount}
            </span>
          )}
          {addedCount > 0 && (
            <span style={{ color: colors.success, fontSize: '0.65rem', fontWeight: 400 }}>
              +{addedCount}
            </span>
          )}
        </div>
      )}
      <div style={{ maxHeight: 600, overflow: 'auto' }}>
        {mode === 'split' ? (
          <SplitBody
            oldLines={oldLines}
            newLines={newLines}
            oldTokens={oldTokens}
            newTokens={newTokens}
          />
        ) : mode === 'inline' ? (
          <InlineBody
            oldLines={oldLines}
            newLines={newLines}
            oldTokens={oldTokens}
            newTokens={newTokens}
          />
        ) : (
          <StackedBody
            oldLines={oldLines}
            newLines={newLines}
            oldTokens={oldTokens}
            newTokens={newTokens}
          />
        )}
      </div>
    </div>
  );
};

type Tokens = ReturnType<typeof useHighlight>;
interface BodyProps {
  oldLines: string[];
  newLines: string[];
  oldTokens: Tokens;
  newTokens: Tokens;
}

/** Original layout: all removed lines, then all added lines. */
const StackedBody: React.FC<BodyProps> = ({ oldLines, newLines, oldTokens, newTokens }) => (
  <>
    {oldLines.map((line, i) => (
      <div
        key={`old-${i}`}
        style={{ display: 'flex', backgroundColor: DEL_BG, color: DEL_FG, lineHeight: 1.5 }}
      >
        <span
          style={{
            ...gutterStyle('rgba(248,150,150,0.4)'),
            borderRight: '1px solid rgba(248,113,113,0.15)',
          }}
        >
          {i + 1}
        </span>
        <span style={markerStyle(colors.error)}>-</span>
        <span style={contentStyle}>{renderLine(oldTokens?.[i], line)}</span>
      </div>
    ))}
    {newLines.map((line, i) => (
      <div
        key={`new-${i}`}
        style={{ display: 'flex', backgroundColor: ADD_BG, color: ADD_FG, lineHeight: 1.5 }}
      >
        <span
          style={{
            ...gutterStyle('rgba(150,230,170,0.4)'),
            borderRight: '1px solid rgba(74,222,128,0.15)',
          }}
        >
          {i + 1}
        </span>
        <span style={markerStyle(colors.success)}>+</span>
        <span style={contentStyle}>{renderLine(newTokens?.[i], line)}</span>
      </div>
    ))}
  </>
);

/** Interleaved unified diff: shared lines once, removals/additions in place. */
const InlineBody: React.FC<BodyProps> = ({ oldLines, newLines, oldTokens, newTokens }) => (
  <>
    {inlineRows(oldLines, newLines).map((r, i) => {
      const isDel = r.kind === 'del';
      const isAdd = r.kind === 'add';
      const bg = isDel ? DEL_BG : isAdd ? ADD_BG : 'transparent';
      const fg = isDel ? DEL_FG : isAdd ? ADD_FG : colors.text;
      const marker = isDel ? '-' : isAdd ? '+' : ' ';
      const markerColor = isDel ? colors.error : isAdd ? colors.success : colors.mutedDim;
      const tok = isDel ? oldTokens?.[r.oldNo! - 1] : newTokens?.[r.newNo! - 1];
      return (
        <div key={i} style={{ display: 'flex', backgroundColor: bg, color: fg, lineHeight: 1.5 }}>
          <span style={gutterStyle(colors.mutedDim)}>{r.oldNo ?? ''}</span>
          <span
            style={{
              ...gutterStyle(colors.mutedDim),
              borderRight: `1px solid ${colors.borderSubtle}`,
            }}
          >
            {r.newNo ?? ''}
          </span>
          <span style={markerStyle(markerColor)}>{marker}</span>
          <span style={contentStyle}>{renderLine(tok, r.text)}</span>
        </div>
      );
    })}
  </>
);

/** Side-by-side: removed lines left, added lines right, LCS-aligned. */
const SplitBody: React.FC<BodyProps> = ({ oldLines, newLines, oldTokens, newTokens }) => (
  <div style={{ minWidth: 'min-content' }}>
    {splitRows(oldLines, newLines).map((r, i) => {
      const leftChanged = r.changed && r.left;
      const rightChanged = r.changed && r.right;
      return (
        <div key={i} style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', lineHeight: 1.5 }}>
          <div
            style={{
              display: 'flex',
              minWidth: 0,
              backgroundColor: leftChanged ? DEL_BG : r.left ? 'transparent' : EMPTY_BG,
              color: leftChanged ? DEL_FG : colors.text,
              borderRight: `1px solid ${colors.borderSubtle}`,
            }}
          >
            <span style={gutterStyle(colors.mutedDim)}>{r.left?.no ?? ''}</span>
            <span style={markerStyle(leftChanged ? colors.error : colors.mutedDim)}>
              {leftChanged ? '-' : ''}
            </span>
            <span style={contentStyle}>
              {r.left ? renderLine(oldTokens?.[r.left.no - 1], r.left.text) : ''}
            </span>
          </div>
          <div
            style={{
              display: 'flex',
              minWidth: 0,
              backgroundColor: rightChanged ? ADD_BG : r.right ? 'transparent' : EMPTY_BG,
              color: rightChanged ? ADD_FG : colors.text,
            }}
          >
            <span style={gutterStyle(colors.mutedDim)}>{r.right?.no ?? ''}</span>
            <span style={markerStyle(rightChanged ? colors.success : colors.mutedDim)}>
              {rightChanged ? '+' : ''}
            </span>
            <span style={contentStyle}>
              {r.right ? renderLine(newTokens?.[r.right.no - 1], r.right.text) : ''}
            </span>
          </div>
        </div>
      );
    })}
  </div>
);

/** Check if a tool call has diff-able content */
export function hasDiff(tc: { name: string; input?: Record<string, unknown> }): boolean {
  return (
    (tc.name === 'Edit' || tc.name === 'MultiEdit') &&
    !!(tc.input?.old_string || tc.input?.new_string)
  );
}

/** Cap how many read lines we render inline; the rest is summarized. */
const READ_PREVIEW_MAX = 200;

/**
 * Read-only, line-numbered preview of the lines a Read tool call returned —
 * the read analogue of DiffView. The response is in `cat -n` format
 * (`␣␣␣123⇥content`); we parse the gutter number off each line and fall back
 * to a sequential index if a line doesn't match.
 */
export const ReadView: React.FC<{
  response: string;
  filePath?: string;
  /** Session cwd — resolves relative paths for the header FileLink. */
  cwd?: string;
}> = ({ response, filePath, cwd }) => {
  const fileName = filePath?.split(/[/\\]/).pop() ?? '';
  const lines = response.split('\n');
  if (lines.length && lines[lines.length - 1] === '') lines.pop(); // trailing newline
  const shown = lines.slice(0, READ_PREVIEW_MAX);
  const hidden = lines.length - shown.length;
  // Strip the `cat -n` gutter to a {num, text} pair, then tokenize the file
  // content as one document so highlighting has cross-line context.
  const parsed = shown.map((raw, i) => {
    const m = /^\s*(\d+)\t(.*)$/.exec(raw);
    return { num: m ? m[1] : String(i + 1), text: m ? m[2] : raw };
  });
  const lang = langForPath(filePath ?? '');
  const tokens = useHighlight(parsed.map((p) => p.text).join('\n'), lang);

  return (
    <div
      style={{
        margin: '6px 0',
        borderRadius: 'var(--wks-radius-sm)',
        overflow: 'hidden',
        border: `1px solid ${colors.borderSubtle}`,
        fontSize: '0.75rem',
        fontFamily: 'var(--claude-mono-font, monospace)',
      }}
    >
      {fileName && (
        <div
          style={{
            padding: '5px 12px',
            backgroundColor: 'rgba(255,255,255,0.03)',
            color: colors.text,
            fontSize: '0.72rem',
            fontWeight: 600,
            borderBottom: `1px solid ${colors.borderSubtle}`,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <FileLink path={filePath!} cwd={cwd}>
            {fileName}
          </FileLink>
          <span style={{ color: colors.muted, fontSize: '0.65rem', fontWeight: 400 }}>
            {lines.length} lines
          </span>
        </div>
      )}
      <div style={{ maxHeight: 600, overflow: 'auto' }}>
        {parsed.map(({ num, text }, i) => (
          <div key={i} style={{ display: 'flex', lineHeight: 1.5 }}>
            <span
              style={{
                color: colors.mutedDim,
                userSelect: 'none',
                width: 44,
                minWidth: 44,
                textAlign: 'right',
                padding: '1px 8px 1px 0',
                fontSize: '0.65rem',
                borderRight: `1px solid ${colors.borderSubtle}`,
              }}
            >
              {num}
            </span>
            <span
              style={{
                color: colors.text,
                padding: '1px 8px',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                flex: 1,
              }}
            >
              {renderLine(tokens?.[i], text)}
            </span>
          </div>
        ))}
        {hidden > 0 && (
          <div style={{ padding: '2px 8px 2px 52px', color: colors.muted, fontSize: '0.65rem' }}>
            +{hidden} more line{hidden !== 1 ? 's' : ''}
          </div>
        )}
      </div>
    </div>
  );
};

/** Check if a tool call has read content to preview. */
export function hasRead(tc: { name: string; response?: unknown }): boolean {
  return tc.name === 'Read' && typeof tc.response === 'string' && tc.response.length > 0;
}
