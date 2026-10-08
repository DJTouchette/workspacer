// Appearance: one of native's eight themes (generated tokens), or "Match the
// phone" — Dark/Light following prefers-color-scheme, the default here. The
// choice is stored with native's slugs (wks.mnext.theme) so a later "follow my
// desktop" setting is a straight copy. boot.js applies it before first paint.
import { THEMES } from './themes.js';

const KEY = 'wks.mnext.theme';
const SIZE_KEY = 'wks.mnext.textSize';
const MOTION_KEY = 'wks.mnext.reduceMotion';
const media = window.matchMedia ? matchMedia('(prefers-color-scheme: light)') : null;

export function pref() {
  try { const p = JSON.parse(localStorage.getItem(KEY)); if (p && (p.match || THEMES.some((t) => t.slug === p.theme))) return p; } catch { /* default */ }
  return { match: true, theme: 'dark' };
}
export const resolved = () => { const p = pref(); return p.match ? (media && media.matches ? 'light' : 'dark') : p.theme; };
export function setPref(p) { localStorage.setItem(KEY, JSON.stringify(p)); apply(); }
export const textSize = () => { const n = Number(localStorage.getItem(SIZE_KEY)); return n >= 13 && n <= 20 ? n : 16; };
export function setTextSize(n) { localStorage.setItem(SIZE_KEY, String(n)); apply(); }
export const reduceMotion = () => localStorage.getItem(MOTION_KEY) === '1';
export function setReduceMotion(on) { localStorage.setItem(MOTION_KEY, on ? '1' : '0'); apply(); }

export function apply() {
  const slug = resolved();
  const root = document.documentElement;
  root.dataset.theme = slug;
  root.style.setProperty('--chat-size', textSize() + 'px');
  root.classList.toggle('reduce-motion', reduceMotion());
  const t = THEMES.find((x) => x.slug === slug);
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta && t) meta.setAttribute('content', t.chat);
}
if (media) media.addEventListener('change', () => { if (pref().match) apply(); });

export { THEMES };
