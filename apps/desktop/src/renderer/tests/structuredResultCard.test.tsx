/**
 * The structured-result card, as a manager actually meets it: inside a fleet
 * wake, rendered by ConversationMessage from the wire format supervisorNudge
 * built. The fixture is a REAL result from a dispatch on 2026-08-23 (the
 * mobile e2e repair) — a boolean, a count, three long `checksRun` strings, a
 * one-element unknown array whose single item is a paragraph, and a
 * multi-sentence caveat.
 */
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { ConversationMessage } from '../src/components/claude/ConversationMessage';
import { StructuredResultCard } from '../src/components/claude/StructuredResultCard';
import { buildFleetMessage } from '../../main/shared/fleetMessages';
import type { ConversationTurn } from '../src/types/claudeSession';

const REAL_RESULT = {
  commit: 'e124a078',
  merged: true,
  testsFixed: 8,
  filesChanged: [
    'apps/desktop/tests/e2e/mobileClient.test.ts',
    'services/hub/cmd/hub/mobile.html',
    '.github/workflows/ci.yml',
  ],
  realBugsFound: [
    'mobile.html fetchConv(): the conversation-poll self-rearm only fires via a full render() call, but a no-op fetch (empty/turnless conversation) deliberately skips render() to avoid a past DOM-thrash bug — which also silently stopped all future polling after the first fetch. An agent with no conversation yet would never get its transcript checked again once the chat screen opened.',
  ],
  checksRun: [
    'npx playwright test tests/e2e/mobileClient.test.ts (run twice for stability, 22/22 both times)',
    'npx vitest run (apps/desktop main): 2154/2154, matches baseline',
    'npx vitest run (apps/desktop/src/renderer): 1142/1142, matches baseline',
  ],
  followUps: ['Watch the first live CI run of the new Mobile client e2e step in ci.yml'],
  caveats:
    'ci.yml change was not run through actionlint or an actual GHA runner (no such tool available in this environment). Did not push per instructions; master is still ~21 commits ahead of origin as before.',
};

const wake = (result: unknown, extra: Record<string, unknown> = {}): ConversationTurn => ({
  role: 'user',
  content: buildFleetMessage('worker-finished', [
    {
      label: 'mobile e2e',
      sessionId: 'w1',
      cwd: '/home/u/Work/wks',
      lastReply: 'Repaired the suite.',
      result: typeof result === 'string' ? result : JSON.stringify(result, null, 2),
      ...extra,
    },
  ]),
});

