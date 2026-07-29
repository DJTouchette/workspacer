/**
 * Spill a pasted image to disk so it can be attached.
 *
 * Attachments are paths — the agent opens the file itself — but a screenshot on
 * the clipboard has no file behind it. `webUtils.getPathForFile` returns '' for
 * it (see fileAttachment.ts), which is why pasting a screenshot used to do
 * nothing at all. Writing it to a temp file gives the paste something to point
 * at, and the composer thumbnails it like any other image.
 *
 * The renderer has the bytes too, but reading them from Electron's clipboard in
 * main avoids shuttling a multi-megabyte blob across the bridge only to write
 * it out here.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { clipboard } from 'electron';

/** Where pasted images land. Under the OS temp dir: these are scratch files the
 *  agent reads once, not documents worth keeping in the user's tree. */
export function pastedImageDir(): string {
  return path.join(os.tmpdir(), 'workspacer-pasted');
}

/** Distinguishes images pasted within the same millisecond. */
let sequence = 0;

export interface PastedImage {
  path: string;
  width: number;
  height: number;
}

/**
 * Write the clipboard's image to a PNG and return its path, or null when the
 * clipboard holds no image (the caller then treats the paste as ordinary text).
 */
export function savePastedImage(): PastedImage | null {
  const img = clipboard.readImage();
  if (img.isEmpty()) return null;

  const dir = pastedImageDir();
  fs.mkdirSync(dir, { recursive: true });
  const name = `pasted-${Date.now()}-${sequence++}.png`;
  const file = path.join(dir, name);
  fs.writeFileSync(file, img.toPNG());

  const { width, height } = img.getSize();
  return { path: file, width, height };
}
