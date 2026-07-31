/**
 * The saved-session file format version. Single source in TS; the Go brain has
 * its own copy in cmd/brain/stores.go and both are pinned to
 * `contracts/session-schema.json` by a test on each side.
 *
 * A reader accepts a file whose version is absent (written before versioning —
 * fall back to sniffing its shape) or is <= its own. A HIGHER version means a
 * newer build wrote it: the reader must refuse to overwrite it rather than
 * treat the shape it cannot parse as an empty session.
 *
 * Bump this when the shape changes in a way an older build cannot round-trip.
 */
export const SESSION_SCHEMA_VERSION = 1;
