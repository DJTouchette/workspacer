export type LibraryScope = 'global' | 'project';
export type LibraryKind = 'prompt' | 'skill';
export type LibraryAction = 'insert' | 'spawn' | 'copy';

/** A reusable prompt or skill stored as a markdown file. Mirrors the main
 *  process's libraryService.LibraryItem. */
export interface LibraryItem {
  id: string;
  scope: LibraryScope;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  action?: LibraryAction;
  body: string;
  path: string;
}

/** Payload for saving/creating an item (id derives from title if omitted). */
export interface LibrarySaveInput {
  scope: LibraryScope;
  id?: string;
  title: string;
  kind: LibraryKind;
  description?: string;
  tags?: string[];
  action?: LibraryAction;
  body: string;
  cwd?: string;
}
