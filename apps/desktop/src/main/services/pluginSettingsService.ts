/**
 * Persists per-plugin settings values — the configurable options a plugin
 * declares in its manifest (`settings`). We store only the *values* the user has
 * set, keyed by plugin id; a plugin applies its own declared defaults for
 * anything unset, so this file stays a small overlay rather than a full mirror.
 *
 *   <configDir>/plugin-settings.json  →  { "<pluginId>": { "<key>": value, … } }
 *
 * The renderer's Settings UI reads/writes these; the plugin-settings bridge
 * (see preload/ipc) injects the current values into each plugin's webview.
 */

import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';

type Values = Record<string, unknown>;
type Store = Record<string, Values>;

function file(): string {
  return path.join(getConfigDir(), 'plugin-settings.json');
}

function readStore(): Store {
  try {
    const raw = fs.readFileSync(file(), 'utf-8');
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as Store) : {};
  } catch {
    return {};
  }
}

function writeStore(store: Store): void {
  try {
    fs.mkdirSync(path.dirname(file()), { recursive: true });
    fs.writeFileSync(file(), JSON.stringify(store, null, 2));
  } catch (err) {
    console.error('[pluginSettings] write failed:', err);
  }
}

export const pluginSettings = {
  /** The saved values for a plugin (empty object if none). Defaults are the
   *  plugin's responsibility, applied on top of these. */
  get(pluginId: string): Values {
    return readStore()[pluginId] ?? {};
  },

  /** Merge `values` into a plugin's saved settings and persist. Returns the
   *  merged result. A value of `null` deletes that key (revert to default). */
  set(pluginId: string, values: Values): Values {
    const store = readStore();
    const merged: Values = { ...(store[pluginId] ?? {}) };
    for (const [k, v] of Object.entries(values)) {
      if (v === null) delete merged[k];
      else merged[k] = v;
    }
    store[pluginId] = merged;
    writeStore(store);
    return merged;
  },
};
