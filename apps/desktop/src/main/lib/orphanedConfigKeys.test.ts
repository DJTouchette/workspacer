/**
 * Retiring ONE orphaned config key.
 *
 * The supervisor ROLE was deleted (series merged at a6ad647d) but existing
 * config.yaml files still carry its block, and config loading deep-merges with
 * no unknown-key pruning — so it round-trips forever. These pin the two things
 * that make removing it safe to ship against a user's real file: the blast
 * radius is exactly one top-level key, and everything else (comments, key
 * order, formatting, trailing newline) survives byte for byte.
 *
 * Twin: services/hub/cmd/brain/config_orphans_test.go.
 */
import { describe, it, expect } from 'vitest';
import * as yaml from 'js-yaml';
import {
  ORPHANED_CONFIG_KEYS,
  pruneOrphanedConfigKeys,
  stripTopLevelBlock,
} from './orphanedConfigKeys';

/** Parse + prune the way loadFromDisk does, so the cases read like real files. */
function prune(raw: string) {
  const parsed = (yaml.load(raw) ?? {}) as Record<string, unknown>;
  const result = pruneOrphanedConfigKeys(parsed, raw);
  return { ...result, parsed };
}

describe('the retired-key list', () => {
  it('is exactly the supervisor block — nothing else is up for deletion', () => {
    expect([...ORPHANED_CONFIG_KEYS]).toEqual(['supervisor']);
  });
});

describe('a config WITH the supervisor block', () => {
  const raw = ['ui:', '  theme: nord', 'supervisor:', '  provider: claude', ''].join('\n');

  it('removes it from the parsed config and from the file text', () => {
    const { removed, text, parsed } = prune(raw);
    expect(removed).toEqual(['supervisor']);
    expect(parsed).toEqual({ ui: { theme: 'nord' } });
    expect(text).toBe('ui:\n  theme: nord\n');
  });

  it('is idempotent — a second pass over the result is a no-op', () => {
    const once = prune(raw).text as string;
    const twice = prune(once);
    expect(twice.removed).toEqual([]);
    expect(twice.text).toBeNull();
  });
});

describe('a config WITHOUT the supervisor block', () => {
  it('is left completely alone, with no error (the new-install case)', () => {
    const raw = 'ui:\n  theme: nord\nclaude:\n  defaultModel: opus\n';
    const { removed, text, parsed } = prune(raw);
    expect(removed).toEqual([]);
    expect(text).toBeNull(); // null = the caller writes nothing at all
    expect(parsed).toEqual({ ui: { theme: 'nord' }, claude: { defaultModel: 'opus' } });
  });

  it('does not choke on an empty config', () => {
    expect(prune('')).toMatchObject({ removed: [], text: null });
  });

  it('does not choke on a comments-only config', () => {
    expect(prune('# nothing here yet\n')).toMatchObject({ removed: [], text: null });
  });
});

describe('a config with the block, surrounding keys AND comments', () => {
  // A hand-annotated file: comments above, between and inside blocks, a
  // deliberately non-default key order, quoting, and a blank-line rhythm.
  const raw = [
    '# my workspacer config — hand edited, do not clobber',
    'ui:',
    '  theme: nord # the only one I can read',
    '  # tried 15, too small',
    '  fontSize: 16',
    '',
    '# left over from the fleet supervisor experiment',
    'supervisor:',
    '  provider: claude',
    '  # cheap worker for digests',
    '  summarizerModel: haiku',
    '',
    '  models:',
    '    coordinator: opus',
    '',
    'claude:',
    "  defaultModel: 'opus'",
    '',
    '# projects last on purpose',
    'projects:',
    '  /home/u/proj:',
    '    label: Proj',
    '',
  ].join('\n');

  const { removed, text, parsed } = prune(raw);

  it('removes only the supervisor key', () => {
    expect(removed).toEqual(['supervisor']);
    expect(Object.keys(parsed)).toEqual(['ui', 'claude', 'projects']);
  });

  it('leaves every other line byte for byte, comments and order included', () => {
    // The expected file is the input with exactly the block's own lines cut:
    // the header comment stays, the comment ABOVE the block stays (it is not
    // part of the block), the trailing blank separator goes with it.
    expect(text).toBe(
      [
        '# my workspacer config — hand edited, do not clobber',
        'ui:',
        '  theme: nord # the only one I can read',
        '  # tried 15, too small',
        '  fontSize: 16',
        '',
        '# left over from the fleet supervisor experiment',
        'claude:',
        "  defaultModel: 'opus'",
        '',
        '# projects last on purpose',
        'projects:',
        '  /home/u/proj:',
        '    label: Proj',
        '',
      ].join('\n'),
    );
  });

  it('means the same document minus the key when re-parsed', () => {
    expect(yaml.load(text as string)).toEqual(parsed);
  });

  it('keeps every surviving line as an untouched substring of the original', () => {
    for (const line of (text as string).split('\n')) {
      expect(raw.split('\n')).toContain(line);
    }
  });
});

