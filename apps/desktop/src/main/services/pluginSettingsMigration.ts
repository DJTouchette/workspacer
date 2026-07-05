/**
 * One-time migration of legacy, locally-persisted plugin settings to the hub.
 *
 * Before the hub owned plugin settings, the desktop stored each plugin's value
 * overlay in `<configDir>/plugin-settings.json`. The hub is now the single
 * source of truth (defaults + overlay, shared with web/remote), so the local
 * store is obsolete. We migrate lazily and per-plugin: the first time the
 * desktop reads a plugin's settings — at which point that plugin is definitely
 * loaded in the hub, so a write will validate — the caller pushes any legacy
 * overlay to the hub, then clears the local copy. Peek and clear are separate so
 * a failed push leaves the entry for a later read to retry (no data loss). When
 * the last entry is cleared the file is removed entirely.
 */

import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';

type Values = Record<string, unknown>;
type Store = Record<string, Values>;

function legacyFile(): string {
  return path.join(getConfigDir(), 'plugin-settings.json');
}

function readStore(): Store | null {
  try {
    const parsed = JSON.parse(fs.readFileSync(legacyFile(), 'utf-8'));
    return parsed && typeof parsed === 'object' ? (parsed as Store) : null;
  } catch {
    return null; // missing or unreadable → nothing to migrate
  }
}

/** The legacy overlay for a plugin, or null when there's nothing to migrate.
 *  Does not mutate the store — call {@link clearLegacyPluginSettings} only after
 *  the values have been pushed to the hub. */
export function peekLegacyPluginSettings(pluginId: string): Values | null {
  const store = readStore();
  const values = store?.[pluginId];
  if (!values || typeof values !== 'object' || Object.keys(values).length === 0) return null;
  return values;
}

/** Drop a plugin's entry from the legacy store once it's been migrated; removes
 *  the file when it's the last entry. Best-effort — a write failure is ignored,
 *  which at worst re-migrates the (idempotent) overlay on a later read. */
export function clearLegacyPluginSettings(pluginId: string): void {
  const store = readStore();
  if (!store || !(pluginId in store)) return;
  delete store[pluginId];
  try {
    if (Object.keys(store).length === 0) fs.rmSync(legacyFile(), { force: true });
    else fs.writeFileSync(legacyFile(), JSON.stringify(store, null, 2));
  } catch {
    /* best-effort */
  }
}
