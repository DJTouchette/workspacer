/**
 * Last-recorded cost/token figures per session, shared by every agent surface.
 *
 * A context rather than props because the readers are scattered — the sidebar
 * card, the Fleet Deck's cards and list, the pane status bar and the Inspector
 * (opened as a pane, a rail and a card expansion) — and threading one more map
 * through all of them would touch far more code than the fix is worth.
 *
 * The default is an EMPTY MAP with no failure recorded, not a throwing hook:
 * SideBar, InspectorCard and SessionStatusBar all render in harnesses and tests
 * with no providers mounted, and that default degrades to exactly the previous
 * behaviour (live figures only, an honest dash when there are none).
 *
 * THE MAP ALONE IS TWO-STATE, AND THERE ARE THREE. A missing entry means "the
 * history DB recorded no figure for this session" — but it means exactly the
 * same thing when the list the map is built from could not be READ at all
 * (`sessions.recent` has no provider on a headless hub, so every entry is
 * missing for a reason that has nothing to do with the sessions). Both render a
 * dash, which is honest either way; what is NOT honest is a surface labelling
 * that dash "no cost recorded for this session" when the truth is that nothing
 * was asked. `unavailable` carries that reason so those labels can tell the two
 * apart. It is the same string `useRecentSessions` already gives the History
 * pane, so the two surfaces cannot disagree about whether we know anything.
 */
import React, { createContext, useContext, useMemo } from 'react';
import type { RecordedUsageBySession } from '../lib/recordedUsage';

export interface RecordedUsageState {
  bySession: RecordedUsageBySession;
  /** Why the recorded-usage source could not be read, or null when it could.
   *  Note it can be set while `bySession` still holds figures: the list poll
   *  keeps its last good answer, so those are real readings that have simply
   *  stopped refreshing. It only qualifies the ABSENCES. */
  unavailable: string | null;
}

const EMPTY: RecordedUsageState = { bySession: {}, unavailable: null };

const RecordedUsageContext = createContext<RecordedUsageState>(EMPTY);

export const RecordedUsageProvider: React.FC<{
  value: RecordedUsageBySession;
  unavailable?: string | null;
  children: React.ReactNode;
}> = ({ value, unavailable = null, children }) => {
  const state = useMemo<RecordedUsageState>(
    () => ({ bySession: value, unavailable }),
    [value, unavailable],
  );
  return <RecordedUsageContext.Provider value={state}>{children}</RecordedUsageContext.Provider>;
};

/** The whole map — for surfaces that render many sessions at once. */
export function useRecordedUsageMap(): RecordedUsageBySession {
  return useContext(RecordedUsageContext).bySession;
}

/** One session's recorded figures, or undefined when nothing was recorded. */
export function useRecordedUsage(sessionId: string | undefined) {
  const { bySession } = useContext(RecordedUsageContext);
  return sessionId ? bySession[sessionId] : undefined;
}

/**
 * Why an absent figure is absent, when the answer is "we could not look".
 *
 * Null means the source answered, so a missing figure really is a figure the
 * history DB never recorded — the only case in which a surface may say so.
 */
export function useRecordedUsageUnavailable(): string | null {
  return useContext(RecordedUsageContext).unavailable;
}

/**
 * The tooltip for a surface's absent cost/token slot, phrased for whichever of
 * the two absences this is. Shared so the Fleet Deck's dash and the agent
 * card's line cannot drift into claiming different things about the same fact.
 */
export function absentUsageTitle(unavailable: string | null): string {
  return unavailable
    ? `Recorded usage could not be read (${unavailable}) — this is what was not available, not a measured $0.00.`
    : 'No usage was ever recorded for this session — not a measured $0.00.';
}
