/**
 * Layout templates: named, reusable arrangements of project directories and the
 * panes/tabs open in each. Unlike a saved session (a live snapshot tied to
 * running Claude session ids), a layout is a *starting point* — restoring it
 * spawns fresh agents for its directories and reopens the panes.
 *
 * Stored one YAML file per layout under <configDir>/layouts/.
 */
import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { asString, byteCompare } from '../lib/providerParity';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { slugLayout } from '../lib/fileUtils';
import { resolveStoreEntry } from '../lib/pathConfinement';

export interface LayoutPane {
  type: string;
  title: string;
  url?: string;
  shell?: string;
  cwd?: string;
  /** Plugin panes: the contributing plugin's id (so a restored pane can re-mint
   *  its agent-cwd-scoped token). */
  pluginId?: string;
}

export interface LayoutTab {
  title: string;
  panes: LayoutPane[];
}

export interface LayoutAgent {
  name: string;
  cwd: string;
  model?: string;
  tabs: LayoutTab[];
}

export interface Layout {
  id: string;
  name: string;
  createdAt: string;
  agents: LayoutAgent[];
}

function layoutsDir(): string {
  return path.join(getConfigDir(), 'layouts');
}

const slug = slugLayout;

/**
 * Resolve the file for a layout id. `layouts.save` is reachable from the bus
 * (web / remote / MCP) and either provider may answer it — the Go brain is the
 * default — so this mirrors layoutFilePath in cmd/brain/stores.go rule for rule,
 * or the same call succeeds here and errors there depending on a delegation flag.
 *
 * An id carrying a separator or a `..` is an escape attempt rather than an
 * untidy name (taken verbatim, `../config` wrote over the user's config.yaml,
 * and since the clobbered file still parses there is no .broken-* backup to
 * recover themes/keybindings/budgets from), so it is refused rather than quietly
 * slugged onto some unrelated layout's file. Everything else is slugged, which
 * is what remove has always done — save used the raw id, so the two disagreed on
 * the filename for any id that wasn't already a slug.
 */
function layoutFilePath(id: string): string {
  if (/[\\/]/.test(id) || id.includes('..')) {
    throw new Error('layout id must not contain a path separator');
  }
  const full = path.join(layoutsDir(), `${slug(id)}.yaml`);
  // Belt and braces: the reject above plus slugging already strip separators,
  // but re-assert the joined path really is a direct child of the layouts dir so
  // a future change to the slug charset can't quietly reopen the escape.
  if (path.dirname(path.resolve(full)) !== path.resolve(layoutsDir())) {
    throw new Error('layout id resolves outside the layouts directory');
  }
  return full;
}

class LayoutService {
  private ensureDir(): void {
    fs.mkdirSync(layoutsDir(), { recursive: true });
  }

  list(): Layout[] {
    this.ensureDir();
    try {
      return (
        fs
          .readdirSync(layoutsDir())
          .filter((f) => f.endsWith('.yaml'))
          .map((f) => {
            // A symlink named like a layout is a legal directory entry, and the
            // layouts dir is one of the few a bus caller can write into — so the
            // entry is confined before it is read, and the canonical path is what
            // gets opened. Twin: cmd/brain/stores.go storeEntryPath.
            const full = resolveStoreEntry(layoutsDir(), f);
            if (full === null) return null;
            try {
              return yaml.load(fs.readFileSync(full, 'utf-8')) as Layout;
            } catch {
              return null;
            }
          })
          .filter((l): l is Layout => !!l && Array.isArray(l.agents))
          // Byte-wise over a COERCED scalar, matching the Go twin's
          // `str(out[i]["createdAt"]) > str(out[j]["createdAt"])`. localeCompare is
          // a method, so `createdAt: 5` — or an unquoted ISO date, which js-yaml 4
          // parses to a Date — threw inside the comparator as soon as V8's sort put
          // that row in the `b` position, and the catch below turned the throw into
          // an EMPTY LIST: every well-formed layout vanished with it, while the
          // brain listed them all. <configDir>/layouts is a configStoreRoot, so
          // writing that file is an ordinary permitted fs.write.
          .sort((a, b) => byteCompare(asString(b.createdAt), asString(a.createdAt)))
      );
    } catch {
      return [];
    }
  }

  save(input: { id?: string; name: string; agents: LayoutAgent[] }): Layout {
    this.ensureDir();
    // Only an explicitly supplied id faces the separator rejection; a *name* is
    // user prose that may legitimately contain a slash, so it becomes an id by
    // slugging first — the same order as the brain's saveLayout.
    const rawId = input.id || slug(input.name);
    const file = layoutFilePath(rawId);
    // The stored id is the slug we actually wrote under, so a later remove() —
    // which re-slugs too — finds this file.
    const id = slug(rawId);
    const layout: Layout = {
      id,
      name: input.name.trim() || id,
      createdAt: new Date().toISOString(),
      agents: input.agents ?? [],
    };
    atomicWriteFileSync(file, yaml.dump(layout, { lineWidth: -1 }));
    return layout;
  }

  remove(id: string): void {
    let file: string;
    try {
      file = layoutFilePath(id);
    } catch {
      return; // as in the brain: a separator-carrying id names no layout to remove
    }
    try {
      fs.unlinkSync(file);
    } catch {
      /* already gone */
    }
  }
}

export const layoutService = new LayoutService();
