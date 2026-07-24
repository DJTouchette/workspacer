import { dialog, type BrowserWindow } from 'electron';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

/**
 * User-uploaded UI fonts. A font file picked in Settings → Appearance is
 * copied to ~/.workspacer/fonts and served through the existing
 * `workspacer-font://` protocol (the same channel the Nerd Font discovery
 * uses); every installed file gets an @font-face injected at window load —
 * and immediately on upload — so `config.ui.fontFamily = "custom:<Family>"`
 * resolves by name in the renderer (lib/uiFont.ts).
 */

const FONT_EXT = /\.(ttf|otf|woff2?)$/i;

export function customFontsDir(): string {
  return path.join(os.homedir(), '.workspacer', 'fonts');
}

/** The shared workspacer-font:// file map (owned by main/index.ts). */
let protocolMap: Map<string, string> | null = null;

/** Display/family name for an installed file: extension, variable-font axis
 *  brackets, and common style suffixes stripped; separators become spaces.
 *  "SpaceGrotesk[wght].ttf" → "SpaceGrotesk", "My_Font-Regular.otf" → "My Font". */
export function customFontFamily(file: string): string {
  return (
    file
      .replace(FONT_EXT, '')
      .replace(/\[[^\]]*\]/g, '')
      .replace(/[-_. ]?(VariableFont[^.]*|Variable|Regular|VF)$/i, '')
      .replace(/[-_.]+/g, ' ')
      .trim() || file
  );
}

function formatOf(file: string): string {
  const ext = (file.split('.').pop() || '').toLowerCase();
  if (ext === 'woff2') return 'woff2';
  if (ext === 'woff') return 'woff';
  if (ext === 'otf') return 'opentype';
  return 'truetype';
}

function fontFaceRule(file: string): string {
  return `@font-face { font-family: "${customFontFamily(file)}"; src: url("workspacer-font://${encodeURIComponent(file)}") format('${formatOf(file)}'); font-weight: 100 900; font-display: swap; }\n`;
}

function installedFiles(): string[] {
  try {
    return fs.readdirSync(customFontsDir()).filter((f) => FONT_EXT.test(f));
  } catch {
    return []; // dir doesn't exist yet — no fonts installed
  }
}

/** Register every installed font into the protocol map (call once at boot). */
export function initCustomFonts(fontFileMap: Map<string, string>): void {
  protocolMap = fontFileMap;
  for (const file of installedFiles()) {
    fontFileMap.set(file, path.join(customFontsDir(), file));
  }
}

/** All installed custom fonts, for the Settings font picker. */
export function listCustomFonts(): Array<{ file: string; family: string }> {
  return installedFiles().map((file) => ({ file, family: customFontFamily(file) }));
}

/** @font-face rules for every installed font — injected next to the Nerd Font
 *  CSS on window load. */
export function customFontsCss(): string {
  return installedFiles().map(fontFaceRule).join('');
}

/**
 * Pick a font file, copy it into the fonts dir, register + inject it live.
 * Returns the family name to store as `custom:<family>`, or null on cancel.
 */
export async function installCustomFont(
  win: BrowserWindow | null,
): Promise<{ file: string; family: string } | null> {
  const result = await dialog.showOpenDialog(win!, {
    title: 'Choose a font file',
    filters: [{ name: 'Fonts', extensions: ['ttf', 'otf', 'woff', 'woff2'] }],
    properties: ['openFile'],
  });
  if (result.canceled || !result.filePaths.length) return null;
  const src = result.filePaths[0];
  const file = path.basename(src);
  if (!FONT_EXT.test(file)) return null;

  const dir = customFontsDir();
  fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(src, path.join(dir, file));
  protocolMap?.set(file, path.join(dir, file));

  // Make the family usable immediately — no restart between upload and apply.
  if (win && !win.isDestroyed()) {
    win.webContents.insertCSS(fontFaceRule(file)).catch(() => {});
  }
  return { file, family: customFontFamily(file) };
}