describe('column 0 is the anchor', () => {
  it('never touches a NESTED key that happens to be called supervisor', () => {
    const raw = [
      'agents:',
      '  supervisor:',
      '    provider: claude',
      'ui:',
      '  theme: nord',
      '',
    ].join('\n');
    const { removed, text } = prune(raw);
    expect(removed).toEqual([]);
    expect(text).toBeNull();
  });

  it('never touches a key that merely starts with the name', () => {
    const raw = 'supervisorLoop:\n  enabled: true\n';
    expect(stripTopLevelBlock(raw, 'supervisor')).toBe(raw);
    expect(prune(raw).removed).toEqual([]);
  });
});

describe('file-shape edge cases', () => {
  it('keeps the trailing newline when the block ends the file', () => {
    const { text } = prune('ui:\n  theme: nord\nsupervisor:\n  provider: claude\n');
    expect(text).toBe('ui:\n  theme: nord\n');
  });

  it('keeps a file with no trailing newline as one', () => {
    const { text } = prune('ui:\n  theme: nord\nsupervisor:\n  provider: claude');
    expect(text).toBe('ui:\n  theme: nord');
  });

  it('removes a block that is the FIRST key', () => {
    const { text } = prune('supervisor:\n  provider: claude\nui:\n  theme: nord\n');
    expect(text).toBe('ui:\n  theme: nord\n');
  });

  it('removes an empty (null-valued) block', () => {
    const { removed, text, parsed } = prune('supervisor:\nui:\n  theme: nord\n');
    expect(removed).toEqual(['supervisor']);
    expect(parsed).toEqual({ ui: { theme: 'nord' } });
    expect(text).toBe('ui:\n  theme: nord\n');
  });

  it('removes a flow-style block on one line', () => {
    const { removed, text } = prune('supervisor: { provider: claude }\nui:\n  theme: nord\n');
    expect(removed).toEqual(['supervisor']);
    expect(text).toBe('ui:\n  theme: nord\n');
  });

  it('preserves CRLF line endings on the lines it keeps', () => {
    const { text } = prune('ui:\r\n  theme: nord\r\nsupervisor:\r\n  provider: claude\r\n');
    expect(text).toBe('ui:\r\n  theme: nord\r\n');
  });

  it('refuses to write when the block cannot be matched textually', () => {
    // A quoted key parses to `supervisor` but the column-0 scan does not match
    // it. In-memory is cleaned; the FILE is left exactly as it was.
    const raw = '"supervisor":\n  provider: claude\nui:\n  theme: nord\n';
    const { removed, text, parsed } = prune(raw);
    expect(removed).toEqual(['supervisor']);
    expect(text).toBeNull();
    expect(parsed).toEqual({ ui: { theme: 'nord' } });
  });

  it('refuses to write when the file would be emptied outright', () => {
    // Nothing left but the block: the result parses to null, not {}, so the
    // equivalence check fails and the file is not touched.
    const { removed, text } = prune('supervisor:\n  provider: claude\n');
    expect(removed).toEqual(['supervisor']);
    expect(text).toBeNull();
  });
});
