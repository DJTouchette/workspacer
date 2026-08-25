/**
 * Composer attachments when there is no host path — the browser half of the
 * paperclip, the drop target and the clipboard.
 *
 * The desktop attaches by HOST PATH: picker, drop and paste all end in a path
 * on the same machine the agent runs on, and the prompt carries only
 * `[Image: /abs/path]` — the agent opens the file itself. A browser has no such
 * path. The file is on the viewer's machine; the agent is on the host's. The
 * old web `pickFiles` papered over that with a `window.prompt` asking the user
 * to TYPE paths "on the host", which is worse than nothing: it looks like a
 * feature and attaches something that doesn't exist over there.
 *
 * The real capability already exists and is already proven — `files.upload`
 * (`services/hub/cmd/hub/upload.go`), which the /m PWA has used for photo
 * attachments since it shipped. It is hub-LOCAL, so `hub:<peer>/files.upload`
 * lands the bytes on the peer that runs the agent; the hub picks the directory
 * and basename (only an allowlisted extension survives from the caller's name)
 * and answers with an absolute path. That path then flows through the identical
 * `[Image: /path]` prefix the desktop builds, so nothing downstream changes.
 *
 * WHICH CLIENTS TAKE THIS PATH IS NOT A PLATFORM CHECK. Desktop remote-client
 * mode ("Connect to remote server") is an Electron shell whose backend IS the
 * web backend (`backend/remoteBackend.ts` = `createWebBackend` + eight
 * host-shell methods) and it deliberately keeps the genuine host `platform`
 * string, so `platform === 'web'` would route it down the host-path branch and
 * attach paths belonging to the wrong machine. The honest signal is the one the
 * renderer already reads: whether a host path could be resolved at all
 * (`getPathForFile` → `''`, `saveClipboardImage` → `null`). Callers fall back
 * to uploading when it could not.
 */

import { postNotification } from './notificationBus';

/** Extensions `files.upload` accepts (upload.go `uploadExts`). Mirrored here so
 *  a refusal costs nothing — encoding 24 MiB to base64 to be told no is rude. */
export const UPLOADABLE_EXTS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'pdf'] as const;
/** upload.go `maxUploadBytes` — the DECODED cap. */
export const MAX_UPLOAD_BYTES = 24 * 1024 * 1024;
/** `accept` for the browser picker: a hint, never the enforcement. */
export const UPLOAD_ACCEPT = UPLOADABLE_EXTS.map((e) => `.${e}`).join(',');
/**
 * Uploads get their own call timeout. The bus default (15s) is sized for
 * control messages; these carry megabytes over whatever link the viewer is on,
 * and a timeout there is a FALSE failure — the hub writes the file anyway. /m
 * settled on the same 90s for the same reason.
 */
export const UPLOAD_TIMEOUT_MS = 90_000;

const MIME_EXT: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'application/pdf': 'pdf',
};

/** File-ish: a real `File`, or anything carrying the three fields we read. */
export interface UploadableFile extends Blob {
  readonly name?: string;
  readonly type: string;
  readonly size: number;
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

/**
 * The advisory name to send. Only the extension survives on the hub side, and a
 * clipboard `File` can arrive with no name at all — deriving one from the MIME
 * type is the difference between an upload and a refusal.
 */
export function uploadFileName(file: Pick<UploadableFile, 'name' | 'type'>): string {
  const raw = (file.name ?? '').trim();
  if (extensionOf(raw)) return raw;
  const fromType = MIME_EXT[(file.type ?? '').toLowerCase()];
  if (!fromType) return raw || 'attachment';
  return `${raw.replace(/\.$/, '') || 'pasted'}.${fromType}`;
}

/** Why the hub would refuse this file, in words the user can act on — or null. */
export function uploadRefusal(file: Pick<UploadableFile, 'name' | 'type' | 'size'>): string | null {
  const name = uploadFileName(file);
  const ext = extensionOf(name);
  if (!(UPLOADABLE_EXTS as readonly string[]).includes(ext)) {
    return `${name} — a browser can only attach ${UPLOADABLE_EXTS.join(', ')} files (the bytes have to travel to the machine running the agent).`;
  }
  if ((file.size ?? 0) > MAX_UPLOAD_BYTES) {
    return `${name} — larger than the ${MAX_UPLOAD_BYTES >> 20} MiB upload limit.`;
  }
  return null;
}

/** Base64 payload for `files.upload` (the `data:` prefix stripped). */
export function readFileBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result ?? '');
      const comma = url.indexOf(',');
      if (comma < 0) reject(new Error('could not be read'));
      else resolve(url.slice(comma + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error('could not be read'));
    reader.readAsDataURL(file);
  });
}

