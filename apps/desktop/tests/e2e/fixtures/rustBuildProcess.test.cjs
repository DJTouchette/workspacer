const test = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const wrapper = path.join(__dirname, 'rustBuildProcess.cjs');
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check) {
  const end = Date.now() + 20_000;
  while (!check()) {
    assert.ok(Date.now() < end, 'process ownership fixture timed out');
    await delay(50);
  }
}
function alive(pid) {
  try {
    process.kill(pid, 0);
    if (process.platform === 'linux')
      return !fs.readFileSync(`/proc/${pid}/stat`, 'utf8').includes(') Z ');
    return true;
  } catch {
    return false;
  }
}
for (const mode of ['deadline', 'signal']) {
  test(
    `compiler ${mode} reaps its grandchild`,
    { skip: process.platform === 'win32' },
    async () => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-build-owner-'));
      const marker = path.join(root, 'grandchild.pid');
      const source = `const {spawn}=require('node:child_process');const fs=require('node:fs');const c=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});fs.writeFileSync(${JSON.stringify(marker)},String(c.pid));setInterval(()=>{},1000);`;
      const owner = spawn(process.execPath, [wrapper], { stdio: ['pipe', 'ignore', 'pipe'] });
      const exited = once(owner, 'exit');
      owner.stderr.resume();
      owner.stdin.end(
        JSON.stringify({
          command: process.execPath,
          args: ['-e', source],
          parentPid: process.pid,
          timeoutMs: mode === 'deadline' ? 5000 : 30000,
        }),
      );
      let grandchild;
      try {
        await until(() => fs.existsSync(marker));
        grandchild = Number(fs.readFileSync(marker, 'utf8'));
        assert.ok(alive(grandchild));
        if (mode === 'signal') owner.kill('SIGTERM');
        const [code] = await exited;
        assert.equal(code, mode === 'deadline' ? 124 : 143);
        await until(() => !alive(grandchild));
      } finally {
        owner.kill('SIGTERM');
        if (grandchild && alive(grandchild)) process.kill(grandchild, 'SIGKILL');
        fs.rmSync(root, { recursive: true, force: true });
      }
    },
  );
}
