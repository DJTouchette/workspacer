/**
 * Finding the images in a chat message.
 *
 * Two different jobs, deliberately separate:
 *
 *  - **What you attached.** The composer turns attachments into a text prefix
 *    (`[Image: /abs/path.png] …`, see claude/fileAttachment). That marker is for
 *    the agent, not for you — the transcript should show the picture and drop
 *    the bookkeeping.
 *  - **What the agent mentioned.** An agent that writes a screenshot says so in
 *    prose ("saved it to /tmp/shot.png"). Those stay in the text (they're part
 *    of the sentence, and the path is a live FileLink); the thumbnail is an
 *    extra below the message.
 */

/** Extensions worth trying to render inline. Mirrors IMAGE_EXTS in
 *  claude/fileAttachment, minus svg — the preview pipeline rasterizes bitmaps,
 *  and an svg that fails simply yields no tile. */
const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|ico|tiff?)$/i;

/** Attachment markers exactly as `buildPromptPrefix` writes them. */
const IMAGE_MARKER = /\[Image:\s*([^\]]+)\]\s*/g;

export function isImagePath(path: string): boolean {
  return IMAGE_EXT.test(path.trim());
}

export interface MessageAttachments {
  /** The message with its `[Image: …]` markers removed. */
  text: string;
  /** Absolute paths pulled out of those markers, in order, deduped. */
  paths: string[];
}

/**
 * Split a user message into the part worth reading and the images it carried.
 *
 * Only the `[Image: …]` markers are touched: `[File: …]` and `[PDF: …]` stay
 * put, because there's nothing to show for them and the marker is the only
 * evidence they were sent at all.
 */
export function extractImageAttachments(content: string | undefined): MessageAttachments {
  if (!content) return { text: '', paths: [] };
  const paths: string[] = [];
  const text = content.replace(IMAGE_MARKER, (whole, raw: string) => {
    const path = raw.trim();
    // A marker for something we can't render stays as text rather than
    // vanishing — otherwise the message would silently lose it.
    if (!isImagePath(path)) return whole;
    if (!paths.includes(path)) paths.push(path);
    return '';
  });
  return { text: text.trim(), paths };
}

/**
 * Image paths mentioned in prose, for the thumbnail strip under an agent's
 * message. Conservative on purpose: absolute paths only (a bare `chart.png`
 * can't be resolved without a cwd, and the caller has one only sometimes), and
 * capped, since a message listing twenty generated frames should not become a
 * contact sheet.
 */
export function imagePathsInText(content: string | undefined, max = 4): string[] {
  if (!content) return [];
  const out: string[] = [];
  // Paths as they appear in prose or code spans: POSIX, UNC, or Windows drive,
  // ending at whitespace or the usual sentence punctuation.
  const re = /(?:\/|\\\\|[a-zA-Z]:[\\/])[^\s`'"<>()[\]]+/g;
  for (const m of content.match(re) ?? []) {
    const path = m.replace(/[.,;:!?]+$/, '');
    if (!isImagePath(path)) continue;
    if (out.includes(path)) continue;
    out.push(path);
    if (out.length >= max) break;
  }
  return out;
}
