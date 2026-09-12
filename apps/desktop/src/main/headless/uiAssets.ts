import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { FONT_EXT, customFontFamily } from '../shared/customFonts';
import { projectIconsDir, mimeForIcon, downloadProjectIcon } from '../services/projectIcons';
import { assertPathAllowed } from '../lib/pathConfinement';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';

const fontDir = () => path.join(os.homedir(), '.workspacer', 'fonts');
const FONT_LIMIT = 12 * 1024 * 1024;
function basename(value: unknown): string {
  if (typeof value !== 'string' || value.length > 200 || !value || /[\\/\0]/.test(value) || value === '.' || value === '..') throw new Error('Asset file must be one filename');
  return value;
}
export function listUIFonts() {
  try { return fs.readdirSync(fontDir()).filter((f) => FONT_EXT.test(f)).map((file) => ({ file, family: customFontFamily(file) })); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []; throw error; }
}
export function readUIAsset(kind: unknown, file: unknown) {
  const name = basename(file);
  if (kind !== 'font' && kind !== 'icon') throw new Error('Unknown UI asset kind');
  if (kind === 'font' ? !FONT_EXT.test(name) : !/^[a-f0-9]{32}\.(png|jpg|gif|webp|svg|ico|avif)$/.test(name)) throw new Error('Invalid UI asset filename');
  const root = kind === 'font' ? fontDir() : projectIconsDir();
  const full = assertPathAllowed('ui.asset', path.join(root, name), [root]);
  const fd = fs.openSync(full, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
  try {
    const limit = kind === 'font' ? FONT_LIMIT : 2 * 1024 * 1024;
    const stat = fs.fstatSync(fd), current = fs.statSync(full);
    if (!stat.isFile() || stat.size > limit || stat.ino !== current.ino || stat.dev !== current.dev || assertPathAllowed('ui.asset', full, [root]) !== full) throw new Error('UI asset changed or exceeds its size limit');
    const buffer = Buffer.alloc(limit + 1);
    let size = 0;
    while (size < buffer.length) {
      const count = fs.readSync(fd, buffer, size, buffer.length - size, null);
      if (count === 0) break;
      size += count;
    }
    if (size > limit) throw new Error('UI asset exceeds its size limit');
    const bytes = buffer.subarray(0, size);
    return { dataBase64: bytes.toString('base64'), mime: kind === 'icon' ? mimeForIcon(name) : `font/${path.extname(name).slice(1).toLowerCase()}`, ...(kind === 'font' ? { family: customFontFamily(name) } : {}) };
  } finally { fs.closeSync(fd); }
}
export function installUIFont(name: unknown, dataBase64: unknown) {
  const file = basename(name);
  if (!FONT_EXT.test(file) || typeof dataBase64 !== 'string' || dataBase64.length > Math.ceil(FONT_LIMIT * 4 / 3) + 4) throw new Error('Choose a font file up to 12 MiB');
  const bytes = Buffer.from(dataBase64, 'base64');
  if (!bytes.length || bytes.length > FONT_LIMIT || !['00010000', '4f54544f', '74727565', '74797031', '774f4646', '774f4632'].includes(bytes.subarray(0,4).toString('hex'))) throw new Error('The file is not a supported font');
  fs.mkdirSync(fontDir(), { recursive: true });
  const target = assertPathAllowed('desktop.installUiFont', path.join(fontDir(), file), [fontDir()]);
  atomicWriteFileSync(target, bytes, { mode: 0o644 });
  return { file, family: customFontFamily(file) };
}
export async function installProjectIcon(url: unknown) {
  if (typeof url !== 'string') throw new Error('Invalid icon URL');
  return { ok: true as const, ...await downloadProjectIcon(url) };
}
