import { describe, it, expect } from 'vitest';
import { tailPadForAnchor, distanceFromContentEnd, PIN_TOP_GAP } from './chatScroll';

describe('tailPadForAnchor', () => {
  it('pads enough for the pinned message to reach the top of the viewport', () => {
    // 600px viewport, 2000px of transcript, newest user message at 1900 — only
    // 100px of content sits below it, so the rest has to be dead space.
    const pad = tailPadForAnchor({ viewportHeight: 600, contentHeight: 2000, anchorTop: 1900 });
    expect(pad).toBe(600 - 100 - PIN_TOP_GAP);
    // Sanity: at max scroll the anchor lands PIN_TOP_GAP below the viewport top.
    const maxScroll = 2000 + pad - 600;
    expect(1900 - maxScroll).toBe(PIN_TOP_GAP);
  });

  it('shrinks by exactly what the reply grows, so the pin holds still', () => {
    const before = tailPadForAnchor({ viewportHeight: 600, contentHeight: 2000, anchorTop: 1900 });
    const after = tailPadForAnchor({ viewportHeight: 600, contentHeight: 2300, anchorTop: 1900 });
    expect(before - after).toBe(300);
    // Total scroll height is unchanged — that's why streaming doesn't move the
    // pinned message even though the sticky-bottom autoscroll keeps firing.
    expect(2000 + before).toBe(2300 + after);
  });

  it('is 0 once the reply already fills the viewport (normal scrolling resumes)', () => {
    expect(tailPadForAnchor({ viewportHeight: 600, contentHeight: 2600, anchorTop: 1900 })).toBe(0);
    expect(tailPadForAnchor({ viewportHeight: 600, contentHeight: 9000, anchorTop: 1900 })).toBe(0);
  });

  it('pads a short transcript whose anchor is above the fold', () => {
    // First message in a fresh session: 80px of transcript, anchor at 40.
    expect(tailPadForAnchor({ viewportHeight: 600, contentHeight: 80, anchorTop: 40 })).toBe(
      600 - 40 - PIN_TOP_GAP,
    );
  });

  it('never pads a zero-height viewport (hidden pane, pre-layout)', () => {
    expect(tailPadForAnchor({ viewportHeight: 0, contentHeight: 2000, anchorTop: 1900 })).toBe(0);
  });
});

describe('distanceFromContentEnd', () => {
  it('reads as "at the bottom" while parked in the tail spacer', () => {
    // Scrolled all the way down with a 400px spacer: the raw distance is 0, and
    // real content ends 400px above the viewport bottom.
    const d = distanceFromContentEnd({
      scrollHeight: 2400,
      scrollTop: 1800,
      clientHeight: 600,
      tailPad: 400,
    });
    expect(d).toBe(-400);
    expect(d <= 150).toBe(true);
  });

  it('reads as "scrolled away" once the content end is far below the view', () => {
    const d = distanceFromContentEnd({
      scrollHeight: 2400,
      scrollTop: 800,
      clientHeight: 600,
      tailPad: 400,
    });
    expect(d).toBe(600);
    expect(d > 150).toBe(true);
  });

  it('matches the plain scroll distance when there is no spacer', () => {
    expect(
      distanceFromContentEnd({
        scrollHeight: 2000,
        scrollTop: 1000,
        clientHeight: 600,
        tailPad: 0,
      }),
    ).toBe(400);
  });
});
