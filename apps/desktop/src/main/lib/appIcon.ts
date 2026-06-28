import { nativeImage, type NativeImage } from 'electron';
import * as path from 'path';
import * as fs from 'fs';

// Resolve the workspacer mark for native chrome (BrowserWindow icon on
// Linux/Windows, and Notification icons everywhere). The renderer ships the
// PNG via its public/ dir (-> dist/renderer/icon.png); in unbuilt dev runs we
// fall back to the source master under build/.
const CANDIDATES = [
  path.join(__dirname, '..', 'renderer', 'icon.png'),   // packaged / built renderer
  path.join(__dirname, '..', '..', 'build', 'icon.png'), // dev, source tree
];

let cachedPath: string | null | undefined;
let cachedImage: NativeImage | null | undefined;

/** Absolute path to the app-icon PNG, or null if none is reachable. */
export function appIconPath(): string | null {
  if (cachedPath === undefined) {
    cachedPath = CANDIDATES.find((p) => fs.existsSync(p)) ?? null;
  }
  return cachedPath;
}

/** The app icon as a NativeImage (cached), or null if unavailable. */
export function appIcon(): NativeImage | null {
  if (cachedImage === undefined) {
    const p = appIconPath();
    const img = p ? nativeImage.createFromPath(p) : null;
    cachedImage = img && !img.isEmpty() ? img : null;
  }
  return cachedImage;
}
