/**
 * The library's kind vocabulary, as a VALUE.
 *
 * It lives here rather than in libraryService because `library.list` validates a
 * caller-supplied `kind` filter against it, and hubCapabilities must be able to
 * read the list in suites that mock the whole libraryService module away — a
 * vocabulary that vanishes under a mock is one the capability crashes without.
 *
 * TWIN: services/hub/cmd/brain/library.go `libraryKinds`, and the renderer's own
 * LibraryKind union (renderer/src/types/library.ts).
 */
export const LIBRARY_KINDS = ['prompt', 'skill', 'agent', 'mcp', 'command', 'dispatch'] as const;

/** Derived from the value above, so the runtime list a filter is checked against
 *  and the compile-time union can never disagree. */
export type LibraryKind = (typeof LIBRARY_KINDS)[number];
