import * as yaml from 'js-yaml';
import { isDeepStrictEqual } from 'util';

/**
 * Top-level config.yaml keys that belong to a REMOVED feature, so nothing reads
 * them any more — and which config loading would otherwise round-trip forever.
 *
 * `deepMerge(defaults, parsed)` copies every key the parsed file has, known or
 * not, and there is deliberately NO general unknown-key pruning: a key can be
 * absent from the defaults because its feature loads late or lives in a plugin,
 * and deleting those would be config loss. So retirement is spelled ONE KEY AT
 * A TIME, here, when the feature that owned it is gone from the code.
 *
 *  - `supervisor` — the fleet-supervisor ROLE, deleted in the supervisor-removal
 *    series merged at a6ad647d. Its config block (models, summarizerModel,
 *    fullAccess, provider) has no reader left in any of the four runtimes.
 *
 * THE RULE for adding a key here: every reader of it must be gone from the
 * repo, and it must be a TOP-LEVEL key. A nested one is not safe to strip
 * textually (the block scan below anchors on column 0), and a key that still
 * has a reader is a feature being deleted, not an orphan being tidied.
 *
 * Mirrored by `orphanedConfigKeys` in services/hub/cmd/brain/config_orphans.go
 * — both writers of config.yaml have to agree, or whichever one still carries
 * the key writes it straight back on its next save.
 */
export const ORPHANED_CONFIG_KEYS: readonly string[] = ['supervisor'];

export interface OrphanPruneResult {
  /** Orphaned keys that were present, and are now deleted from `parsed`. */
  removed: string[];
  /** The rewritten file text to persist, or null when there is nothing safe
   *  (or nothing) to write. Null with a non-empty `removed` means the in-memory
   *  config is clean but the file was left exactly as it was. */
  text: string | null;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Delete a top-level `key:` block from raw YAML **text**, leaving every other
 * byte — comments, key order, quoting, indentation — untouched.
 *
 * We do not round-trip through js-yaml here on purpose. `yaml.dump` of a parsed
 * document drops every comment and re-orders keys into the defaults' order, so
 * the "tidy up one dead key" change would silently eat a user's annotated
 * config. Text surgery is the only way to keep that promise; the caller
 * re-parses the result and refuses to write unless it means exactly the same
 * thing minus the key (see pruneOrphanedConfigKeys).
 *
 * A block runs from its `key:` line to the next line that is non-blank and
 * unindented. Blank lines inside it, and the blank separator after it, go with
 * it — but only when a real line follows, so a block at end-of-file leaves the
 * file's trailing newline alone. Column 0 is the anchor throughout: a
 * same-named key nested under something else is never touched.
 */
export function stripTopLevelBlock(raw: string, key: string): string {
  const head = new RegExp(`^${escapeRegExp(key)}:(\\s|$)`);
  // split('\n') keeps a CRLF file's '\r' at the end of each line, so joining
  // back gives byte-identical line endings for everything we did not remove.
  const lines = raw.split('\n');
  const out: string[] = [];
  let i = 0;
  while (i < lines.length) {
    if (!head.test(lines[i].replace(/\r$/, ''))) {
      out.push(lines[i]);
      i++;
      continue;
    }
    i++; // the `key:` line itself
    // Consume the block body. Blank lines are held back until we know whether a
    // block line still follows them: trailing blanks before the next key are
    // the block's own separator (drop them), trailing blanks at EOF are the
    // file's ending (keep them).
    let pending: string[] = [];
    while (i < lines.length) {
      const line = lines[i].replace(/\r$/, '');
      if (line.trim() === '') {
        pending.push(lines[i]);
        i++;
        continue;
      }
      if (/^[ \t]/.test(line)) {
        pending = [];
        i++;
        continue;
      }
      break;
    }
    if (i >= lines.length) out.push(...pending); // EOF: keep the file's tail
  }
  return out.join('\n');
}

/**
 * Strip every ORPHANED_CONFIG_KEYS entry from a freshly-parsed config and, when
 * it can be done without changing anything else, hand back the file text to
 * persist.
 *
 * `parsed` is MUTATED (the keys are deleted) whether or not a write is
 * possible, so the running process never carries a dead key even if the file
 * on disk keeps it for now.
 *
 * Idempotent: a config with no orphaned key returns `{ removed: [], text: null }`
 * and the caller writes nothing — which is the common case, so a normal boot
 * touches config.yaml exactly as often as it did before.
 */
export function pruneOrphanedConfigKeys(
  parsed: Record<string, unknown>,
  raw: string,
): OrphanPruneResult {
  const removed = ORPHANED_CONFIG_KEYS.filter((key) =>
    Object.prototype.hasOwnProperty.call(parsed, key),
  );
  if (removed.length === 0) return { removed: [], text: null };

  let text = raw;
  for (const key of removed) text = stripTopLevelBlock(text, key);
  for (const key of removed) delete parsed[key];

  if (text === raw) return { removed, text: null }; // nothing matched textually

  // The safety net. Text surgery on YAML is only allowed to change the ONE
  // thing it claims to: re-read what we produced and require that it means
  // exactly the pruned document. Anything else (a quoted key we did not match,
  // an anchor, a multi-doc file, a block we mis-bounded) leaves the file alone
  // rather than writing a config we cannot prove is equivalent.
  let reparsed: unknown;
  try {
    reparsed = yaml.load(text);
  } catch {
    return { removed, text: null };
  }
  if (!isDeepStrictEqual(reparsed, parsed)) return { removed, text: null };

  return { removed, text };
}