export interface UploadedAttachment {
  /** Absolute path ON THE AGENT'S MACHINE, from the hub. */
  path: string;
  /** The name the user recognises — the generated basename means nothing. */
  name: string;
}

export type UploadAttachmentFn = (input: {
  name: string;
  dataBase64: string;
  sessionId?: string;
}) => Promise<{ path: string; size?: number }>;

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Upload each file and collect the paths. Never throws: a per-file `errors`
 * entry is the point — a browser attachment can fail for reasons the user must
 * see (wrong type, too big, no provider on a hub that predates the capability),
 * and the old behaviour of quietly attaching nothing is exactly the failure
 * mode this whole change exists to remove.
 */
export async function uploadAttachments(
  files: readonly UploadableFile[],
  opts: { sessionId?: string; upload?: UploadAttachmentFn } = {},
): Promise<{ attached: UploadedAttachment[]; errors: string[] }> {
  const attached: UploadedAttachment[] = [];
  const errors: string[] = [];
  if (files.length === 0) return { attached, errors };

  const upload = opts.upload ?? window.electronAPI?.uploadAttachment;
  if (!upload) {
    return {
      attached,
      errors: [
        'This client cannot upload attachments — the server it is connected to has no files.upload capability.',
      ],
    };
  }

  for (const file of files) {
    const refusal = uploadRefusal(file);
    if (refusal) {
      errors.push(refusal);
      continue;
    }
    const name = uploadFileName(file);
    try {
      const dataBase64 = await readFileBase64(file);
      const res = await upload({ name, dataBase64, sessionId: opts.sessionId });
      if (!res?.path) throw new Error('the server returned no path');
      attached.push({ path: res.path, name });
    } catch (err) {
      errors.push(`${name} — ${message(err)}`);
    }
  }
  return { attached, errors };
}

/** Surface upload failures in the notification center (and its toast). */
export function reportAttachmentFailures(errors: readonly string[]): void {
  if (errors.length === 0) return;
  postNotification({
    level: 'error',
    title: errors.length === 1 ? 'Attachment failed' : `${errors.length} attachments failed`,
    body: errors.join('\n'),
    source: 'workspacer',
  });
}

/**
 * The browser's own file picker, as a promise of `File`s.
 *
 * `oncancel` is not universal, and a promise that never settles would hang the
 * caller's `await` forever (the paperclip would appear to do nothing, twice
 * over) — so a window refocus with nothing chosen is the fallback signal,
 * delayed so a real pick reports first.
 */
export function openBrowserFilePicker(accept: string = UPLOAD_ACCEPT): Promise<UploadableFile[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    input.accept = accept;
    input.style.position = 'fixed';
    input.style.left = '-9999px';

    let settled = false;
    const finish = (files: UploadableFile[]): void => {
      if (settled) return;
      settled = true;
      window.removeEventListener('focus', onFocus);
      input.remove();
      resolve(files);
    };
    const onFocus = (): void => {
      setTimeout(() => {
        if (!input.files || input.files.length === 0) finish([]);
      }, 500);
    };

    input.addEventListener('change', () => finish(Array.from(input.files ?? [])));
    input.addEventListener('cancel', () => finish([]));
    window.addEventListener('focus', onFocus);
    document.body.appendChild(input);
    input.click();
  });
}
