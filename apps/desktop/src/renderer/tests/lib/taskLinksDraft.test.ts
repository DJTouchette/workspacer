import { describe, expect, it } from 'vitest';
import { createTaskLinksDraft, rebaseTaskLinksDraft } from '../../src/lib/taskLinksDraft';
import type { TaskLinks } from '../../../../main/shared/dispatchHistory';

const base = (): TaskLinks => ({
  pullRequest: { number: '1', url: 'https://example.test/pr/1' },
  tickets: [{ id: 'T-1', url: 'https://example.test/ticket' }, { id: 'T-2' }],
  references: [{ label: 'Design', url: 'https://example.test/design' }],
});
const edit = (links: TaskLinks = base()) => createTaskLinksDraft(structuredClone(links));

describe('reference draft three-way merge', () => {
  it('keeps concurrent additions, changes and deletions through repeated rebases', () => {
    let draft = edit();
    draft.links = { ...draft.links, pullRequest: { ...draft.links.pullRequest, number: '2' } };
    const current = base();
    current.tickets = [{ id: 'EXTERNAL-1' }];
    current.references![0].url = 'https://example.test/new-design';
    draft = rebaseTaskLinksDraft(draft, current);
    expect(draft.links).toEqual({
      ...current,
      pullRequest: { ...current.pullRequest, number: '2' },
    });
    const next = { ...current, tickets: [...current.tickets, { id: 'EXTERNAL-2' }] };
    draft = rebaseTaskLinksDraft(draft, next);
    expect(draft.links.tickets).toEqual(next.tickets);
    expect(draft.links.pullRequest?.number).toBe('2');
  });

  it('honors explicit removals and does not resurrect untouched concurrent deletions', () => {
    const draft = edit();
    draft.links = {
      ...draft.links,
      pullRequest: undefined,
      tickets: [draft.links.tickets![1]],
      references: [],
    };
    draft.ticketOrigins.splice(0, 1);
    draft.referenceOrigins = [];
    const current = base();
    current.tickets = [current.tickets![0], { id: 'EXTERNAL-1' }];
    expect(rebaseTaskLinksDraft(draft, current).links).toEqual({
      tickets: [{ id: 'EXTERNAL-1' }],
      references: [],
    });
  });

  it('merges PR number and URL independently, including an explicit field removal', () => {
    const draft = edit();
    draft.links = {
      ...draft.links,
      pullRequest: { number: undefined, url: draft.links.pullRequest!.url },
    };
    const current = base();
    current.pullRequest!.url = 'https://example.test/new-pr';
    expect(rebaseTaskLinksDraft(draft, current).links.pullRequest).toEqual({
      url: current.pullRequest!.url,
    });
  });

  it.each(['tickets', 'references'] as const)(
    'tracks %s identity edits across URL changes and repeated races',
    (kind) => {
      const draft = edit();
      if (kind === 'tickets')
        draft.links = {
          ...draft.links,
          tickets: [{ ...draft.links.tickets![0], id: 'Renamed' }, draft.links.tickets![1]],
        };
      else
        draft.links = {
          ...draft.links,
          references: [{ ...draft.links.references![0], label: 'Renamed' }],
        };
      const current = base();
      current[kind]![0].url = 'https://example.test/changed';
      const first = rebaseTaskLinksDraft(draft, current);
      const next = structuredClone(current);
      next[kind]![0].url = 'https://example.test/changed-again';
      const result = rebaseTaskLinksDraft(first, next);
      expect(result.links[kind]![0]).toEqual({
        [kind === 'tickets' ? 'id' : 'label']: 'Renamed',
        url: next[kind]![0].url,
      });
    },
  );

  it.each(['number', 'url'] as const)(
    'rejects conflicting PR %s edits without mutating the draft',
    (field) => {
      const draft = edit();
      draft.links = {
        ...draft.links,
        pullRequest: {
          ...draft.links.pullRequest,
          [field]: field === 'number' ? '2' : 'https://example.test/mine',
        },
      };
      const before = structuredClone(draft);
      const current = base();
      current.pullRequest![field] = field === 'number' ? '3' : 'https://example.test/theirs';
      expect(() => rebaseTaskLinksDraft(draft, current)).toThrow(`Both edits changed PR ${field}`);
      expect(draft).toEqual(before);
      // Choosing the current value explicitly resolves the collision.
      draft.links.pullRequest![field] = current.pullRequest![field];
      expect(rebaseTaskLinksDraft(draft, current).links.pullRequest).toEqual(current.pullRequest);
    },
  );

  it.each(['tickets', 'references'] as const)(
    'rejects %s URL collisions and delete-versus-edit',
    (kind) => {
      const draft = edit();
      draft.links = structuredClone(draft.links);
      draft.links[kind]![0].url = 'https://example.test/mine';
      const current = base();
      current[kind]![0].url = 'https://example.test/theirs';
      expect(() => rebaseTaskLinksDraft(draft, current)).toThrow('Both edits changed');
      current[kind] = [];
      expect(() => rebaseTaskLinksDraft(draft, current)).toThrow('removed from the current task');
      const removed = edit();
      removed.links = { ...removed.links, [kind]: [] };
      removed[kind === 'tickets' ? 'ticketOrigins' : 'referenceOrigins'] = [];
      expect(() => rebaseTaskLinksDraft(removed, { ...base(), [kind]: draft.links[kind] })).toThrow(
        'removed in your draft',
      );
    },
  );

  it('matches identities case-insensitively and rejects rename collisions', () => {
    const draft = edit();
    draft.links = {
      ...draft.links,
      tickets: [
        { ...draft.links.tickets![0], url: 'https://example.test/mine' },
        draft.links.tickets![1],
      ],
    };
    const current = base();
    current.tickets![0].id = 't-1';
    expect(rebaseTaskLinksDraft(draft, current).links.tickets![0]).toEqual({
      id: 't-1',
      url: 'https://example.test/mine',
    });
    draft.links.tickets![0].id = 'T-2';
    expect(() => rebaseTaskLinksDraft(draft, current)).toThrow('Duplicate reference label');
  });

  it('retains concurrent PR additions and reconciles compatible new identities', () => {
    const draft = edit({});
    draft.links = { tickets: [{ id: 'NEW' }] };
    draft.ticketOrigins = [undefined];
    const current: TaskLinks = {
      pullRequest: { number: '5' },
      tickets: [{ id: 'NEW', url: 'https://example.test/new' }],
    };
    expect(rebaseTaskLinksDraft(draft, current).links).toEqual({ ...current, references: [] });
    draft.links.tickets![0].url = 'https://example.test/conflict';
    expect(() => rebaseTaskLinksDraft(draft, current)).toThrow('Both edits changed');
  });
});
