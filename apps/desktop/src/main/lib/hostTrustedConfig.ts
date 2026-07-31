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
 * Return `partial` without any host-trusted section, leaving the on-disk values
 * alone. Copies rather than deleting in place — the caller still owns the object
 * it passed in.
 */
export function dropHostTrusted<T extends Record<string, unknown>>(partial: T): Partial<T> {
  const found = HOST_TRUSTED_SECTIONS.filter((k) => k in partial);
  if (found.length === 0) return partial;
  console.warn(
    `[hub] config.save: ignoring host-trusted section(s) ${found.join(', ')} from a bus client`,
  );
  const out = { ...partial };
  for (const k of found) delete (out as Record<string, unknown>)[k];
  return out;
}
