// Test-only Cargo owner. The synchronous caller may be killed or time out; this
// asynchronous wrapper retains the process-group root until tree cleanup ends.
const { spawn, spawnSync } = require('node:child_process');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (part) => {
  input += part;
});
process.stdin.on('end', () => {
  let spec;
  try {
    spec = JSON.parse(input);
  } catch {
    process.exitCode = 2;
    return;
  }
  if (
    !Array.isArray(spec.args) ||
    !Number.isSafeInteger(spec.parentPid) ||
    !Number.isFinite(spec.timeoutMs) ||
    spec.timeoutMs < 1
  ) {
    process.exitCode = 2;
    return;
  }
  const child = spawn(spec.command, spec.args, {
    stdio: ['ignore', 'inherit', 'inherit'],
    detached: process.platform !== 'win32',
    windowsHide: true,
  });
  let stopped = false;
  let deadline, parentWatch;
  function killTree() {
    if (!child.pid) return;
    if (process.platform === 'win32') {
      // Run while the root is alive: killing Cargo first loses its descendants.
      spawnSync('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], {
        stdio: 'ignore',
        windowsHide: true,
        timeout: 10_000,
      });
    } else {
      try {
        process.kill(-child.pid, 'SIGKILL');
      } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
  }
  function stop(code) {
    if (stopped) return;
    stopped = true;
    process.exitCode = code;
    clearTimeout(deadline);
    clearInterval(parentWatch);
    killTree();
  }
  process.on('SIGTERM', () => stop(143));
  process.on('SIGINT', () => stop(130));
  deadline = setTimeout(() => stop(124), spec.timeoutMs);
  parentWatch = setInterval(() => {
    try {
      process.kill(spec.parentPid, 0);
    } catch (error) {
      if (error.code === 'ESRCH') stop(143);
    }
  }, 500);
  child.on('error', (error) => {
    console.error(error.message);
    stop(1);
  });
  child.on('exit', (code) => {
    if (!stopped) {
      // Also retire a descendant that outlived a failed compiler root on Unix.
      stop(code ?? 1);
    }
  });
});
