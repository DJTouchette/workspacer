/**
 * The BoardPane's load-bearing claim: THE DRAG IS REAL.
 *
 * A kanban shape promises you can move cards, so the thing worth testing is not
 * that a card renders — it is that dropping one calls the write, and that what
 * the board then shows is what the write returned from DISK rather than an
 * optimistic guess. The rest of the file pins the "show everything, invent
 * nothing" rules: no blank cards, no hidden entries, no status the entry did
 * not claim.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, cleanup } from '@testing-library/react';
import { BoardPane } from '../src/panes/BoardPane';
import { cardsForBrief } from '../../main/shared/briefBoard';
import type { BoardData, BoardLane } from '../../main/services/briefBoardService';

const BRIEF = [
  '## Now',
  '- ✅ **A resolved thing** that should have been pruned.',
  '- 🚧 **A live thing** that is genuinely in flight.',
  '- a plain entry that says nothing about its own state',
  '',
  '## Direction',
  '- A durable goal.',
  '',
  '## Recently',
  '- 2026-08-22: something merged.',
  '',
  '## User',
  '- Prefers fewer blocking questions.',
  '',
].join('\n');

function lane(overrides: Partial<BoardLane> = {}): BoardLane {
  const { cards, extras } = cardsForBrief(BRIEF);
  return {
    key: '/home/u/Work/demo',
    dir: '/home/u/Work/demo',
    label: 'Demo',
    kind: 'project',
    briefPath: '/home/u/Work/demo/.workspacer/brief.md',
    archivePath: '/home/u/Work/demo/.workspacer/brief.archive.md',
    exists: true,
    cards,
    extras,
    indexed: false,
    ...overrides,
  };
}

const board = (lanes: BoardLane[]): BoardData => ({
  lanes,
  columns: ['Now', 'Direction', 'Recently', 'archive'],
});

let loadBriefBoard: ReturnType<typeof vi.fn>;
let moveBriefCard: ReturnType<typeof vi.fn>;

beforeEach(() => {
  loadBriefBoard = vi.fn().mockResolvedValue(board([lane()]));
  moveBriefCard = vi.fn();
  (window as unknown as { electronAPI: unknown }).electronAPI = {
    loadBriefBoard,
    moveBriefCard,
  };
});

afterEach(() => {
  // No global auto-cleanup in this suite's setup, so the DOM would accumulate
  // and every query would find two of everything. `electronAPI` is defined
  // non-configurably by tests/setup.ts, so it is overwritten, never deleted.
  cleanup();
  (window as unknown as { electronAPI: unknown }).electronAPI = {};
  vi.restoreAllMocks();
});

/** A DataTransfer good enough for jsdom, which ships none. */
function dataTransfer(): DataTransfer {
  const store = new Map<string, string>();
  return {
    setData: (k: string, v: string) => store.set(k, v),
    getData: (k: string) => store.get(k) ?? '',
    effectAllowed: '',
    dropEffect: '',
  } as unknown as DataTransfer;
}

/** The drop target for one lane's column. */
function column(col: string, laneKey = '/home/u/Work/demo'): HTMLElement {
  const el = document.querySelector(`[data-lane="${laneKey}"][data-column="${col}"]`);
  if (!el) throw new Error(`no ${col} column for ${laneKey}`);
  return el as HTMLElement;
}

/** Drag the card whose title is `title` onto `col` in its lane. */
async function drag(title: string, col: string, laneKey?: string): Promise<void> {
  const card = (await screen.findByText(title)).closest('[draggable]') as HTMLElement;
  const dt = dataTransfer();
  fireEvent.dragStart(card, { dataTransfer: dt });
  const target = column(col, laneKey);
  fireEvent.dragOver(target, { dataTransfer: dt });
  fireEvent.drop(target, { dataTransfer: dt });
}

