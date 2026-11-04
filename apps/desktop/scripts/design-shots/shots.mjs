/**
 * The shot list. Each entry:
 *   id         file name (NN-slug) — keep the numbering stable so a redesign
 *              run lines up file-for-file with the baseline
 *   desc       one line for --list and the README
 *   world(w)   optional: derive this shot's world from the default one
 *   localStorage  optional per-shot localStorage seed
 *   url(stage) optional: a different entry URL (e.g. the /m PWA)
 *   ready(page) optional: replaces the default "hero card painted" wait
 *   run(page, {stage, world})  drive the UI to the screen
 */

// ── helpers ─────────────────────────────────────────────────────────────────

const pause = (page, ms = 600) => page.waitForTimeout(ms);

/** Run a command-palette action by its title. */
export async function palette(page, title) {
  await page.keyboard.press('Control+k');
  const input = page.getByPlaceholder(/Search actions/i);
  await input.waitFor({ timeout: 5000 });
  await input.fill(title);
  await pause(page, 400);
  await page.keyboard.press('Enter');
  await pause(page, 1200);
}

/** Select an agent by its sidebar card title. */
async function selectAgent(page, label) {
  await page.getByText(label, { exact: true }).first().click();
  await pause(page, 1200);
}

/** Open Settings and pick a section from its left nav. */
async function settingsSection(page, section) {
  await page.keyboard.press('Control+,');
  await page.getByPlaceholder(/Search settings/i).waitFor({ timeout: 8000 });
  if (section) {
    await page.getByText(section, { exact: true }).last().click();
  }
  await pause(page, 1200);
}

const emptyWorld = (w) => ({
  ...w,
  layout: { agents: [], activeAgentId: '' },
  snapshots: [],
});

const emptyReady = async (page) => {
  await page.locator('.app-root, #root > *').first().waitFor({ timeout: 30_000 });
  await pause(page, 3000);
};

/** Headless Chromium reports Notification.permission as 'denied' whatever the
 *  context grants, so /m would show a "blocked" banner a real paired phone
 *  does not. Present it as granted. */
function grantNotifications() {
  try {
    Object.defineProperty(Notification, 'permission', { get: () => 'granted' });
  } catch {
    /* no Notification API */
  }
}

async function mobileReady(page) {
  await page.locator('.agent').first().waitFor({ timeout: 30_000 });
  await pause(page, 1500);
}

const withTheme = (theme) => (w) => ({ ...w, config: { ...w.config, ui: { ...w.config.ui, theme } } });

// ── shots ───────────────────────────────────────────────────────────────────

