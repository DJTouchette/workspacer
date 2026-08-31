/**
 * The wire mapping for claudemon's canonical selection slice.
 *
 * Two properties matter and both are absence-shaped, which is why they need
 * tests rather than a spread: a field nobody has spoken to must come back
 * ABSENT (not 200_000, not null, not 0), and an update that omits a field must
 * not erase what an earlier, richer update supplied.
 */
import { describe, it, expect } from 'vitest';
import {
  applySelectionSlice,
  mergeSelectionSlice,
  readRequestedSelection,
  readResolvedContextWindow,
  readSelectionSlice,
} from './canonicalSelection';

describe('readSelectionSlice — both spellings', () => {
  it('maps claudemon snake_case to camelCase', () => {
    expect(
      readSelectionSlice({
        requested_selection: { model: 'claude-opus-5', context_window: 1_000_000 },
        resolved_context_window: 1_000_000,
      }),
    ).toEqual({
      requestedSelection: { model: 'claude-opus-5', contextWindow: 1_000_000 },
      resolvedContextWindow: 1_000_000,
    });
  });

  // A current brain emits camelCase alongside the snake originals; an older one
  // forwards only claudemon's. One reader, both shapes — that is skew safety.
  it('reads a peer desktop row and a brain row to the same slice', () => {
    const desktop = readSelectionSlice({
      requestedSelection: { model: 'gpt-5-codex', contextWindow: 272_000 },
      resolvedContextWindow: 272_000,
    });
    const brain = readSelectionSlice({
      requested_selection: { model: 'gpt-5-codex', context_window: 272_000 },
      resolved_context_window: 272_000,
    });
    expect(desktop).toEqual(brain);
  });

  it('carries the owner pair verbatim rather than re-deriving it', () => {
    // A legacy marker in the model string is the OWNER's spelling — a receiver
    // does not re-pair it, and the explicit window is what it says it is.
    expect(
      readRequestedSelection({
        requested_selection: { model: 'opus[1m]', context_window: 1_000_000 },
      }),
    ).toEqual({ model: 'opus[1m]', contextWindow: 1_000_000 });
  });

  it('keeps a null window as "no window chosen" rather than inventing one', () => {
    expect(
      readRequestedSelection({ requested_selection: { model: 'opus', context_window: null } }),
    ).toEqual({ model: 'opus', contextWindow: null });
    expect(readRequestedSelection({ requested_selection: { model: 'opus' } })).toEqual({
      model: 'opus',
      contextWindow: null,
    });
  });
});

describe('readSelectionSlice — absence is a fact', () => {
  it('reports absence for a row that carries neither field', () => {
    expect(readSelectionSlice({ sessionId: 's1', mode: 'input' })).toEqual({});
  });

  it('reports absence for a non-row', () => {
    for (const v of [null, undefined, 'x', 7, []]) expect(readSelectionSlice(v)).toEqual({});
  });

  // 0 is how some producers spell "unknown"; it must not become a window.
  it('refuses 0, negatives, floats and non-numbers as a resolved window', () => {
    for (const v of [0, -1, 1.5, '200000', null, {}]) {
      expect(readResolvedContextWindow({ resolved_context_window: v })).toBeUndefined();
    }
  });

  it('refuses a half-read selection rather than reporting part of it', () => {
    expect(
      readRequestedSelection({ requested_selection: { context_window: 200_000 } }),
    ).toBeUndefined();
    expect(
      readRequestedSelection({ requested_selection: { model: '', context_window: 1 } }),
    ).toBeUndefined();
    expect(
      readRequestedSelection({ requested_selection: { model: 'opus', context_window: 0 } }),
    ).toBeUndefined();
  });
});

describe('mergeSelectionSlice — presence-aware', () => {
  const rich = {
    requestedSelection: { model: 'claude-opus-5', contextWindow: 1_000_000 },
    resolvedContextWindow: 1_000_000,
  };

  it('keeps the richer base when the overlay omits both fields', () => {
    expect(mergeSelectionSlice(rich, {})).toEqual(rich);
    expect(mergeSelectionSlice(rich, undefined)).toEqual(rich);
  });

  it('keeps the base field the overlay does not speak to', () => {
    expect(mergeSelectionSlice(rich, { resolvedContextWindow: 200_000 })).toEqual({
      requestedSelection: rich.requestedSelection,
      resolvedContextWindow: 200_000,
    });
  });

  it('lets the overlay supply a field the base never had', () => {
    expect(mergeSelectionSlice({}, rich)).toEqual(rich);
  });

  // The owner's number is stored as given. Occupancy belongs to the raw
  // provider status pair at the DISPLAY seam (busContextLimit), never here.
  it('never rewrites the owner window on occupancy', () => {
    expect(mergeSelectionSlice({ resolvedContextWindow: 200_000 }, {})).toEqual({
      resolvedContextWindow: 200_000,
    });
  });
});

describe('applySelectionSlice', () => {
  it('omits rather than nulls what nobody knows', () => {
    const row: Record<string, unknown> = { sessionId: 's1', resolvedContextWindow: 200_000 };
    applySelectionSlice(row, {});
    expect('resolvedContextWindow' in row).toBe(false);
    expect(JSON.parse(JSON.stringify(row))).toEqual({ sessionId: 's1' });
  });

  it('writes both fields through unchanged', () => {
    const row: Record<string, unknown> = { sessionId: 's1' };
    applySelectionSlice(row, {
      requestedSelection: { model: 'opus', contextWindow: null },
      resolvedContextWindow: 200_000,
    });
    expect(row).toEqual({
      sessionId: 's1',
      requestedSelection: { model: 'opus', contextWindow: null },
      resolvedContextWindow: 200_000,
    });
  });
});
