import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let url: string;
test.beforeAll(async () => {
  const port = await freePort();
  url = `http://127.0.0.1:${port}/usage-pacing-harness.html`;
  vite = spawn('npx', ['vite', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
    cwd: path.resolve(__dirname, '../../src/renderer'),
    stdio: 'ignore',
  });
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      /* starting */
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error('Usage harness did not start');
});
test.afterAll(() => {
  vite?.kill();
});
test('light/dark, narrow/wide consumption and keyboard detail', async ({ page }, info) => {
  for (const theme of ['light', 'dark'])
    for (const width of [360, 1200]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${url}?theme=${theme}`);
      // "above pace", not "ahead": ahead of schedule is praise, ahead of your
      // allowance is not, and this label used to read as the former.
      const above = page.getByText('above pace', { exact: true });
      await expect(above).toBeVisible();
      await expect(page.getByText('ahead', { exact: true })).toHaveCount(0);
      await expect(page.getByText('on pace', { exact: true })).toBeVisible();
      // The colour a user actually sees: the error token, and the same one on
      // the consumed bar beside it. Only a real browser resolves these.
      const errorColor = await page.evaluate(() =>
        getComputedStyle(document.documentElement).getPropertyValue('--wks-error').trim(),
      );
      expect(errorColor).not.toBe('');
      const asRgb = (v: string) =>
        page.evaluate((c) => {
          const el = document.createElement('span');
          el.style.color = c;
          document.body.append(el);
          const out = getComputedStyle(el).color;
          el.remove();
          return out;
        }, v);
      expect(await above.evaluate((el) => getComputedStyle(el).color)).toBe(
        await asRgb(errorColor),
      );
      // The harness's second card is the 70%-against-52% one; its bar must
      // carry the same colour as its word, and the on-pace card's must not.
      const bar = (n: number) =>
        page
          .getByTestId('usage-consumed')
          .nth(n)
          .evaluate((el) => getComputedStyle(el).backgroundColor);
      expect(await bar(1)).toBe(await asRgb(errorColor));
      expect(await bar(2)).not.toBe(await asRgb(errorColor));
      await expect(page.getByTestId('usage-consumed').first()).toHaveCSS('width', '0px');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: info.outputPath(`usage-${theme}-${width}.png`),
        fullPage: true,
      });
      const identity = '/home/user/company/department/team/accounts/work';
      const card = page.getByRole('button', { name: `Show usage detail: claude · ${identity}` });
      await card.focus();
      await page.keyboard.press('Enter');
      await expect(page.getByRole('dialog').getByText(identity)).toBeVisible();
      await page.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
    }
});

// The Settings control that decides which week those cards are paced against,
// in the real browser rather than in jsdom. What a screenshot can show and a
// unit test cannot is that the unavailable state reads as UNAVAILABLE — dimmed
// with a reason — rather than as a setting that quietly reverted.
test('usage schedule control: live switch and the unavailable state', async ({ page }, info) => {
  for (const theme of ['light', 'dark'])
    for (const width of [360, 1200]) {
      await page.setViewportSize({ width, height: 900 });

      await page.goto(`${url}?theme=${theme}&schedule=seven_day`);
      const week = page.getByRole('button', { name: /Work week/ });
      const every = page.getByRole('button', { name: /Every day/ });
      await expect(week).toBeEnabled();
      await expect(page.getByText(/rises evenly across all seven days/)).toBeVisible();

      await week.click();
      await expect(page.getByText(/climbs Monday to Friday/)).toBeVisible();
      await expect(every).toBeEnabled();
      // The HIGHLIGHT has to follow the hint. A control whose selected pill
      // disagrees with its own explanation is worse than one that does not
      // update at all, and only a real browser resolves these custom
      // properties to the colours a user sees.
      const fill = (b: typeof week) => b.evaluate((el) => getComputedStyle(el).backgroundColor);
      // The pills cross-fade over 120ms, so a colour read the instant the hint
      // flips catches both mid-transition — which is also what made the first
      // screenshot of this control look as though the highlight had not moved.
      await page.waitForTimeout(300);
      expect(await fill(every)).toBe('rgba(0, 0, 0, 0)');
      expect(await fill(week)).not.toBe('rgba(0, 0, 0, 0)');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: info.outputPath(`usage-schedule-${theme}-${width}.png`),
        fullPage: true,
      });

      await page.goto(`${url}?theme=${theme}&schedule=unavailable`);
      await expect(page.getByText(/does not offer the usage-schedule setting/)).toBeVisible();
      await expect(page.getByRole('button', { name: /Work week/ })).toBeDisabled();
      await expect(page.getByRole('button', { name: /Every day/ })).toBeDisabled();
      await page.screenshot({
        path: info.outputPath(`usage-schedule-unavailable-${theme}-${width}.png`),
        fullPage: true,
      });
    }
});