describe('<BoardPane> — the drag writes back', () => {
  it('calls the write with the dragged entry, its lane and the target column', async () => {
    moveBriefCard.mockImplementation(async () => lane());
    render(<BoardPane />);
    const card = (await screen.findByText('A resolved thing')).closest(
      '[draggable]',
    ) as HTMLElement;
    const id = cardsForBrief(BRIEF).cards.find((c) => c.title === 'A resolved thing')!.id;

    const dt = dataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    const target = column('Recently');
    fireEvent.dragOver(target, { dataTransfer: dt });
    fireEvent.drop(target, { dataTransfer: dt });

    await waitFor(() => expect(moveBriefCard).toHaveBeenCalledTimes(1));
    expect(moveBriefCard).toHaveBeenCalledWith({
      key: '/home/u/Work/demo',
      entryId: id,
      to: 'Recently',
    });
  });

  it('ARCHIVING is a write too, and says where the entry went', async () => {
    moveBriefCard.mockImplementation(async () => lane({ cards: [], extras: [] }));
    render(<BoardPane />);
    await drag('A resolved thing', 'archive');

    await waitFor(() => expect(moveBriefCard).toHaveBeenCalledTimes(1));
    expect(moveBriefCard.mock.calls[0][0].to).toBe('archive');
    expect(await screen.findByText(/moved into brief\.archive\.md/i)).toBeTruthy();
  });

  it('renders the lane the WRITE returned, not an optimistic guess', async () => {
    // Main says the entry came back with a different title than the board had.
    // If the pane were guessing locally, this would never appear.
    const after = lane();
    after.cards = after.cards.map((c) =>
      c.title === 'A resolved thing' ? { ...c, title: 'What disk actually says' } : c,
    );
    moveBriefCard.mockResolvedValue(after);

    render(<BoardPane />);
    await drag('A resolved thing', 'Recently');
    expect(await screen.findByText('What disk actually says')).toBeTruthy();
  });

  it('reports a refused move and re-reads, instead of leaving the card where it was dropped', async () => {
    moveBriefCard.mockRejectedValue(new Error('brief.md changed under us'));
    render(<BoardPane />);
    await screen.findByText('A resolved thing');
    expect(loadBriefBoard).toHaveBeenCalledTimes(1);

    await drag('A resolved thing', 'Recently');

    expect(await screen.findByText(/not moved.*changed under us/i)).toBeTruthy();
    await waitFor(() => expect(loadBriefBoard).toHaveBeenCalledTimes(2));
  });

  it('does not offer to drag an ARCHIVED card — cold storage is append-only', async () => {
    const withArchive = lane();
    withArchive.cards = [
      ...withArchive.cards,
      {
        ...withArchive.cards[0],
        id: 'arch1',
        title: 'An archived thing',
        column: 'archive',
        archived: true,
      },
    ];
    loadBriefBoard.mockResolvedValue(board([withArchive]));
    render(<BoardPane />);
    const card = (await screen.findByText('An archived thing')).closest(
      '[draggable]',
    ) as HTMLElement;
    expect(card.getAttribute('draggable')).toBe('false');
  });

  it('refuses a drop into a different project’s lane', async () => {
    const other = lane({ key: '/home/u/Work/other', dir: '/home/u/Work/other', label: 'Other' });
    loadBriefBoard.mockResolvedValue(board([lane(), other]));
    render(<BoardPane />);

    const card = (await screen.findAllByText('A resolved thing'))[0].closest(
      '[draggable]',
    ) as HTMLElement;
    const dt = dataTransfer();
    fireEvent.dragStart(card, { dataTransfer: dt });
    // The OTHER lane's Recently column belongs to a different brief.
    fireEvent.drop(column('Recently', '/home/u/Work/other'), { dataTransfer: dt });

    expect(moveBriefCard).not.toHaveBeenCalled();
  });
});

describe('<BoardPane> — shows everything, invents nothing', () => {
  it('renders every entry, and no blank card', async () => {
    render(<BoardPane />);
    for (const title of [
      'A resolved thing',
      'A live thing',
      'a plain entry that says nothing about its own state',
      'A durable goal.',
    ]) {
      expect(await screen.findByText(title)).toBeTruthy();
    }
  });

  it('shows an entry from a section with no column of its own, rather than dropping it', async () => {
    render(<BoardPane />);
    // `## User` has no column; it belongs in the lane footer, not nowhere.
    expect(await screen.findByText(/Other sections/i)).toBeTruthy();
    expect(await screen.findByText(/Prefers fewer blocking questions/)).toBeTruthy();
  });

  it('shows no status chip for an entry that never claimed one', async () => {
    render(<BoardPane />);
    const card = (
      await screen.findByText('a plain entry that says nothing about its own state')
    ).closest('[draggable]') as HTMLElement;
    expect(card.textContent).not.toMatch(/in flight|landed|waiting on you|next up/i);
  });

  it('labels a live entry with /standup’s own wording', async () => {
    render(<BoardPane />);
    const card = (await screen.findByText('A live thing')).closest('[draggable]') as HTMLElement;
    expect(card.textContent).toMatch(/In flight/i);
  });

  it('invites a brief for a project that has none, without pretending it failed', async () => {
    loadBriefBoard.mockResolvedValue(board([lane({ exists: false, cards: [], extras: [] })]));
    render(<BoardPane />);
    expect(await screen.findByText(/No brief here yet/i)).toBeTruthy();
  });

  it('says so when the board is not available at all (web)', async () => {
    (window as unknown as { electronAPI: unknown }).electronAPI = {};
    render(<BoardPane />);
    expect(await screen.findByText(/needs the desktop app/i)).toBeTruthy();
  });
});
