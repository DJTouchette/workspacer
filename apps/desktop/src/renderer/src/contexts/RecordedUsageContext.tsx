/**
 * Last-recorded cost/token figures per session, shared by every agent surface.
 *
 * A context rather than props because the readers are scattered — the sidebar
 * card, the Fleet Deck's cards and list, the pane status bar and the Inspector
 * (opened as a pane, a rail and a card expansion) — and threading one more map
 * through all of them would touch far more code than the fix is worth.
 *
 * The default is an EMPTY MAP, not a throwing hook: SideBar, InspectorCard and
 * SessionStatusBar all render in harnesses and tests with no providers mounted,
 * and an empty map degrades to exactly the previous behaviour (live figures
 * only, an honest dash when there are none).
 */
import React, { createContext, useContext } from 'react';
import type { RecordedUsageBySession } from '../lib/recordedUsage';

const RecordedUsageContext = createContext<RecordedUsageBySession>({});

export const RecordedUsageProvider: React.FC<{
  value: RecordedUsageBySession;
  children: React.ReactNode;
}> = ({ value, children }) => (
  <RecordedUsageContext.Provider value={value}>{children}</RecordedUsageContext.Provider>
);

/** The whole map — for surfaces that render many sessions at once. */
export function useRecordedUsageMap(): RecordedUsageBySession {
  return useContext(RecordedUsageContext);
}

/** One session's recorded figures, or undefined when nothing was recorded. */
export function useRecordedUsage(sessionId: string | undefined) {
  const map = useContext(RecordedUsageContext);
  return sessionId ? map[sessionId] : undefined;
}
