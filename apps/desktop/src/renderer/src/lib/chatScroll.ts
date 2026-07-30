/**
 * Scroll math for the chat transcript's "your message pins to the top" tail.
 *
 * The transcript renders a spacer of dead space below the last real content so
 * the newest user message can be scrolled all the way up to the top of the
 * viewport — the reply then grows *into* the empty space below it instead of
 * the pair being pinned to the bottom edge with no room to read.
 *
 * The trick is that the spacer is derived, not animated: it's exactly the
 * slack needed to put the pinned message at the top, so it shrinks by as much
 * as the reply grows. Total scroll height stays constant while the reply
 * streams, which means the existing sticky-bottom autoscroll keeps the pinned
 * message in place without any per-frame scroll math of its own. Once the reply
 * is taller than the viewport the spacer is 0 and the transcript behaves
 * exactly as it did before — normal follow-the-bottom streaming.
 */

/** How much breathing room is left above the pinned message, in px. */
export const PIN_TOP_GAP = 12;

export interface TailPadInput {
  /** Visible height of the scroll container (`clientHeight`). */
  viewportHeight: number;
  /** Scrollable height of the container EXCLUDING the current spacer. */
  contentHeight: number;
  /** Scroll offset of the pinned message's top edge (0 = top of content). */
  anchorTop: number;
  /** Breathing room to leave above the pinned message. */
  topGap?: number;
}

/**
 * Height of the tail spacer needed for `anchorTop` to reach the top of the
 * viewport at maximum scroll. 0 when the content below the anchor already
 * fills the viewport (nothing to pad — the transcript scrolls normally).
 */
export function tailPadForAnchor({
  viewportHeight,
  contentHeight,
  anchorTop,
  topGap = PIN_TOP_GAP,
}: TailPadInput): number {
  if (viewportHeight <= 0) return 0;
  // maxScroll = contentHeight + pad - viewportHeight, and we need that to reach
  // anchorTop - topGap for the pinned message to land at the top of the view.
  return Math.max(0, Math.round(anchorTop - topGap + viewportHeight - contentHeight));
}

export interface ContentEndInput {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
  /** Current tail spacer height — dead space, not content. */
  tailPad: number;
}

/**
 * Distance from the bottom of the viewport to the end of the REAL content
 * (the tail spacer doesn't count). Negative once the content end is above the
 * viewport bottom — i.e. sitting in the padded region after a send.
 *
 * Both "am I stuck to the bottom?" and "show the scroll-to-bottom button?" ask
 * this rather than the raw scroll distance, so the spacer never reads as
 * "the user scrolled away from the latest message".
 */
export function distanceFromContentEnd({
  scrollHeight,
  scrollTop,
  clientHeight,
  tailPad,
}: ContentEndInput): number {
  return scrollHeight - scrollTop - clientHeight - tailPad;
}
