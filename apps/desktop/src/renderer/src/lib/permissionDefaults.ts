/**
 * The spawn-time permission default, read and written as ONE value.
 *
 * Two config keys spell the same decision:
 *   - claude.defaultPermissionMode — the mode the spawn dialog pre-selects
 *   - claude.skipPermissionsDefault — the bypass spelling of the same thing
 * The spawn dialog pre-selects the mode and then forces bypass when the skip
 * flag is on; the hub facade resolves an OMITTED spawn_agent skipPermissions
 * from EITHER (cmd/mcp configSkipPermissionsDefault). Until now neither was in
 * the Settings UI at all — the only way to set them was hand-editing
 * config.yaml, which is how "full access is on but workers still prompt" became
 * undiagnosable.
 *
 * Surfacing them as two independent controls would let the operator leave them
 * contradicting each other, so Settings drives both through these helpers:
 * "Full access" sets both, every other mode clears the skip flag.
 */

/** Permission modes offered as the spawn default. Same ids as
 *  PROVIDER_CAPS.claude (the spawn dialog normalizes them for the managed
 *  providers), so the picker and the dialog cannot disagree about the names. */
export const PERMISSION_MODE_DEFAULTS: { value: string; label: string }[] = [
  { value: '', label: 'Ask to approve' },
  { value: 'acceptEdits', label: 'Accept edits' },
  { value: 'plan', label: 'Plan mode' },
  { value: 'bypassPermissions', label: 'Full access' },
];

/** What the two keys currently mean, as one mode id. The skip flag wins — it is
 *  the stronger claim, and it is what the spawn dialog itself honours last.
 *  An unrecognized defaultPermissionMode reads as '' (approvals on): the same
 *  fail-closed direction as permissionModeMeansBypass. */
export function currentPermissionModeDefault(claude?: {
  defaultPermissionMode?: string;
  skipPermissionsDefault?: boolean;
}): string {
  if (claude?.skipPermissionsDefault === true) return 'bypassPermissions';
  const mode = claude?.defaultPermissionMode ?? '';
  return PERMISSION_MODE_DEFAULTS.some((m) => m.value === mode) ? mode : '';
}

/** The config patch that sets the default to `mode`, keeping both keys in
 *  agreement in both directions. */
export function permissionModeDefaultPatch(mode: string): {
  defaultPermissionMode: string;
  skipPermissionsDefault: boolean;
} {
  return {
    defaultPermissionMode: mode,
    skipPermissionsDefault: mode === 'bypassPermissions',
  };
}
