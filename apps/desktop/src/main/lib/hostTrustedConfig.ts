/**
 * Config sections only the host process may write.
 *
 * Every `config.save` that arrives over the hub bus — a remote/web client, a
 * plugin, or an agent through the MCP facade — is by definition not from the
 * host, so a section listed here is dropped unconditionally before the merge.
 * The desktop's own Settings write goes through configService in-process and is
 * unaffected.
 *
 * `updates` is the whole list today and it earns its place: `updates.channel` is
 * concatenated into the electron-updater feed URL the desktop then downloads and
 * installs from, so one `../` in a channel relocates the updater to somebody
 * else's repo. That is persistent code execution laundered through the app's own
 * update dialog, not a setting a bus caller gets to choose.
 *
 * This is the twin of `dropHostTrusted` in services/hub/cmd/brain/config.go —
 * the Go copy is the one that runs under the default DELEGATE_CATALOG_TO_BRAIN,
 * this one when delegation is off. `contracts/host-trusted-config-cases.json`
 * pins both against the same fixture.
 */

/** Top-level config keys a bus caller may never write. */
export const HOST_TRUSTED_SECTIONS = ['updates'] as const;

/**
 * The same rule at SUB-KEY granularity, dotted from the root. A section-level
 * drop is the wrong tool for these: they live inside sections a bus client
 * legitimately edits, so dropping the whole section would break ordinary
 * settings while keeping it leaves these writable.
 *
 * - `agents.binaries` is the launcher path `managedSpawn.configuredBin()` reads
 *   and hands to the provider as argv[0] for every spawned agent. `config.save`
 *   is not in capspec.PathParam and its name is not path-bearing, so neither
 *   classification detector could see it — while `fs.write` of config.yaml is
 *   refused by the secret gate, `config.save` rewrote the same file by design.
 *   Combined with an `fs.write` over an existing executable inside the caller's
 *   own agent cwd (writeFileSync preserves the 0755 mode), that was arbitrary
 *   host code execution on the next spawn.
 * - `claude.profiles` carries `configDir` (which becomes CLAUDE_CONFIG_DIR, i.e.
 *   the settings.json supplying permissions.allow and hooks) and `extraArgs`
 *   (--dangerously-skip-permissions). A bus caller planting one there is
 *   persistent, and the LOCAL spawn path (ipc.ts) does not scrub it.
 */
export const HOST_TRUSTED_PATHS = ['agents.binaries', 'claude.profiles'] as const;

/**
 * Return `partial` without any host-trusted section, leaving the on-disk values
 * alone. Copies rather than deleting in place — the caller still owns the object
 * it passed in.
 */
export function dropHostTrusted<T extends Record<string, unknown>>(partial: T): Partial<T> {
  const found = HOST_TRUSTED_SECTIONS.filter((k) => k in partial);
  const foundPaths = HOST_TRUSTED_PATHS.map((dotted) => {
    const [section, ...rest] = dotted.split('.');
    const key = rest.join('.');
    const sub = partial[section];
    // Absent, null, or not an object — nothing nested to strip.
    if (typeof sub !== 'object' || sub === null || Array.isArray(sub)) return null;
    return key in (sub as Record<string, unknown>) ? { section, key } : null;
  }).filter((x): x is { section: string; key: string } => x !== null);

  if (found.length === 0 && foundPaths.length === 0) return partial;
  if (found.length > 0) {
    console.warn(
      `[hub] config.save: ignoring host-trusted section(s) ${found.join(', ')} from a bus client`,
    );
  }
  const out = { ...partial };
  for (const k of found) delete (out as Record<string, unknown>)[k];
  for (const { section, key } of foundPaths) {
    console.warn(
      `[hub] config.save: ignoring host-trusted key ${section}.${key} from a bus client`,
    );
    // Copy the section too — the caller still owns the object it passed in.
    const copy = { ...((out as Record<string, unknown>)[section] as Record<string, unknown>) };
    delete copy[key];
    (out as Record<string, unknown>)[section] = copy;
  }
  return out;
}
