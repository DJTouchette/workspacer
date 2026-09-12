// Native history input for the shared brief-board implementation.
import { sessionHistory } from './sessionHistory';
import { setBoardRecentSessions } from './briefBoardCore';
setBoardRecentSessions((limit) => sessionHistory.recent(limit));
export * from './briefBoardCore';
