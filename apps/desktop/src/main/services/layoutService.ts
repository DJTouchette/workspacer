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
import { getConfigDir } from './configService';
import { slugLayout } from '../lib/fileUtils';

export interface LayoutPane {
  type: string;
  title: string;
  url?: string;
  shell?: string;
  cwd?: string;
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

class LayoutService {
  private ensureDir(): void {
    fs.mkdirSync(layoutsDir(), { recursive: true });
  }

  list(): Layout[] {
    this.ensureDir();
    try {
      return fs.readdirSync(layoutsDir())
        .filter((f) => f.endsWith('.yaml'))
        .map((f) => {
          try { return yaml.load(fs.readFileSync(path.join(layoutsDir(), f), 'utf-8')) as Layout; }
          catch { return null; }
        })
        .filter((l): l is Layout => !!l && Array.isArray(l.agents))
        .sort((a, b) => (b.createdAt || '').localeCompare(a.createdAt || ''));
    } catch {
      return [];
    }
  }

  save(input: { id?: string; name: string; agents: LayoutAgent[] }): Layout {
    this.ensureDir();
    const id = input.id || slug(input.name);
    const layout: Layout = {
      id,
      name: input.name.trim() || id,
      createdAt: new Date().toISOString(),
      agents: input.agents ?? [],
    };
    fs.writeFileSync(path.join(layoutsDir(), `${id}.yaml`), yaml.dump(layout, { lineWidth: -1 }), 'utf-8');
    return layout;
  }

  remove(id: string): void {
    try { fs.unlinkSync(path.join(layoutsDir(), `${slug(id)}.yaml`)); } catch { /* already gone */ }
  }
}

export const layoutService = new LayoutService();
