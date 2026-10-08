// Projects as the hub knows them (native projects.rs): the shared config.yaml
// `projects` map (keyed by normalized directory; `favourite` = pinned,
// `lastOpened` epoch ms), the legacy `directories.favourites/recent` lists
// (read, never written), and the folders the live fleet works in.
//
// `projects` is replaced WHOLESALE by config.save, so every write is built
// from a fresh config.get and verified against what the hub returns — a save
// that could not take the config lock answers with the old config.
import { call } from './bus.js';
import { sessions, cfg, loadConfig, isLive } from './store.js';
import { basename, hash } from './util.js';

export function projectKey(cwd) {
  const key = String(cwd || '').trim().replace(/\\/g, '/');
  const trimmed = key.replace(/\/+$/, '');
  if (!trimmed) return key ? '/' : key;
  if (trimmed.length === 2 && trimmed.endsWith(':') && key.length > 2) return trimmed + '/';
  return trimmed;
}
const windowsShaped = (p) => p.startsWith('//') || /^[a-z]:/i.test(p);
export function sameDir(a, b) {
  const x = projectKey(a), y = projectKey(b);
  return x === y || (windowsShaped(x) && x.toLowerCase() === y.toLowerCase());
}
export const absoluteDir = (p) => /^(\/|\\\\|[a-z]:[/\\])/i.test(String(p || '').trim()) && !String(p).includes('\0');

const PALETTE = ['#6b8afd', '#c084fc', '#f472b6', '#fb923c', '#2dd4bf', '#38bdf8', '#a3a3f5', '#e879a6'];
function fnv(s) { let h = 0x811c9dc5; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193); } return h >>> 0; }
export function initials(name) {
  const words = String(name || '').split(/[\s._-]+/).filter(Boolean);
  if (!words.length) return '?';
  if (words.length === 1) { const c = words[0].match(/^([a-z]+)([A-Z][a-z]*)/); return c ? (c[1][0] + c[2][0]).toUpperCase() : words[0].slice(0, 2).toUpperCase(); }
  return (words[0][0] + words[1][0]).toUpperCase();
}

/** Every project, pinned first then most recently used. */
export function projectList() {
  const map = (cfg && cfg.projects && typeof cfg.projects === 'object') ? cfg.projects : {};
  const legacyFav = ((cfg && cfg.directories && (cfg.directories.favourites || cfg.directories.favorites)) || []).map(projectKey);
  const legacyRecent = ((cfg && cfg.directories && cfg.directories.recent) || []).map(projectKey);
  const out = new Map();
  const add = (dir, patch) => {
    if (!dir || !absoluteDir(dir)) return;
    const key = projectKey(dir);
    const existing = [...out.keys()].find((k) => sameDir(k, key)) || key;
    const cur = out.get(existing) || { path: existing, label: '', pinned: false, lastOpened: 0, running: 0, color: '', icon: '' };
    out.set(existing, Object.assign(cur, patch(cur)));
  };
  for (const [k, e] of Object.entries(map)) {
    const entry = e && typeof e === 'object' ? e : {};
    add(k, () => ({ label: String(entry.label || '').trim(), pinned: entry.favourite === true, lastOpened: Number(entry.lastOpened) || 0,
      color: /^(#[0-9a-f]{3,8})$/i.test(String(entry.color || '')) ? entry.color : '', icon: String(entry.icon || '').trim() }));
  }
  for (const p of legacyFav) add(p, (c) => ({ pinned: c.pinned || !Object.keys(map).some((k) => sameDir(k, p)) }));
  for (const p of legacyRecent) add(p, () => ({}));
  for (const s of sessions.values()) {
    if (!s.cwd || s.hub) continue;
    add(s.cwd, (c) => ({ running: c.running + (isLive(s) ? 1 : 0), lastOpened: Math.max(c.lastOpened, s.lastActivity || 0) }));
  }
  return [...out.values()].map((p) => ({
    ...p,
    name: p.label || basename(p.path) || p.path,
    tag: initials(p.label || basename(p.path)),
    tint: p.color || PALETTE[fnv(projectKey(p.path)) % PALETTE.length],
  })).sort((a, b) => (Number(b.pinned) - Number(a.pinned)) || (b.lastOpened - a.lastOpened) || a.name.localeCompare(b.name));
}

/** Pin or unpin `dir` on the hub; resolves '' or an error text. */
export async function setPinned(dir, pinned) {
  try {
    const config = await call('config.get', {});
    const projects = Object.assign({}, (config && config.projects) || {});
    const keys = Object.keys(projects).filter((k) => sameDir(k, dir));
    if (!keys.length) keys.push(projectKey(dir));
    for (const k of keys) {
      const entry = projects[k] && typeof projects[k] === 'object' ? { ...projects[k] } : {};
      entry.favourite = pinned;
      projects[k] = entry;
    }
    const saved = await call('config.save', { projects });
    const held = Object.entries((saved && saved.projects) || {}).filter(([k]) => sameDir(k, dir));
    if (!held.length || !held.every(([, e]) => e && e.favourite === pinned)) {
      return 'The hub did not save this change. Its config may be locked by another writer; try again.';
    }
    await loadConfig();
    return '';
  } catch (e) { return String((e && e.message) || e); }
}

/** exists / branch / uncommitted count for a launch folder (native inspect_project). */
export async function inspect(path) {
  try { await call('fs.listDir', { path }); } catch (e) { return { exists: false, error: String((e && e.message) || e) }; }
  try {
    const st = await call('git.status', { cwd: path });
    return { exists: true, branch: st && st.branch, changes: Array.isArray(st && st.files) ? st.files.length : 0 };
  } catch (e) {
    const msg = String((e && e.message) || e);
    return { exists: true, git: false, error: /not inside a git work tree|not a git repository/i.test(msg) ? '' : msg };
  }
}

export const projectId = (p) => hash(projectKey(p));
