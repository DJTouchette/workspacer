/**
 * The History pane over permanent history (`sessions.history`,
 * docs/session-roster.md §5): every session, paged; hidden ones say so and can
 * be unhidden; Delete removes a stopped session for good, but only after an
 * inline confirmation; and a backend without the method keeps the old list.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent, waitFor, act } from '@testing-library/react';
import SessionsPane from '../src/panes/SessionsPane';
import type { HistoryAgentSession, RecentAgentSession } from '../../main/shared/ipcTypes';

const DIR = '/home/u/Work/demo';

vi.mock('../src/contexts/ConfigContext', () => ({
  useConfigContext: () => ({ config: { projects: { [DIR]: {} } } }),
}));
vi.mock('../src/hooks/useProjectSessions', () => ({
  useProjectSessions: () => ({ byDir: {}, loading: false }),
}));
vi.mock('../src/hooks/useSessionAnalytics', () => ({
  useSessionAnalytics: () => ({
    summary: null,
    bySessionId: {},
    unrecordedSessions: 0,
    unrecordedComplete: true,
    loading: false,
    unavailable: null,
    refresh: () => {},
  }),
}));

const hist = (over: Partial<HistoryAgentSession> = {}): HistoryAgentSession => ({
  sessionId: 'h1',
  provider: 'claude',
  cwd: DIR,
  mode: 'stopped',
  transport: 'stream',
  archived: false,
  updatedAt: Date.now() - 60_000,
  startedAt: 0,
  name: '',
  title: 'history row',
  model: '',
  hidden: false,
  onSidebar: false,
  transcriptCopy: true,
  ...over,
});

type Api = Record<string, unknown>;
const api = (): Api => window.electronAPI as unknown as Api;
const added: string[] = [];
function stub(name: string, fn: unknown): void {
  api()[name] = fn;
  added.push(name);
}

beforeEach(() => {
  added.length = 0;
});
afterEach(() => {
  cleanup();
  for (const name of added) delete api()[name];
});

const row = (id: string) => document.querySelector(`[data-history-row="${id}"]`) as HTMLElement;

describe('SessionsPane — permanent history', () => {
  it('lists history pages and loads older ones on demand', async () => {
    const listSessionHistory = vi
      .fn()
      .mockResolvedValueOnce({
        sessions: [hist({ sessionId: 'new', title: 'newest' })],
        next: 'c1',
      })
      .mockResolvedValueOnce({ sessions: [hist({ sessionId: 'old', title: 'oldest' })] });
    stub('listSessionHistory', listSessionHistory);
    render(<SessionsPane sessions={[]} />);
    await screen.findByText('newest');
    expect(listSessionHistory).toHaveBeenLastCalledWith({ limit: 50 });
    fireEvent.click(screen.getByText('Load older sessions'));
    await screen.findByText('oldest');
    expect(listSessionHistory).toHaveBeenLastCalledWith({ limit: 50, cursor: 'c1' });
    await waitFor(() => expect(screen.queryByText('Load older sessions')).toBeNull());
  });

  it('shows hidden sessions with a Hidden badge and unhides them through the shared archive', async () => {
    stub(
      'listSessionHistory',
      vi
        .fn()
        .mockResolvedValue({ sessions: [hist({ sessionId: 'h', title: 'tucked', hidden: true })] }),
    );
    const sessionArchiveSet = vi.fn().mockResolvedValue({ version: 2, archived: {} });
    stub('sessionArchiveSet', sessionArchiveSet);
    render(<SessionsPane sessions={[]} />);
    await screen.findByText('tucked');
    expect(row('h').textContent).toContain('Hidden');
    fireEvent.click(screen.getByText('Unhide'));
    await waitFor(() => expect(sessionArchiveSet).toHaveBeenCalledWith('h', false));
    await waitFor(() => expect(row('h').textContent).not.toContain('Hidden'));
  });

  it('deletes from history only after an inline confirmation, without resuming the row', async () => {
    stub('listSessionHistory', vi.fn().mockResolvedValue({ sessions: [hist({ sessionId: 'd' })] }));
    const forgetSession = vi.fn().mockResolvedValue({ ok: true, sessionId: 'd' });
    stub('forgetSession', forgetSession);
    const onResume = vi.fn();
    render(<SessionsPane sessions={[]} onResume={onResume} />);
    await screen.findByText('history row');
    fireEvent.click(screen.getByText('Delete'));
    expect(forgetSession).not.toHaveBeenCalled();
    expect(screen.getByText('Delete permanently?')).toBeTruthy();
    fireEvent.click(screen.getByText('Cancel'));
    expect(screen.queryByText('Delete permanently?')).toBeNull();
    fireEvent.click(screen.getByText('Delete'));
    fireEvent.click(screen.getByText('Confirm delete'));
    await waitFor(() => expect(forgetSession).toHaveBeenCalledWith('d'));
    await waitFor(() => expect(screen.queryByText('history row')).toBeNull());
    expect(onResume).not.toHaveBeenCalled();
  });

  it('keeps the row and says why when the delete is refused', async () => {
    stub('listSessionHistory', vi.fn().mockResolvedValue({ sessions: [hist({ sessionId: 'd' })] }));
    stub('forgetSession', vi.fn().mockRejectedValue(new Error('session is live; stop it first')));
    render(<SessionsPane sessions={[]} />);
    await screen.findByText('history row');
    fireEvent.click(screen.getByText('Delete'));
    fireEvent.click(screen.getByText('Confirm delete'));
    expect((await screen.findByRole('alert')).textContent).toContain('session is live');
    expect(screen.getByText('history row')).toBeTruthy();
  });

  it('lists live and in-workspace sessions without offering resume or delete', async () => {
    stub(
      'listSessionHistory',
      vi.fn().mockResolvedValue({
        sessions: [
          hist({ sessionId: 'live', title: 'running now', mode: 'responding' }),
          hist({ sessionId: 'card', title: 'has a card' }),
        ],
      }),
    );
    stub('forgetSession', vi.fn());
    const onResume = vi.fn();
    render(<SessionsPane sessions={[]} excludeSessionIds={['live', 'card']} onResume={onResume} />);
    await screen.findByText('running now');
    expect(row('live').textContent).toContain('Live');
    expect(row('card').textContent).toContain('On sidebar');
    fireEvent.click(row('live'));
    fireEvent.click(row('card'));
    expect(onResume).not.toHaveBeenCalled();
    const liveDelete = row('live').querySelector('button') as HTMLButtonElement;
    expect(liveDelete.disabled).toBe(true);
  });

  it('searches on the server and keeps rows it matched by first prompt', async () => {
    const listSessionHistory = vi
      .fn()
      .mockResolvedValueOnce({ sessions: [hist({ sessionId: 'a', title: 'alpha' })] })
      .mockResolvedValueOnce({
        sessions: [
          hist({ sessionId: 'b', title: 'unrelated label', firstPrompt: 'zebra stripes' }),
        ],
      });
    stub('listSessionHistory', listSessionHistory);
    render(<SessionsPane sessions={[]} />);
    await screen.findByText('alpha');
    fireEvent.change(screen.getByPlaceholderText(/Search by message/), {
      target: { value: 'zebra' },
    });
    await screen.findByText('unrelated label');
    expect(listSessionHistory).toHaveBeenLastCalledWith({ limit: 50, query: 'zebra' });
  });

  it('falls back to the recent list when the backend has no history method or it fails', async () => {
    const recent = [{ ...hist({ sessionId: 'r', title: 'from recent' }) }] as RecentAgentSession[];
    const { unmount } = render(<SessionsPane sessions={recent} />);
    expect(screen.getByText('from recent')).toBeTruthy();
    unmount();

    stub('listSessionHistory', vi.fn().mockRejectedValue(new Error('no provider')));
    render(<SessionsPane sessions={recent} />);
    await act(async () => {});
    expect(screen.getByText('from recent')).toBeTruthy();
  });
});