export const SHOTS = [
  {
    id: '01-first-run-welcome',
    desc: 'First launch: welcome / onboarding overlay over an empty workspace',
    world: (w) => ({ ...emptyWorld(w), config: { ...w.config, onboardingDismissed: false } }),
    localStorage: { 'wks:first-task-guidance-dismissed': null },
    ready: emptyReady,
  },
  {
    id: '02-empty-state',
    desc: 'No agents yet, onboarding dismissed: the empty home',
    world: emptyWorld,
    ready: emptyReady,
  },
  {
    id: '03-overview',
    desc: 'Overview dashboard: usage, fleet status, projects',
    run: (page) => palette(page, 'Open Overview'),
  },
  {
    id: '04-agent-chat',
    desc: 'Hero agent mid-conversation: tool calls, inline diff, running Bash, task plan, sidebar fleet in mixed states',
  },
  {
    id: '05-agent-chat-inspector',
    desc: 'Same agent with the inspector rail open (plan, files, usage)',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+E');
      await pause(page, 1500);
    },
  },
  {
    id: '06-permission-prompt',
    desc: 'Agent blocked on a Bash permission prompt',
    run: (page) => selectAgent(page, 'Reconcile fee rounding'),
  },
  {
    id: '07-question-prompt',
    desc: 'Agent asking a multiple-choice question',
    run: (page) => selectAgent(page, 'Dark-mode pass on settings'),
  },
  {
    id: '08-finished-agent',
    desc: 'Codex agent finished, idle with uncommitted changes',
    run: (page) => selectAgent(page, 'Add retry budget to webhooks'),
  },
  {
    id: '09-review-changes',
    desc: 'Git review pane: changed files + diff for the hero agent',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+G');
      await pause(page, 2500);
    },
  },
  {
    id: '10-fleet-deck',
    desc: 'Fleet Deck: every agent at a glance',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+F');
      await pause(page, 1800);
    },
  },
  {
    id: '11-inbox',
    desc: 'Inbox drawer: approval, question and review items',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+I');
      await pause(page, 1200);
    },
  },
  {
    id: '12-command-palette',
    desc: 'Command palette (Ctrl+K)',
    run: async (page) => {
      await page.keyboard.press('Control+k');
      await pause(page, 1000);
    },
  },
  {
    id: '13-spawn-dialog',
    desc: 'New agent dialog',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+N');
      await pause(page, 1500);
    },
  },
  {
    id: '14-terminal-pane',
    desc: 'Shell terminal pane with output',
    run: async (page) => {
      await palette(page, 'New Terminal');
      await pause(page, 1500);
    },
  },
  {
    id: '15-context-window',
    desc: 'Context window breakdown for the current agent',
    run: (page) => palette(page, 'Context Window'),
  },
  {
    id: '16-session-history',
    desc: 'Session history pane',
    run: (page) => palette(page, 'Session History'),
  },
  {
    id: '17-agent-monitor',
    desc: 'Agent monitor: subagents and workflow runs',
    run: (page) => palette(page, 'Open Agent Monitor'),
  },
  {
    id: '18-recent-agents',
    desc: 'Recent agents pane',
    run: (page) => palette(page, 'Recent agents'),
  },
  {
    id: '19-library',
    desc: 'Prompt library',
    run: async (page) => {
      await page.keyboard.press('Control+Shift+L');
      await pause(page, 1200);
    },
  },
  {
    id: '20-settings-session',
    desc: 'Settings → Session',
    run: (page) => settingsSection(page, 'Session'),
  },
  {
    id: '21-settings-appearance',
    desc: 'Settings → Appearance',
    run: (page) => settingsSection(page, 'Appearance'),
  },
  {
    id: '22-settings-jobs',
    desc: 'Settings → Jobs (scheduled agent jobs)',
    run: (page) => settingsSection(page, 'Jobs'),
  },
  {
    id: '23-settings-routing',
    desc: 'Settings → Routing',
    run: (page) => settingsSection(page, 'Routing'),
  },
  {
    id: '24-settings-keybindings',
    desc: 'Settings → Keybindings',
    run: (page) => settingsSection(page, 'Keybindings'),
  },
  {
    id: '25-layouts-dialog',
    desc: 'Layouts dialog',
    run: (page) => palette(page, 'Layouts…'),
  },
  {
    id: '26-shortcuts-overlay',
    desc: 'Keyboard shortcuts reference (F1)',
    run: async (page) => {
      await page.keyboard.press('F1');
      await pause(page, 1000);
    },
  },
  {
    id: '27-remote-control',
    desc: 'Remote control / share-to-phone dialog',
    run: (page) => palette(page, 'Remote Control…'),
  },
  {
    id: '28-focus-mode',
    desc: 'Focus UI mode (minimal chrome)',
    world: (w) => ({ ...w, config: { ...w.config, ui: { ...w.config.ui, mode: 'focus' } } }),
  },
  {
    id: '29-light-agent-chat',
    desc: 'Light theme: hero agent chat',
    world: withTheme('light'),
  },
  {
    id: '30-light-overview',
    desc: 'Light theme: Overview',
    world: withTheme('light'),
    run: (page) => palette(page, 'Open Overview'),
  },
  {
    id: '31-mobile-fleet',
    desc: 'Phone PWA (/m): fleet list',
    viewport: { width: 430, height: 932 },
    // Otherwise /m shows a "notifications are blocked" banner a real paired
    // phone would not have.
    context: { isMobile: true, hasTouch: true },
    initScript: grantNotifications,
    url: (stage) => stage.mobileUrl,
    ready: mobileReady,
  },
  {
    id: '32-mobile-chat',
    desc: 'Phone PWA (/m): agent chat',
    viewport: { width: 430, height: 932 },
    // Otherwise /m shows a "notifications are blocked" banner a real paired
    // phone would not have.
    context: { isMobile: true, hasTouch: true },
    initScript: grantNotifications,
    url: (stage) => stage.mobileUrl,
    ready: mobileReady,
    run: async (page) => {
      await page.locator('.agent', { hasText: 'atlas' }).first().click();
      await pause(page, 2000);
    },
  },
  {
    id: '33-mobile-flows',
    desc: 'Phone PWA (/m): Flows tab — running workflow for the hero agent',
    viewport: { width: 430, height: 932 },
    context: { isMobile: true, hasTouch: true },
    initScript: grantNotifications,
    url: (stage) => stage.mobileUrl,
    ready: mobileReady,
    run: async (page) => {
      await page.getByText('Flows', { exact: true }).first().click();
      await pause(page, 1500);
    },
  },
  {
    id: '34-settings-fleet-manager',
    desc: 'Settings → Fleet Manager',
    run: (page) => settingsSection(page, 'Fleet Manager'),
  },
];
