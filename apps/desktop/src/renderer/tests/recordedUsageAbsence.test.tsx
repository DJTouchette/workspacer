/**
 * Unknown, unavailable and zero are three different facts on the AGENT
 * surfaces too.
 *
 * The per-agent surfaces (this card, the Fleet Deck list, the sidebar, the
 * status bar, the Inspector) get their cold-start figures from
 * `RecordedUsageContext`, whose map is built from the daemon session list. A
 * session with no entry in that map and a map that could not be BUILT AT ALL
 * both arrive as `undefined`, and the card used to answer both with the same
 * sentence — "No usage yet", which asserts this agent has spent nothing.
 *
 * Pinned here:
 *  - a recorded figure renders, labelled as last-recorded rather than live;
 *  - no figure, source healthy → "No usage recorded", never "$0.00";
 *  - no figure, source unreadable → "Usage unavailable", a different sentence,
 *    because nothing was measured to be missing.
 */
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { AgentCard } from '../src/components/AgentCard';
import { InspectorCard } from '../src/components/claude/InspectorCard';
import { RecordedUsageProvider } from '../src/contexts/RecordedUsageContext';
import { absentUsageTitle } from '../src/contexts/RecordedUsageContext';
import type { RecordedUsageBySession } from '../src/lib/recordedUsage';
import type { AgentWorkspace } from '../src/types/pane';

vi.mock('../src/contexts/AttentionContext', () => ({
  useAttention: () => ({
    openAgent: () => {},
    approve: () => {},
    answer: () => {},
    sendMessage: () => {},
    feed: [],
  }),
}));
vi.mock('../src/hooks/usePageVisible', () => ({ usePageVisible: () => true }));
vi.mock('../src/hooks/useGitBranch', () => ({ useGitBranch: () => '' }));
vi.mock('../src/components/AgentCardBody', () => ({
  AgentCardBody: () => <div data-testid="body" />,
}));

const SID = 'sess-1';
const agent = {
  id: 'a1',
  name: 'worker',
  cwd: '/home/u/Work/demo',
  provider: 'claude',
  sessionId: SID,
} as unknown as AgentWorkspace;

function renderCard(bySession: RecordedUsageBySession, unavailable: string | null = null) {
  return render(
    <RecordedUsageProvider value={bySession} unavailable={unavailable}>
      {/* No snapshot at all — the cold start this whole seam exists for. */}
      <AgentCard agent={agent} onOpen={() => {}} />
    </RecordedUsageProvider>,
  );
}

afterEach(cleanup);

describe('AgentCard — the three states of an absent figure', () => {
  it('renders a recorded figure, and says it is the last recorded one', () => {
    renderCard({ [SID]: { costUSD: 46.24, billedTokens: 30_000_000 } });
    expect(screen.getByText(/\$46\.24/)).toBeInTheDocument();
    // Not a live reading, and the card must not let the number imply it is.
    expect(screen.getByText(/last recorded/)).toBeInTheDocument();
  });

  it('says nothing was recorded — never $0.00 — when the source answered', () => {
    renderCard({});
    expect(screen.getByText('No usage recorded')).toBeInTheDocument();
    expect(screen.queryByText(/\$0\.00/)).toBeNull();
    expect(screen.getByTitle(absentUsageTitle(null))).toBeInTheDocument();
  });

  it('distinguishes a source it could not read from one with nothing to give', () => {
    renderCard({}, 'no provider for sessions.recent');
    expect(screen.getByText('Usage unavailable')).toBeInTheDocument();
    // The two absences must not share a sentence.
    expect(screen.queryByText('No usage recorded')).toBeNull();
    expect(
      screen.getByTitle(absentUsageTitle('no provider for sessions.recent')),
    ).toBeInTheDocument();
  });

  it('keeps a figure it already has when the source later goes unreadable', () => {
    // The list poll keeps its last good answer, so `unavailable` can be set
    // while real readings are still in hand. Those stay on screen; only the
    // ABSENCES are re-labelled.
    renderCard({ [SID]: { costUSD: 46.24 } }, 'hub link down');
    expect(screen.getByText(/\$46\.24/)).toBeInTheDocument();
    expect(screen.queryByText('Usage unavailable')).toBeNull();
  });
});

describe('InspectorCard — the Usage tab says which absence it is showing', () => {
  /** The Usage tab of a card bound to a session with NO live snapshot — the
   *  cold start the recorded-usage seam exists for. */
  function renderUsage(bySession: RecordedUsageBySession, unavailable: string | null = null) {
    render(
      <RecordedUsageProvider value={bySession} unavailable={unavailable}>
        <InspectorCard snapshot={null} sessionId={SID} />
      </RecordedUsageProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: /Usage/ }));
  }

  it('renders the recorded figures rather than short-circuiting to an empty tab', () => {
    // The tab used to gate on a LIVE source (`!sl && !usage`), which no cold
    // start has — so these tiles were unreachable in the only case they were
    // written for, and the tab said "No usage data yet" over a history DB that
    // held the numbers.
    renderUsage({ [SID]: { costUSD: 46.24, billedTokens: 30_000_000 } });
    expect(screen.getByText('$46.24')).toBeInTheDocument();
    expect(screen.getByText(/Cost · last recorded/)).toBeInTheDocument();
    expect(screen.getByText(/Billed tokens · last recorded/)).toBeInTheDocument();
    expect(screen.queryByText(/No usage data yet/)).toBeNull();
  });

  it('says there is no data — never $0.00 — when the source answered with none', () => {
    renderUsage({});
    expect(screen.getByText(/No usage data yet/)).toBeInTheDocument();
    expect(screen.queryByText(/\$0\.00/)).toBeNull();
  });

  it('names the reason instead when the record could not be consulted', () => {
    renderUsage({}, 'headless');
    expect(screen.getByText(/could not be read \(headless\)/)).toBeInTheDocument();
    // "we could not look" must not wear the sentence for "we looked, and it
    // was empty".
    expect(screen.queryByText(/No usage data yet/)).toBeNull();
  });
});