describe('<StructuredResultCard> inside a fleet wake', () => {
  it('shows a real result as a card — not as JSON', () => {
    render(<ConversationMessage turn={wake(REAL_RESULT)} />);

    expect(screen.getByText('structured result')).toBeTruthy();
    // The scannable strip: did it land, where, and how much.
    expect(screen.getByTitle(/merged: yes/)).toBeTruthy();
    expect(screen.getByLabelText('Copy commit e124a078')).toBeTruthy();
    expect(screen.getByTitle('tests fixed: 8')).toBeTruthy();

    // The caveat is READABLE without clicking anything — the whole point.
    expect(screen.getByText(/was not run through actionlint/)).toBeTruthy();

    // The evidence is there, verbatim.
    expect(screen.getByText(/22\/22 both times/)).toBeTruthy();
    expect(screen.getByText(/Watch the first live CI run/)).toBeTruthy();

    // Nothing renders as a raw dump.
    expect(screen.queryByText(/"filesChanged":/)).toBeNull();
  });

  it('keeps a field no schema anticipated, labelled from its key', () => {
    render(<ConversationMessage turn={wake(REAL_RESULT)} />);
    expect(screen.getByText('real bugs found')).toBeTruthy();
    // Its single item is a whole paragraph: the head shows, the rest is one
    // click away — the card never stretches to fit it.
    const head = screen.getByText(/mobile\.html fetchConv/);
    expect(head.textContent?.length).toBeLessThan(REAL_RESULT.realBugsFound[0].length);
    fireEvent.click(screen.getByText('more'));
    expect(screen.getByText(/once the chat screen opened/)).toBeTruthy();
  });

  it('leads with the count for a file list and shows the paths on demand', () => {
    render(<ConversationMessage turn={wake(REAL_RESULT)} />);
    const toggle = screen.getByRole('button', { name: /files changed/ });
    expect(within(toggle).getByText('3')).toBeTruthy();
    expect(within(toggle).getByText('files changed')).toBeTruthy();
    expect(screen.queryByText('.github/workflows/ci.yml')).toBeNull();

    fireEvent.click(toggle);
    // …as FileLinks, the app's one path affordance (each wears its
    // destination's icon and resolves against the worker's cwd).
    const link = screen.getByText('.github/workflows/ci.yml');
    expect(link.getAttribute('data-open-target')).toBe('editor');
    expect(screen.getByText('apps/desktop/tests/e2e/mobileClient.test.ts')).toBeTruthy();
  });

  it('renders an unknown nested object as key/value rows', () => {
    render(
      <ConversationMessage
        turn={wake({ secretsCheck: { scanned: 41, findings: 'none' }, decisionTaken: 'kept v1' })}
      />,
    );
    expect(screen.getByText('secrets check')).toBeTruthy();
    expect(screen.getByText('scanned')).toBeTruthy();
    expect(screen.getByText('41')).toBeTruthy();
    expect(screen.getByText('findings')).toBeTruthy();
    expect(screen.getByText('decision taken')).toBeTruthy();
    expect(screen.getByText('kept v1')).toBeTruthy();
  });

  it('renders what it has when a required field never arrived', () => {
    // The dispatch's schema required `commit`; this worker ran out of context
    // and reported neither it nor the file list.
    render(<ConversationMessage turn={wake({ merged: false, caveats: 'ran out of context' })} />);
    expect(screen.getByTitle('merged: no')).toBeTruthy();
    expect(screen.getByText('ran out of context')).toBeTruthy();
    expect(screen.queryByText(/undefined/)).toBeNull();
    expect(screen.queryByLabelText(/Copy commit/)).toBeNull();
  });

  it('says "none" for an empty array instead of showing an empty section', () => {
    render(<ConversationMessage turn={wake({ merged: true, filesChanged: [], caveats: '' })} />);
    expect(screen.getByText('files changed')).toBeTruthy();
    expect(screen.getByText('none')).toBeTruthy();
    // An explicitly empty caveat is an answer, and reads as one.
    expect(screen.getByText('none reported')).toBeTruthy();
  });

  it('collapses a very long array behind its own count', () => {
    const items = Array.from({ length: 30 }, (_, i) => `check number ${i + 1} passed`);
    render(<ConversationMessage turn={wake({ checksRun: items })} />);
    expect(screen.getByText('check number 3 passed')).toBeTruthy();
    expect(screen.queryByText('check number 4 passed')).toBeNull();
    fireEvent.click(screen.getByText('+27 more'));
    expect(screen.getByText('check number 30 passed')).toBeTruthy();
    fireEvent.click(screen.getByText('show fewer'));
    expect(screen.queryByText('check number 30 passed')).toBeNull();
  });

  it('shows the bytes that did arrive when the payload is malformed', () => {
    // What an oversized result actually looks like: the wake truncates the
    // object mid-string, so it is no longer valid JSON.
    const truncated =
      '{\n  "commit": "e124a078",\n  "checksRun": ["npx vitest run (apps\n[truncated: 8192 bytes of validated result]';
    render(<ConversationMessage turn={wake(truncated)} />);
    expect(screen.getByText('no structured result')).toBeTruthy();
    expect(screen.getByText(/arrived truncated — showing it as it arrived/)).toBeTruthy();
    expect(screen.getByText(/\[truncated: 8192 bytes/)).toBeTruthy();
  });

  it('says the contract was missed when no result could be read at all', () => {
    const text = buildFleetMessage('worker-finished', [
      {
        label: 'mobile e2e',
        sessionId: 'w1',
        cwd: '/home/u/Work/wks',
        lastReply: 'Repaired the suite.',
        resultError: 'no `wks-result` block in the worker final message',
      },
    ]);
    render(<ConversationMessage turn={{ role: 'user', content: text }} />);
    expect(screen.getByText('no structured result')).toBeTruthy();
    expect(screen.getByText(/no `wks-result` block/)).toBeTruthy();
    // …and the prose report is still right there beside it.
    fireEvent.click(screen.getByText('last reply'));
    expect(screen.getByText('Repaired the suite.')).toBeTruthy();
  });

  it('adds nothing at all to a wake that carried no result', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'mobile e2e', sessionId: 'w1', cwd: '/home/u/Work/wks', lastReply: 'done' },
    ]);
    render(<ConversationMessage turn={{ role: 'user', content: text }} />);
    expect(screen.queryByText('structured result')).toBeNull();
    expect(screen.queryByText('no structured result')).toBeNull();
  });

  it('renders nothing when handed neither a result nor an error', () => {
    const { container } = render(<StructuredResultCard />);
    expect(container.innerHTML).toBe('');
  });
});
