import { test, expect } from '@playwright/test';
import { startMobileHub, HOST_TOKEN, type MobileHub } from './fixtures/mobileHub';

let hub: MobileHub;
test.beforeAll(async () => { hub = await startMobileHub(); });
test.afterAll(async () => { await hub.stop(); });
test.beforeEach(() => hub.reset());

test('browse remote folders and dispatch an explicit model, effort, full access and edited task', async ({ page }, testInfo) => {
  await page.setViewportSize({width:390,height:844});
  const folderCalls: string[] = [];
  await page.routeWebSocket('**/bus?*', route => {
    const server = route.connectToServer();
    route.onMessage(raw => {
      const f = JSON.parse(String(raw));
      if (f.op === 'call' && f.method === 'fs.listDir') {
        folderCalls.push(f.params.path);
        const selected = f.params.path === '/data/repos/mobile-demo';
        route.send(JSON.stringify({op:'result', id:f.id, result:{path:selected ? '/data/repos/mobile-demo':'/data/repos',parent:'/data/repos',home:'/data/home',dirs:selected ? []:['mobile-demo']}}));
      } else server.send(raw);
    });
  });
  await page.goto(`${hub.url}/m?token=${HOST_TOKEN}`);
  await page.locator('nav [data-tab="new"]').click();
  await page.getByText('Browse server folders…', {exact:true}).click();
  await expect(page.locator('#sheet')).toContainText('mobile-demo/');
  await page.locator('#sheet').getByRole('button', {name:'mobile-demo/'}).click();
  await page.locator('#sheet').getByRole('button', {name:'Use this folder'}).click();
  await expect(page.locator('[data-dir="/data/repos/mobile-demo"]')).toHaveClass(/on/);
  await expect(page.locator('#spawnModel')).toBeEnabled();
  await page.locator('#spawnModel').selectOption({label:'Opus 5 (1M)'});
  await page.locator('#spawnEffort').selectOption('high');
  await page.getByRole('button', {name:'Full access',exact:true}).click();
  await page.locator('#spawnLabel').fill('Mobile dispatch');
  await page.locator('#spawnMessage').fill('Implement the selected change and run its tests.');
  await page.locator('#spawnGo').scrollIntoViewIfNeeded();
  await page.screenshot({path:testInfo.outputPath('mobile-spawn.png')});
  await page.locator('#spawnGo').click();
  await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
  expect(hub.callsTo('agents.spawn')[0].params).toMatchObject({
    cwd:'/data/repos/mobile-demo', provider:'claude', model:'claude-opus-5', contextWindow:1000000,
    effort:'high', permissionMode:'bypassPermissions', skipPermissions:true, yoloGranted:true,
    label:'Mobile dispatch', message:'Implement the selected change and run its tests.',
  });
  expect(folderCalls).toContain('/data/repos/mobile-demo');
  expect(hub.callsTo('agents.sendMessage')).toHaveLength(0);
});

test('changing providers clears an incompatible model and preserves the drafted task', async ({page}) => {
  await page.goto(`${hub.url}/m?token=${HOST_TOKEN}`);
  await page.locator('nav [data-tab="new"]').click();
  await expect(page.locator('#spawnModel')).toBeEnabled();
  await page.locator('#spawnModel').selectOption({label:'Opus 5 (1M)'});
  await page.locator('#spawnMessage').fill('My task survives changing provider.');
  await page.locator('[data-prov="codex"]').click();
  await expect(page.locator('#spawnModel')).toBeEnabled();
  await expect(page.locator('#spawnModel')).toHaveValue('');
  await page.locator('#spawnModel').selectOption({label:'gpt-5.4'});
  await expect(page.locator('#spawnMessage')).toHaveValue('My task survives changing provider.');
  await page.locator('#spawnGo').click();
  await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
  expect(hub.callsTo('agents.spawn')[0].params).toMatchObject({provider:'codex',model:'gpt-5.4',message:'My task survives changing provider.'});
  expect(hub.callsTo('agents.spawn')[0].params.contextWindow).toBeUndefined();
});
