/** Native SQLite adapter for shared analytics queries. */
import { database } from './db';
import { SessionHistoryStore } from './sessionHistoryCore';
export * from './sessionHistoryCore';
export const sessionHistory = new SessionHistoryStore(database);
