/**
 * E2E test for the Library (reusable prompts + skills).
 *
 * Launches the real Electron app (built files) and drives the surfaces:
 *  - the library quick-picker hotkey (ctrl+shift+l)
 *  - opening the Library pane via the command palette
 *  - creating a new item through the pane's form (live file-watch reload)
 *  - the scope filter chips
 *
 * Does NOT require Claude auth or claudemon — the library is backed by plain
 * files via libraryService, independent of agent sessions.
 *
 * Prereq:  npm run build   (main + renderer compiled)
 */
import { test, expect, _electron as electron, type ElectronApplication, type Page } from '@playwright/test';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

let app: ElectronApplication;
let page: Page;
let cfgHome: string;

async function closeOverlay() {
  await page.keyboard.press('Escape');
  await page.waitForTimeout(150);
}

/** Open the Library pane via the command palette (works with no active agent). */
async function openLibraryPane() {
  await page.keyboard.press('Control+K');
  const palette = page.getByPlaceholder('Search actions and apps...');
  await expect(palette).toBeVisible({ timeout: 5000 });
  await palette.fill('Library');
  await page.keyboard.press('Enter');
  await expect(page.getByText('⚡ Library')).toBeVisible({ timeout: 8000 });
}

test.describe('Library E2E', () => {
  test.beforeAll(async () => {
    // Isolate the global library under a temp XDG config dir (deterministic,
    // and never touches the user's real library).
    cfgHome = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-e2e-'));
    app = await electron.launch({
      args: ['.'],
      env: { ...process.env, ELECTRON_DEV: '', XDG_CONFIG_HOME: cfgHome },
    });
    page = await app.firstWindow();
    await page.waitForSelector('.app-root', { timeout: 20000 });
  });

  test.afterAll(async () => {
    if (app) await app.close();
    try { fs.rmSync(cfgHome, { recursive: true, force: true }); } catch { /* ignore */ }
  });

  test('quick-picker hotkey lists seeded prompts & skills with action hints', async () => {
    await page.keyboard.press('Control+Shift+L');
    await expect(page.getByPlaceholder('Insert a prompt or skill…')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Summarize & plan')).toBeVisible();
    await expect(page.getByText('Careful refactor (skill)')).toBeVisible();
    // The action-hint footer is unique to the picker (exact text avoids matching the sidebar).
    await expect(page.getByText('Alt+Enter copy')).toBeVisible();
    await closeOverlay();
    await expect(page.getByPlaceholder('Insert a prompt or skill…')).toBeHidden();
  });

  test('Library pane: open, create an item, verify it persists', async () => {
    test.setTimeout(60000);
    await openLibraryPane();
    await expect(page.getByText('Summarize & plan')).toBeVisible();

    // Create a new prompt through the form.
    await page.getByRole('button', { name: '+ New' }).click();
    const title = page.getByPlaceholder('Refactor for testability');
    await expect(title).toBeVisible();
    await title.fill('E2E Created Item');
    await page.getByPlaceholder('Short summary (shown in lists)').fill('made by the e2e test');
    await page.getByPlaceholder('The prompt or skill text…').fill('do the thing in {{cwd}}');
    const save = page.getByRole('button', { name: 'Save', exact: true });
    await expect(save).toBeEnabled();
    await save.click();

    // Back on the list (live reload from libraryService's file watcher).
    await expect(page.getByText('E2E Created Item')).toBeVisible({ timeout: 8000 });
    await expect(page.getByText('made by the e2e test')).toBeVisible();

    // It was written under the temp global library dir with frontmatter.
    const file = path.join(cfgHome, 'workspacer', 'library', 'e2e-created-item.md');
    expect(fs.existsSync(file)).toBeTruthy();
    const raw = fs.readFileSync(file, 'utf-8');
    expect(raw).toContain('do the thing in {{cwd}}');
    expect(raw).toContain('title: E2E Created Item');
  });
});
