import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LayoutGrid, Archive, AlertTriangle, RefreshCw, FolderOpen } from 'lucide-react';
import { claudeColors as colors } from '../components/claude-shared';
import { ProjectMark } from '../components/ProjectMark';
import {
  BOARD_COLUMNS,
  BOARD_COLUMN_LABELS,
  BRIEF_STATUS_LABELS,
  statusRank,
  type BoardColumn,
  type BriefCard,
  type BriefStatus,
} from '../../../main/shared/briefBoard';
import type { BoardData, BoardLane } from '../../../main/services/briefBoardService';

/**
 * The brief board: every `.workspacer/brief.md` the fleet keeps, rendered as
 * topic cards, one swimlane per project plus the manager's own fleet brief.
 *
 * THE DRAG IS REAL, and that is the point rather than a flourish. A kanban
 * shape promises you can move cards, so a board where dragging did nothing
 * would advertise a guarantee the plumbing cannot honour. Here every drop is a
 * write: moving a card between `Now` / `Direction` / `Recently` relocates the
 * entry's lines inside the markdown, and dropping one on **Archive** moves it
 * out of `brief.md` and into `brief.archive.md` beside it — the `/checkpoint`
 * chore that never gets done. That is the feature; the dashboard is a
 * side-effect of it.
 *
 * WHAT IT DELIBERATELY DOES NOT DO
 *  - It does not edit entry text. The user's hand-written wording is
 *    authoritative and survives every move byte for byte (see
 *    main/shared/briefBoard). Status and archival are the whole surface.
 *  - It does not rank or hide. Every entry gets a card, including ones under a
 *    section the board has no column for (they land in the lane footer), and
 *    including ones whose status could not be determined. The moment a board
 *    decides what matters, you stop trusting it is the whole picture.
 *  - It does not invent a status. A card whose entry says nothing about its own
 *    state shows no chip, rather than being labelled from the section it sits
 *    in — stale resolved entries sit unpruned in `## Now`, and calling those
 *    "in flight" would relabel exactly the rot this board exists to surface.
 *
 * Cards are read once per open and re-read from disk after every write, so what
 * you see is the file, never what the pane hoped the file became.
 */

const STATUS_COLOR: Record<BriefStatus, string> = {
  'waiting-on-you': colors.error,
  'in-flight': colors.accent,
  'next-up': colors.purple,
  landed: colors.success,
};

/** Drag payload. A card is identified by its lane and its content hash, so a
 *  drop that lands after the brief changed underneath resolves to "that entry
 *  is gone" rather than to the wrong entry. */
interface DragPayload {
  laneKey: string;
  cardId: string;
  from: BoardColumn;
}

const DRAG_MIME = 'application/x-wks-brief-card';

// ── Card ─────────────────────────────────────────────────────────────────────

