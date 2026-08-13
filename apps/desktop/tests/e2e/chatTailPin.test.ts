/**
 * The tail pin: "your newest message rides the top of the viewport while the
 * reply streams in below it".
 *
 * This runs in a REAL browser on purpose. ClaudePaneTailPin.test.tsx covers the
 * same feature in jsdom and passed right through the regression this file
 * exists for, because jsdom returns 0 from getBoundingClientRect and
 * scrollHeight — it can assert the tail spacer was COMPUTED, never that the
 * message LANDED anywhere. The bug was that the spacer was built correctly and
 * never scrolled into, which is invisible to any test without layout.
 *
 * Both directions matter and they pull against each other:
 *   - after a send, the newest message must sit at the TOP (the pin);
 *   - after the USER scrolls up, a later follow tick must NOT drag them back
 *     (what 33f4a83c fixed, and what the pin fix must not undo).
 *
 * Drives src/renderer/chat-pin-harness.html on its own Vite server — no
 * Electron, no daemons, no ports the running app cares about.
 */
import { test, expect, type Page } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';

const PORT = 5199; // deliberately far from the app's 5173 dev server
const URL = `http://localhost:${PORT}/chat-pin-harness.html`;
const rendererDir = path.resolve(__dirname, '../../src/renderer');

let vite: ChildProcess;

test.beforeAll(async () => {
  // --strictPort so a collision fails loudly instead of silently serving the
  // harness from a port this test isn't looking at.
  vite = spawn('npx', ['vite', '--port', String(PORT), '--strictPort'], {
    cwd: rendererDir,
    stdio: 'ignore',
    detached: false,
  });
  const deadline = Date.now() + 60_000;
  for (;;) {
    try {
      const res = await fetch(URL);
      if (res.ok) break;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error(`vite did not serve ${URL} within 60s`);
    await new Promise((r) => setTimeout(r, 500));
  }
});

test.afterAll(() => {
  vite?.kill();
});

/** Scroll geometry of the pane, measured from the pin anchor outward. */
async function geometry(page: Page) {
  return page.evaluate(() => {
    const anchor = document.querySelector('[data-pin-anchor]');
    if (!anchor) throw new Error('no [data-pin-anchor] — the pane did not render');
    let c = anchor.parentElement;
    while (c && !/(auto|scroll)/.test(getComputedStyle(c).overflowY)) c = c.parentElement;
    if (!c) throw new Error('no scroll container above the anchor');
    const a = anchor.getBoundingClientRect();
    const r = c.getBoundingClientRect();
    if (r.height === 0) throw new Error('scroll container has zero height — layout collapsed');
    return {
      anchorTop: Math.round(a.top - r.top),
      viewport: Math.round(r.height),
      scrollTop: Math.round(c.scrollTop),
      maxScroll: Math.round(c.scrollHeight - c.clientHeight),
    };
  });
}

async function openPane(page: Page) {
  await page.goto(URL, { waitUntil: 'load' });
  // The pane opens in Term view; the GUI subtree is display:none until switched,
  // so there is literally nothing to measure before this.
  await page.locator('text=GUI').click();
  await expect(page.locator('textarea:visible').first()).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(1000); // let the transcript settle at its natural bottom
}

async function send(page: Page, text: string) {
  const composer = page.locator('textarea:visible').first();
  await composer.click();
  await composer.fill(text);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(1500);
}

test('the message you just sent rides the top of the viewport', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await openPane(page);

  const before = await geometry(page);
  // A restored transcript opens at its natural bottom — the pin is not armed
  // until you send, so the newest message starts low.
  expect(before.anchorTop).toBeGreaterThan(before.viewport / 2);

  await send(page, 'This message should ride the top of the viewport.');

  const after = await geometry(page);
  // The regression parked it at ~90% of the viewport, short by exactly the
  // spacer height. Anything in the top fifth is the pin working.
  expect(after.anchorTop).toBeLessThan(after.viewport / 5);
  // And the view must actually be scrolled INTO the spacer, not merely have one.
  expect(after.maxScroll - after.scrollTop).toBeLessThanOrEqual(2);
});

test('scrolling up after a send is not undone by the next follow tick', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await openPane(page);
  await send(page, 'pinned message');

  const pinned = await geometry(page);
  expect(pinned.maxScroll - pinned.scrollTop).toBeLessThanOrEqual(2);

  // The user scrolls up mid-reply to re-read something.
  await page.mouse.move(550, 400);
  await page.mouse.wheel(0, -900);
  await page.waitForTimeout(600);
  const scrolledUp = await geometry(page);
  expect(scrolledUp.scrollTop).toBeLessThan(pinned.scrollTop - 200);

  // A viewport change fires the ResizeObserver, which runs followTail. If the
  // pin's programmatic-scroll guard swallowed the user's scroll, this snaps
  // back to the bottom — the bug 33f4a83c fixed.
  await page.setViewportSize({ width: 1100, height: 780 });
  await page.waitForTimeout(1200);
  const afterTick = await geometry(page);
  expect(afterTick.scrollTop).toBeLessThan(afterTick.maxScroll - 200);
});
