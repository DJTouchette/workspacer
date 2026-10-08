#!/usr/bin/env node
// Builds services/hub-rs/assets/web/m-next/icons.svg: a <symbol> sprite of the
// Lucide icons the native client uses (gpui-component's IconName set is
// Lucide) plus native's brand marks. Checked in; rerun after adding a name.
//   node scripts/build-mobile-icons.mjs [lucide-react dir]
// Default lucide source: the desktop renderer's lucide-react (0.460).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const lucide = process.argv[2] || path.join(ROOT, 'apps/desktop/src/renderer/node_modules/lucide-react');
const brands = path.join(ROOT, 'apps/native/assets/icons/brand');
const names = ['plus', 'folder', 'folder-open', 'book-open', 'settings', 'search', 'chevron-left', 'chevron-right',
  'chevron-down', 'chevron-up', 'arrow-up', 'arrow-down', 'arrow-left', 'arrow-right', 'x', 'check', 'triangle-alert',
  'circle-check', 'circle-x', 'eye-off', 'inbox', 'square-terminal', 'bot', 'file', 'copy', 'external-link',
  'loader-circle', 'calendar', 'star', 'redo', 'info', 'palette', 'globe', 'bell', 'user-round', 'file-diff',
  'message-square-plus', 'reply', 'megaphone', 'gallery-vertical-end', 'square', 'pause', 'play', 'snowflake',
  'sparkles', 'git-branch', 'paperclip', 'ellipsis', 'archive', 'history', 'shield-check', 'type', 'smartphone',
  'server', 'zap', 'list-checks', 'clock', 'refresh-cw', 'circle-stop', 'pin', 'trash-2', 'rotate-ccw', 'wifi-off',
  'image', 'moon', 'sun', 'power', 'file-text', 'pencil', 'list-todo'];
let out = '<svg xmlns="http://www.w3.org/2000/svg">\n';
for (const n of names) {
  const src = fs.readFileSync(path.join(lucide, 'dist/esm/icons', n + '.js'), 'utf8');
  const body = src.slice(src.indexOf('[', src.indexOf('createLucideIcon(')), src.lastIndexOf(']);') + 1);
  const nodes = Function('return ' + body)();
  const inner = nodes.map(([tag, a]) => '<' + tag + ' ' + Object.entries(a).filter(([k]) => k !== 'key')
    .map(([k, v]) => `${k}="${v}"`).join(' ') + '/>').join('');
  out += `<symbol id="i-${n}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</symbol>\n`;
}
for (const b of ['claude', 'openai']) {
  const svg = fs.readFileSync(path.join(brands, b + '.svg'), 'utf8');
  const inner = svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');
  out += `<symbol id="b-${b}" viewBox="0 0 24 24" fill="currentColor">${inner}</symbol>\n`;
}
fs.writeFileSync(path.join(ROOT, 'services/hub-rs/assets/web/m-next/icons.svg'), out + '</svg>\n');
console.log('icons:', names.length + 2);