const Card: React.FC<{
  card: BriefCard;
  laneKey: string;
  onDragStart: (p: DragPayload) => void;
  onDragEnd: () => void;
  busy: boolean;
}> = ({ card, laneKey, onDragStart, onDragEnd, busy }) => {
  const [expanded, setExpanded] = useState(false);
  // Archived cards are not draggable: the archive is cold storage, append-only
  // by the /checkpoint doctrine, and a board that let you drag back out of it
  // would be promising an un-archive the file format does not support.
  const draggable = !card.archived && !busy;

  return (
    <div
      draggable={draggable}
      onDragStart={(e) => {
        const payload: DragPayload = { laneKey, cardId: card.id, from: card.column };
        e.dataTransfer.setData(DRAG_MIME, JSON.stringify(payload));
        e.dataTransfer.effectAllowed = 'move';
        onDragStart(payload);
      }}
      onDragEnd={onDragEnd}
      onClick={() => setExpanded((v) => !v)}
      title={card.text}
      style={{
        border: `1px solid ${card.retracted ? colors.warning : colors.borderSubtle}`,
        borderLeft: `2px solid ${card.status ? STATUS_COLOR[card.status] : colors.divider}`,
        borderRadius: 6,
        background: colors.bgSecondary,
        padding: '7px 9px',
        cursor: draggable ? 'grab' : 'default',
        opacity: busy ? 0.5 : 1,
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 6,
          color: colors.textBright,
          fontSize: '0.78rem',
          fontWeight: 600,
          lineHeight: 1.3,
        }}
      >
        {card.marker && <span style={{ flex: 'none' }}>{card.marker}</span>}
        <span>{card.title}</span>
      </div>

      {(card.status || card.retracted || card.date) && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 5, flexWrap: 'wrap' }}>
          {card.status && (
            <span
              style={{
                color: STATUS_COLOR[card.status],
                fontSize: '0.6rem',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.04em',
              }}
            >
              {BRIEF_STATUS_LABELS[card.status]}
            </span>
          )}
          {card.retracted && (
            <span
              title="This entry leads with a retraction — its own bold claim is the one being corrected."
              style={{
                color: colors.warning,
                fontSize: '0.6rem',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: '0.04em',
              }}
            >
              retracted
            </span>
          )}
          {card.date && (
            <span style={{ color: colors.mutedDim, fontSize: '0.62rem' }}>{card.date}</span>
          )}
        </div>
      )}

      {card.summary && !expanded && (
        <div
          style={{
            color: colors.muted,
            fontSize: '0.7rem',
            lineHeight: 1.4,
            display: '-webkit-box',
            WebkitLineClamp: 3,
            WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
          }}
        >
          {card.summary}
        </div>
      )}

      {/* Click to see the entry as the user wrote it. Nothing on this board is
          a summary you cannot check against the source. */}
      {expanded && (
        <div
          style={{
            color: colors.text,
            fontSize: '0.68rem',
            lineHeight: 1.5,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            borderTop: `1px solid ${colors.divider}`,
            paddingTop: 5,
          }}
        >
          {card.text}
        </div>
      )}

      {card.refs.length > 0 && (
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
          {card.refs.map((r) => (
            <span
              key={r}
              style={{
                fontFamily: 'var(--wks-font-mono, monospace)',
                fontSize: '0.6rem',
                color: colors.mutedDim,
                background: colors.bgToolbar,
                borderRadius: 3,
                padding: '1px 4px',
              }}
            >
              {r}
            </span>
          ))}
        </div>
      )}
    </div>
  );
};

// ── Column ───────────────────────────────────────────────────────────────────

const Column: React.FC<{
  column: BoardColumn;
  lane: BoardLane;
  cards: BriefCard[];
  dragging: DragPayload | null;
  busyId: string | null;
  onDragStart: (p: DragPayload) => void;
  onDragEnd: () => void;
  onDrop: (p: DragPayload, to: BoardColumn) => void;
}> = ({ column, lane, cards, dragging, busyId, onDragStart, onDragEnd, onDrop }) => {
  const [over, setOver] = useState(false);
  // A drop is only legal within its own lane: an entry belongs to a project's
  // brief, and moving it across projects would be a rewrite of two files, not a
  // move — out of scope, and out of the "status and archival only" contract.
  const accepts = !!dragging && dragging.laneKey === lane.key && dragging.from !== column;

  return (
    <div
      onDragOver={(e) => {
        if (!accepts) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        if (!accepts) return;
        e.preventDefault();
        try {
          const raw = e.dataTransfer.getData(DRAG_MIME);
          if (raw) onDrop(JSON.parse(raw) as DragPayload, column);
        } catch {
          /* a drop we cannot read is a drop that does nothing */
        }
      }}
      style={{
        flex: 1,
        minWidth: 190,
        display: 'flex',
        flexDirection: 'column',
        gap: 5,
        padding: 6,
        borderRadius: 6,
        border: `1px ${accepts ? 'dashed' : 'solid'} ${
          over ? colors.accent : accepts ? colors.borderSubtle : 'transparent'
        }`,
        background: over ? colors.bgSecondary : 'transparent',
        transition: 'background 120ms, border-color 120ms',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 5,
          color: colors.muted,
          fontSize: '0.62rem',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          padding: '0 2px',
        }}
      >
        {column === 'archive' && <Archive size={11} strokeWidth={2} />}
        <span>{BOARD_COLUMN_LABELS[column]}</span>
        <span style={{ color: colors.mutedDim, fontWeight: 500 }}>{cards.length}</span>
      </div>

      {cards.length === 0 && (
        <div style={{ color: colors.mutedDim, fontSize: '0.66rem', padding: '4px 2px' }}>
          {column === 'archive' ? 'Drop a card here to file it away' : 'Nothing here'}
        </div>
      )}

      {cards.map((c) => (
        <Card
          key={c.id}
          card={c}
          laneKey={lane.key}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          busy={busyId === c.id}
        />
      ))}
    </div>
  );
};

// ── Lane ─────────────────────────────────────────────────────────────────────

const Lane: React.FC<{
  lane: BoardLane;
  dragging: DragPayload | null;
  busyId: string | null;
  onDragStart: (p: DragPayload) => void;
  onDragEnd: () => void;
  onDrop: (p: DragPayload, to: BoardColumn) => void;
}> = ({ lane, dragging, busyId, onDragStart, onDragEnd, onDrop }) => {
  const byColumn = useMemo(() => {
    const m = new Map<BoardColumn, BriefCard[]>();
    for (const col of BOARD_COLUMNS) m.set(col, []);
    for (const c of lane.cards) m.get(c.column)?.push(c);
    // Same-status cards cluster, file order within. Nothing is dropped and
    // nothing is ranked by importance.
    for (const list of m.values()) {
      list.sort((a, b) => statusRank(a.status) - statusRank(b.status));
    }
    return m;
  }, [lane.cards]);

  return (
    <div
      style={{
        border: `1px solid ${lane.kind === 'fleet' ? colors.accent : colors.border}`,
        borderRadius: 8,
        background: colors.bg,
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 7,
          padding: '7px 10px',
          borderBottom: `1px solid ${colors.divider}`,
          background: colors.bgToolbar,
        }}
      >
        <ProjectMark cwd={lane.dir} size={16} />
        <span style={{ color: colors.textBright, fontSize: '0.8rem', fontWeight: 600 }}>
          {lane.label}
        </span>
        {lane.kind === 'fleet' && (
          <span
            title="The Fleet Manager's own brief"
            style={{
              color: colors.accent,
              fontSize: '0.58rem',
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: '0.05em',
            }}
          >
            fleet
          </span>
        )}
        <span style={{ flex: 1 }} />
        {lane.indexed && (
          <span
            title="A .workspacer/brief.index.json is supplying card titles"
            style={{ color: colors.mutedDim, fontSize: '0.6rem' }}
          >
            indexed
          </span>
        )}
        <span title={lane.briefPath} style={{ color: colors.mutedDim, fontSize: '0.62rem' }}>
          {lane.cards.filter((c) => !c.archived).length} in the brief
        </span>
      </div>

      {lane.error ? (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            padding: '10px 12px',
            color: colors.error,
            fontSize: '0.72rem',
          }}
        >
          <AlertTriangle size={13} strokeWidth={2} />
          Couldn’t read {lane.briefPath} — {lane.error}
        </div>
      ) : !lane.exists ? (
        // A missing brief and an unreadable one are different answers and must
        // not read the same: only the first is an invitation to write one.
        <div style={{ padding: '10px 12px', color: colors.muted, fontSize: '0.72rem' }}>
          No brief here yet. Ask the Fleet Manager to set one up and it writes{' '}
          <code style={{ color: colors.text }}>{lane.briefPath}</code>.
        </div>
      ) : (
        <>
          <div style={{ display: 'flex', gap: 4, padding: 6, alignItems: 'flex-start' }}>
            {BOARD_COLUMNS.map((col) => (
              <Column
                key={col}
                column={col}
                lane={lane}
                cards={byColumn.get(col) ?? []}
                dragging={dragging}
                busyId={busyId}
                onDragStart={onDragStart}
                onDragEnd={onDragEnd}
                onDrop={onDrop}
              />
            ))}
          </div>

          {/* Entries under a section with no column of its own (`## User` on
              the fleet brief). Shown, not dropped — the board's claim is that
              it is the whole brief. */}
          {lane.extras.length > 0 && (
            <div
              style={{
                borderTop: `1px solid ${colors.divider}`,
                padding: '6px 10px 8px',
                display: 'flex',
                flexDirection: 'column',
                gap: 4,
              }}
            >
              <div
                style={{
                  color: colors.mutedDim,
                  fontSize: '0.6rem',
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: '0.05em',
                }}
              >
                Other sections
              </div>
              {lane.extras.map((c) => (
                <div key={c.id} title={c.text} style={{ fontSize: '0.7rem', color: colors.text }}>
                  <span style={{ color: colors.mutedDim }}>{c.group ?? '—'} · </span>
                  {c.title}
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
};

// ── Pane ─────────────────────────────────────────────────────────────────────

export const BoardPane: React.FC = () => {
  const [board, setBoard] = useState<BoardData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [dragging, setDragging] = useState<DragPayload | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const reload = useCallback(async () => {
    const api = window.electronAPI?.loadBriefBoard;
    if (!api) {
      setError('The brief board needs the desktop app — it writes to local files.');
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const data = await api();
      if (!alive.current) return;
      setBoard(data);
      setError(null);
    } catch (err) {
      if (alive.current) setError((err as Error).message);
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const onDrop = useCallback(
    async (payload: DragPayload, to: BoardColumn) => {
      const move = window.electronAPI?.moveBriefCard;
      if (!move) return;
      setBusyId(payload.cardId);
      setNotice(null);
      try {
        const lane = await move({ key: payload.laneKey, entryId: payload.cardId, to });
        if (!alive.current) return;
        // Swap in the lane the MAIN process re-read from disk. Never a local
        // optimistic edit: the board's whole claim is that it shows the file.
        setBoard((prev) =>
          prev ? { ...prev, lanes: prev.lanes.map((l) => (l.key === lane.key ? lane : l)) } : prev,
        );
        setNotice(
          to === 'archive'
            ? 'Archived — the entry moved into brief.archive.md.'
            : `Moved to ## ${to}.`,
        );
      } catch (err) {
        if (!alive.current) return;
        // A refused move means the brief changed under us. Say so and re-read,
        // rather than leaving a card sitting in a column the file disagrees with.
        setNotice(`Not moved — ${(err as Error).message}`);
        void reload();
      } finally {
        if (alive.current) setBusyId(null);
      }
    },
    [reload],
  );

  const totalCards = board?.lanes.reduce((n, l) => n + l.cards.length, 0) ?? 0;

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        background: colors.bg,
        color: colors.text,
        fontFamily: 'var(--wks-font-ui, inherit)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '9px 12px',
          borderBottom: `1px solid ${colors.divider}`,
          position: 'sticky',
          top: 0,
          background: colors.bgToolbar,
          zIndex: 1,
        }}
      >
        <LayoutGrid size={14} strokeWidth={1.9} color={colors.accent} />
        <span style={{ color: colors.textBright, fontSize: '0.82rem', fontWeight: 600 }}>
          Brief board
        </span>
        <span style={{ color: colors.mutedDim, fontSize: '0.68rem' }}>
          {totalCards} card{totalCards === 1 ? '' : 's'} across {board?.lanes.length ?? 0} project
          {board?.lanes.length === 1 ? '' : 's'}
        </span>
        <span style={{ flex: 1 }} />
        {notice && (
          <span style={{ color: colors.muted, fontSize: '0.68rem' }} role="status">
            {notice}
          </span>
        )}
        <button
          onClick={() => void reload()}
          title="Re-read every brief from disk"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
            background: 'transparent',
            border: `1px solid ${colors.borderSubtle}`,
            borderRadius: 5,
            color: colors.muted,
            fontSize: '0.68rem',
            padding: '3px 7px',
            cursor: 'pointer',
          }}
        >
          <RefreshCw size={11} strokeWidth={2} />
          Reload
        </button>
      </div>

      <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div style={{ color: colors.mutedDim, fontSize: '0.68rem', lineHeight: 1.5 }}>
          Every project keeps a living brief at <b>.workspacer/brief.md</b>. Drag a card between
          columns to move the entry between the brief’s own sections — or onto <b>Archive</b> to
          move it out into <b>brief.archive.md</b>. The wording is never edited: only where the
          entry lives changes.
        </div>

        {error && (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              color: colors.error,
              fontSize: '0.72rem',
            }}
          >
            <AlertTriangle size={13} strokeWidth={2} />
            {error}
          </div>
        )}

        {loading && !board && (
          <div style={{ color: colors.muted, fontSize: '0.72rem' }}>Reading briefs…</div>
        )}

        {board?.lanes.length === 0 && !loading && (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              color: colors.muted,
              fontSize: '0.72rem',
            }}
          >
            <FolderOpen size={13} strokeWidth={2} />
            No projects registered yet — add one in Settings → Projects.
          </div>
        )}

        {board?.lanes.map((lane) => (
          <Lane
            key={lane.key}
            lane={lane}
            dragging={dragging}
            busyId={busyId}
            onDragStart={setDragging}
            onDragEnd={() => setDragging(null)}
            onDrop={(p, to) => void onDrop(p, to)}
          />
        ))}
      </div>
    </div>
  );
};

export default BoardPane;
