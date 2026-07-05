/**
 * Import cookies from the user's local Chrome install into Workspacer's
 * `persist:browser` partition. This sidesteps Microsoft's embedded-webview
 * OAuth block entirely — as far as the server's concerned, you're a signed-in
 * Chrome user.
 *
 * How Chrome stores cookies on Windows (Chrome 80+):
 *  1. `Local State` JSON has `os_crypt.encrypted_key` (base64). After decode,
 *     the first 5 bytes are the literal ASCII "DPAPI"; the rest is the
 *     32-byte AES master key encrypted with DPAPI for the current user.
 *  2. Each row in `Cookies` SQLite has `encrypted_value` of shape
 *     `"v10" + 12-byte nonce + ciphertext + 16-byte GCM tag`.
 *
 * We shell out to PowerShell for the DPAPI step (no new native dep needed),
 * use Node's built-in `crypto` for AES-GCM, and Workspacer's already-bundled
 * `better-sqlite3` to read the cookies file.
 *
 * The DPAPI path described above is Windows-only. It's a fallback: the default
 * import path (see the IPC handler in ipc.ts) drives a live Chrome over CDP and
 * is cross-platform. Direct cookie-file decryption on macOS / Linux (Keychain /
 * libsecret with an "AES-128-CBC + PBKDF2 from a known salt" scheme) is not
 * implemented — those platforms rely on the CDP path.
 */

import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as crypto from 'crypto';
import { execFileSync, spawn } from 'child_process';
import { session } from 'electron';
import Database from 'better-sqlite3';
import WebSocket from 'ws';

const PARTITION = 'persist:browser';

interface ImportOptions {
  /** If set, only import cookies whose host_key contains one of these substrings.
   *  Empty/undefined means: import everything Chrome has. */
  domainFilter?: string[];
}

interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
  /** Per-prefix counts (v10, v11, v20, other, plaintext) so we can tell at a
   *  glance whether Chrome moved to app-bound encryption. Values may be
   *  string for the `source` tag. */
  diagnostics?: Record<string, number | string>;
}

function chromeUserDataDir(): string | null {
  if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA;
    if (!localAppData) return null;
    return path.join(localAppData, 'Google', 'Chrome', 'User Data');
  }
  if (process.platform === 'darwin') {
    return path.join(os.homedir(), 'Library', 'Application Support', 'Google', 'Chrome');
  }
  return path.join(os.homedir(), '.config', 'google-chrome');
}

function edgeUserDataDir(): string | null {
  if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA;
    if (!localAppData) return null;
    return path.join(localAppData, 'Microsoft', 'Edge', 'User Data');
  }
  if (process.platform === 'darwin') {
    return path.join(os.homedir(), 'Library', 'Application Support', 'Microsoft Edge');
  }
  return path.join(os.homedir(), '.config', 'microsoft-edge');
}

function readLocalStateMasterKey(userDataDir: string): Buffer {
  const localStatePath = path.join(userDataDir, 'Local State');
  const raw = fs.readFileSync(localStatePath, 'utf-8');
  const parsed = JSON.parse(raw);
  const encryptedKeyB64: string | undefined = parsed?.os_crypt?.encrypted_key;
  if (!encryptedKeyB64) {
    throw new Error(
      'Local State has no os_crypt.encrypted_key — Chrome may use a newer encryption scheme.',
    );
  }
  const encryptedKey = Buffer.from(encryptedKeyB64, 'base64');
  // First 5 bytes are the literal ASCII "DPAPI"; strip them off.
  if (encryptedKey.slice(0, 5).toString('utf-8') !== 'DPAPI') {
    throw new Error('Expected encrypted_key to start with "DPAPI" prefix.');
  }
  const dpapiBlob = encryptedKey.slice(5);
  return dpapiUnprotect(dpapiBlob);
}

/** Decrypt a DPAPI blob using the current user's key. Windows-only. */
function dpapiUnprotect(blob: Buffer): Buffer {
  if (process.platform !== 'win32') {
    throw new Error('DPAPI decryption is Windows-only in this build.');
  }
  // We pipe the base64 through env so PowerShell can pick it up without
  // tripping over command-line quoting. The script just calls ProtectedData.
  const script = `
    Add-Type -AssemblyName System.Security;
    $enc = [System.Convert]::FromBase64String($env:WKS_DPAPI_BLOB);
    $dec = [System.Security.Cryptography.ProtectedData]::Unprotect(
      $enc, $null,
      [System.Security.Cryptography.DataProtectionScope]::CurrentUser);
    [System.Console]::Out.Write([System.Convert]::ToBase64String($dec));
  `;
  const env = { ...process.env, WKS_DPAPI_BLOB: blob.toString('base64') };
  const result = execFileSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-Command', script],
    { env, encoding: 'utf-8', windowsHide: true, timeout: 10000 },
  );
  return Buffer.from(result.trim(), 'base64');
}

function decryptCookieValue(encrypted: Buffer, masterKey: Buffer): string | null {
  // Format: "v10" + 12-byte nonce + ciphertext + 16-byte tag
  if (encrypted.length < 3 + 12 + 16) return null;
  const prefix = encrypted.slice(0, 3).toString('utf-8');
  if (prefix !== 'v10' && prefix !== 'v11') {
    // v20+ uses Chrome's app-bound encryption (additional layer); not handled.
    return null;
  }
  const nonce = encrypted.slice(3, 3 + 12);
  const tag = encrypted.slice(encrypted.length - 16);
  const ciphertext = encrypted.slice(3 + 12, encrypted.length - 16);
  try {
    const decipher = crypto.createDecipheriv('aes-256-gcm', masterKey, nonce);
    decipher.setAuthTag(tag);
    const plain = Buffer.concat([decipher.update(ciphertext), decipher.final()]);
    return plain.toString('utf-8');
  } catch {
    return null;
  }
}

/** Chrome stores timestamps as microseconds since 1601-01-01 UTC. */
function chromeTimeToUnixSeconds(chromeMicros: number): number {
  if (!chromeMicros || chromeMicros <= 0) return 0;
  // 11644473600 = seconds between 1601-01-01 and 1970-01-01
  return chromeMicros / 1_000_000 - 11644473600;
}

interface CookieRow {
  host_key: string;
  name: string;
  encrypted_value: Buffer;
  value: string;
  path: string;
  expires_utc: number;
  is_secure: number;
  is_httponly: number;
  samesite: number;
}

// ── CDP path ──────────────────────────────────────────────────────────────
//
// Chrome's v20 cookies aren't decryptable from a sibling process. The
// workaround is to ask Chrome itself: launch a headless Chrome pointed at the
// user's real profile, then read cookies over Chrome DevTools Protocol — no
// decryption math needed because Chrome's own code unseals them.

function findChromeExecutable(): string | null {
  if (process.platform !== 'win32') return null;
  const candidates = [
    path.join(
      process.env['PROGRAMFILES'] || 'C:\\Program Files',
      'Google',
      'Chrome',
      'Application',
      'chrome.exe',
    ),
    path.join(
      process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)',
      'Google',
      'Chrome',
      'Application',
      'chrome.exe',
    ),
    path.join(process.env['LOCALAPPDATA'] || '', 'Google', 'Chrome', 'Application', 'chrome.exe'),
  ];
  return candidates.find((p) => p && fs.existsSync(p)) ?? null;
}

function findEdgeExecutable(): string | null {
  if (process.platform !== 'win32') return null;
  const candidates = [
    path.join(
      process.env['PROGRAMFILES'] || 'C:\\Program Files',
      'Microsoft',
      'Edge',
      'Application',
      'msedge.exe',
    ),
    path.join(
      process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)',
      'Microsoft',
      'Edge',
      'Application',
      'msedge.exe',
    ),
    path.join(process.env['LOCALAPPDATA'] || '', 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
  ];
  return candidates.find((p) => p && fs.existsSync(p)) ?? null;
}

function isProcessRunning(imageName: string): boolean {
  if (process.platform !== 'win32') return false;
  try {
    const out = execFileSync(
      'tasklist',
      ['/fi', `imagename eq ${imageName}`, '/fo', 'csv', '/nh'],
      {
        encoding: 'utf-8',
        timeout: 5000,
        windowsHide: true,
      },
    );
    return new RegExp(imageName.replace('.', '\\.'), 'i').test(out);
  } catch {
    return false;
  }
}

/** Check Chrome's policy hives for the flags that block remote debugging.
 *  Returns a human-readable description of the offending policies or null. */
function chromeDebuggingPolicyBlock(): string | null {
  if (process.platform !== 'win32') return null;
  const policyKeys = [
    'HKLM\\SOFTWARE\\Policies\\Google\\Chrome',
    'HKCU\\SOFTWARE\\Policies\\Google\\Chrome',
    'HKLM\\SOFTWARE\\Policies\\Chromium',
  ];
  const interesting = [
    'RemoteDebuggingAllowed',
    'DeveloperToolsAvailability',
    'DeveloperToolsDisabled',
  ];
  const findings: string[] = [];
  for (const key of policyKeys) {
    try {
      const out = execFileSync('reg', ['query', key], {
        encoding: 'utf-8',
        timeout: 3000,
        windowsHide: true,
      });
      for (const name of interesting) {
        const m = new RegExp(`${name}\\s+REG_DWORD\\s+0x([0-9a-f]+)`, 'i').exec(out);
        if (m) {
          const val = parseInt(m[1], 16);
          // DeveloperToolsAvailability: 0/1 = allowed, 2 = disabled
          // RemoteDebuggingAllowed: 0 = disabled, 1 = allowed
          // DeveloperToolsDisabled: 1 = disabled
          if (
            (name === 'DeveloperToolsAvailability' && val === 2) ||
            (name === 'RemoteDebuggingAllowed' && val === 0) ||
            (name === 'DeveloperToolsDisabled' && val === 1)
          ) {
            findings.push(`${key}\\${name}=${val}`);
          }
        }
      }
    } catch {
      // Key doesn't exist — nothing to enforce.
    }
  }
  return findings.length ? findings.join(', ') : null;
}

function pickPort(): number {
  // Pick a random port in the dynamic range; not bulletproof but the headless
  // Chrome process is short-lived and only listening on loopback.
  return 30000 + Math.floor(Math.random() * 25000);
}

async function waitFor<T>(
  fn: () => Promise<T | null>,
  timeoutMs: number,
  intervalMs: number,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let lastErr: any;
  while (Date.now() < deadline) {
    try {
      const v = await fn();
      if (v !== null) return v;
    } catch (e) {
      lastErr = e;
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error(`timed out: ${lastErr?.message ?? 'no result'}`);
}

interface CDPCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  expires: number; // unix seconds, -1 = session
  size: number;
  httpOnly: boolean;
  secure: boolean;
  session: boolean;
  sameSite?: 'Strict' | 'Lax' | 'None';
}

/** Recursively copy a directory, skipping anything matching `skip()`. */
function copyDir(src: string, dst: string, skip?: (name: string) => boolean): void {
  if (!fs.existsSync(src)) return;
  fs.mkdirSync(dst, { recursive: true });
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    if (skip?.(entry.name)) continue;
    const s = path.join(src, entry.name);
    const d = path.join(dst, entry.name);
    if (entry.isDirectory()) {
      copyDir(s, d, skip);
    } else if (entry.isFile()) {
      try {
        fs.copyFileSync(s, d);
      } catch {}
    }
  }
}

/** Copy just the files Chrome needs to launch a profile and decrypt cookies.
 *  Avoids the multi-hundred-MB cache and history copies. */
function stageProfileCopy(realUserDataDir: string): string {
  const stagedRoot = path.join(os.tmpdir(), `wks-chrome-stage-${Date.now()}`);
  const stagedDefault = path.join(stagedRoot, 'Default');
  fs.mkdirSync(stagedDefault, { recursive: true });

  // `Local State` has the os_crypt encrypted_key and app_bound_encrypted_key.
  // Without it Chrome can't unwrap any cookie encryption.
  fs.copyFileSync(path.join(realUserDataDir, 'Local State'), path.join(stagedRoot, 'Local State'));

  // `Preferences` is required for Chrome to recognise the profile dir as
  // valid. Without it, Chrome runs through first-run setup.
  const prefsSrc = path.join(realUserDataDir, 'Default', 'Preferences');
  if (fs.existsSync(prefsSrc)) {
    fs.copyFileSync(prefsSrc, path.join(stagedDefault, 'Preferences'));
  }
  // Secure Preferences validates Preferences integrity; copy if present.
  const securePrefsSrc = path.join(realUserDataDir, 'Default', 'Secure Preferences');
  if (fs.existsSync(securePrefsSrc)) {
    fs.copyFileSync(securePrefsSrc, path.join(stagedDefault, 'Secure Preferences'));
  }

  // Copy the entire Default/Network/ directory so we get Cookies, Cookies-wal,
  // Cookies-shm, NetworkDataMigrated, TransportSecurity, etc. — Chrome's
  // network service won't see the latest cookie writes without the WAL files.
  const netSrc = path.join(realUserDataDir, 'Default', 'Network');
  if (fs.existsSync(netSrc)) {
    copyDir(netSrc, path.join(stagedDefault, 'Network'));
  }
  // Older Chrome put cookies at Default/Cookies — copy if present.
  const oldCookies = path.join(realUserDataDir, 'Default', 'Cookies');
  if (fs.existsSync(oldCookies)) {
    fs.copyFileSync(oldCookies, path.join(stagedDefault, 'Cookies'));
    if (fs.existsSync(oldCookies + '-journal')) {
      fs.copyFileSync(oldCookies + '-journal', path.join(stagedDefault, 'Cookies-journal'));
    }
  }
  return stagedRoot;
}

async function fetchCookiesViaCDP(opts: {
  userDataDir: string;
  browserExe: string;
  headlessMode: 'new' | 'old' | 'off';
  profileDirectory?: string;
}): Promise<CDPCookie[]> {
  const port = pickPort();
  const tmpDir = path.join(os.tmpdir(), `wks-chrome-cdp-${Date.now()}`);
  fs.mkdirSync(tmpDir, { recursive: true });

  // Chrome refuses to enable remote debugging on its DEFAULT user-data-dir
  // (string-compare against AppData\Local\Google\Chrome\User Data). A
  // directory junction at a non-default path that resolves to the real
  // profile fools the path check while letting Chrome read the real cookie
  // store as-is. Falling back to a partial profile copy if junction creation
  // fails (e.g. on non-NTFS or no permission).
  let stagedProfile: string;
  let cleanupAsJunction = false;
  try {
    const junctionDir = path.join(os.tmpdir(), `wks-chrome-link-${Date.now()}`);
    execFileSync('cmd.exe', ['/c', 'mklink', '/J', junctionDir, opts.userDataDir], {
      windowsHide: true,
      timeout: 5000,
      stdio: 'ignore',
    });
    stagedProfile = junctionDir;
    cleanupAsJunction = true;
    console.log(
      `[chromeCookieImport] using directory junction ${junctionDir} -> ${opts.userDataDir}`,
    );
  } catch (err: any) {
    console.warn(
      '[chromeCookieImport] junction creation failed, falling back to staged copy:',
      err?.message,
    );
    stagedProfile = stageProfileCopy(opts.userDataDir);
  }

  const headlessArg =
    opts.headlessMode === 'new'
      ? ['--headless=new']
      : opts.headlessMode === 'old'
        ? ['--headless']
        : [];

  const profileArg = opts.profileDirectory ? [`--profile-directory=${opts.profileDirectory}`] : [];
  const args = [
    `--user-data-dir=${stagedProfile}`,
    ...profileArg,
    `--disk-cache-dir=${tmpDir}`,
    `--remote-debugging-port=${port}`,
    ...headlessArg,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-extensions',
    '--disable-default-apps',
    '--disable-component-update',
    '--disable-background-networking',
    '--disable-sync',
    '--mute-audio',
    'about:blank',
  ];
  console.log(
    `[chromeCookieImport] spawning ${path.basename(opts.browserExe)} (headless=${opts.headlessMode}, port=${port})`,
  );
  const child = spawn(opts.browserExe, args, {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });

  let stderrBuf = '';
  let stdoutBuf = '';
  let exitInfo: { code: number | null; signal: NodeJS.Signals | null } | null = null;
  child.stdout?.on('data', (d) => {
    const s = d.toString();
    stdoutBuf += s;
    // Print live so a hang lets the user see Chrome's banner / policy errors.
    process.stdout.write(`[chrome-stdout] ${s}`);
  });
  child.stderr?.on('data', (d) => {
    const s = d.toString();
    stderrBuf += s;
    process.stderr.write(`[chrome-stderr] ${s}`);
  });
  child.on('exit', (code, signal) => {
    exitInfo = { code, signal };
  });

  let killed = false;
  const killChrome = () => {
    if (killed) return;
    killed = true;
    try {
      child.kill();
    } catch {}
    if (process.platform === 'win32') {
      try {
        execFileSync('taskkill', ['/F', '/T', '/PID', String(child.pid)], {
          windowsHide: true,
          timeout: 3000,
        });
      } catch {}
    }
  };

  try {
    // Poll /json/version until Chrome is ready (usually <1s). If Chrome
    // exits before the port opens, surface that immediately with whatever
    // it printed.
    const versionInfo = await waitFor(
      async () => {
        if (exitInfo) {
          throw new Error(
            `chrome.exe exited (code=${exitInfo.code} signal=${exitInfo.signal}) before opening the debug port. ` +
              `stderr: ${stderrBuf.trim().slice(0, 500)} stdout: ${stdoutBuf.trim().slice(0, 200)}`,
          );
        }
        const res = await fetch(`http://127.0.0.1:${port}/json/version`);
        if (!res.ok) return null;
        return res.json() as Promise<{ webSocketDebuggerUrl: string }>;
      },
      15000,
      250,
    );

    const cookies = await new Promise<CDPCookie[]>((resolve, reject) => {
      const ws = new WebSocket(versionInfo.webSocketDebuggerUrl);
      const timeout = setTimeout(() => {
        try {
          ws.close();
        } catch {}
        reject(new Error('CDP getAllCookies timed out'));
      }, 15000);

      // Multi-step CDP: open a page so the network service initialises and
      // reads the cookie store, THEN ask Storage for everything. Chrome's
      // network service lazy-loads cookies — without a page target, the
      // store can be reported as empty.
      ws.on('open', () => {
        ws.send(
          JSON.stringify({ id: 1, method: 'Target.createTarget', params: { url: 'about:blank' } }),
        );
      });
      let cookiesResolved = false;
      ws.on('message', (data: Buffer | string) => {
        try {
          const msg = JSON.parse(typeof data === 'string' ? data : data.toString('utf-8'));
          if (msg.id === 1) {
            if (msg.error) {
              clearTimeout(timeout);
              try {
                ws.close();
              } catch {}
              return reject(new Error(`Target.createTarget failed: ${msg.error.message}`));
            }
            // Give Chrome ~250ms to actually load the network service for
            // the new target, then ask for cookies.
            setTimeout(() => {
              ws.send(JSON.stringify({ id: 2, method: 'Storage.getCookies' }));
            }, 250);
            return;
          }
          if (msg.id === 2 && !cookiesResolved) {
            cookiesResolved = true;
            clearTimeout(timeout);
            try {
              ws.close();
            } catch {}
            if (msg.error)
              return reject(new Error(`Storage.getCookies failed: ${msg.error.message}`));
            resolve((msg.result?.cookies ?? []) as CDPCookie[]);
          }
        } catch (err) {
          clearTimeout(timeout);
          reject(err);
        }
      });
      ws.on('error', (err: Error) => {
        clearTimeout(timeout);
        reject(err);
      });
    });

    return cookies;
  } finally {
    killChrome();
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {}
    if (cleanupAsJunction) {
      // Remove just the junction, NOT its target. `rmdir` deletes the
      // junction entry without recursing into the real User Data dir.
      try {
        execFileSync('cmd.exe', ['/c', 'rmdir', stagedProfile], {
          windowsHide: true,
          timeout: 3000,
          stdio: 'ignore',
        });
      } catch {}
    } else {
      try {
        fs.rmSync(stagedProfile, { recursive: true, force: true });
      } catch {}
    }
  }
}

type BrowserKind = 'chrome' | 'edge';

interface BrowserSpec {
  kind: BrowserKind;
  exe: string;
  userDataDir: string;
  imageName: string;
}

function resolveBrowser(kind: BrowserKind): BrowserSpec {
  if (kind === 'edge') {
    const exe = findEdgeExecutable();
    if (!exe)
      throw new Error('Could not find msedge.exe. Microsoft Edge is required for this option.');
    const userDataDir = edgeUserDataDir();
    if (!userDataDir || !fs.existsSync(userDataDir)) {
      throw new Error(`Edge user data directory not found (looked at: ${userDataDir}).`);
    }
    return { kind: 'edge', exe, userDataDir, imageName: 'msedge.exe' };
  }
  const exe = findChromeExecutable();
  if (!exe) throw new Error('Could not find chrome.exe. Install Google Chrome or use Edge.');
  const userDataDir = chromeUserDataDir();
  if (!userDataDir || !fs.existsSync(userDataDir)) {
    throw new Error(`Chrome user data directory not found (looked at: ${userDataDir}).`);
  }
  return { kind: 'chrome', exe, userDataDir, imageName: 'chrome.exe' };
}

/** All profile subdirectories in this browser's user data dir, with the
 *  last-used one (per Local State) sorted first. */
function listProfiles(userDataDir: string): string[] {
  const profiles: string[] = [];
  let lastUsed: string | undefined;
  try {
    const localState = JSON.parse(fs.readFileSync(path.join(userDataDir, 'Local State'), 'utf-8'));
    lastUsed = localState?.profile?.last_used;
    const cache = localState?.profile?.info_cache ?? {};
    for (const dir of Object.keys(cache)) {
      if (fs.existsSync(path.join(userDataDir, dir))) profiles.push(dir);
    }
  } catch {}
  // Fall back to scanning the dir if Local State didn't pan out.
  if (profiles.length === 0) {
    try {
      for (const entry of fs.readdirSync(userDataDir, { withFileTypes: true })) {
        if (!entry.isDirectory()) continue;
        if (entry.name === 'Default' || /^Profile \d+$/.test(entry.name)) {
          profiles.push(entry.name);
        }
      }
    } catch {}
  }
  // Sort so last-used comes first.
  if (lastUsed && profiles.includes(lastUsed)) {
    return [lastUsed, ...profiles.filter((p) => p !== lastUsed)];
  }
  return profiles;
}

/** Import via the CDP path. Returns the same shape as the direct importer. */
export async function importChromeCookiesViaCDP(
  opts: ImportOptions & { browser?: BrowserKind } = {},
): Promise<ImportResult> {
  if (process.platform !== 'win32') {
    throw new Error('CDP-based cookie import is Windows-only in this build.');
  }
  const browser = resolveBrowser(opts.browser ?? 'chrome');
  if (isProcessRunning(browser.imageName)) {
    throw new Error(
      `${browser.kind === 'edge' ? 'Microsoft Edge' : 'Google Chrome'} is currently running. Close it and try again — the CDP import needs to launch a temporary headless instance using your real profile.`,
    );
  }

  if (browser.kind === 'chrome') {
    const policyBlock = chromeDebuggingPolicyBlock();
    if (policyBlock) {
      throw new Error(
        `Chrome's remote-debugging is disabled by enterprise policy (${policyBlock}). Try Edge instead.`,
      );
    }
  }

  // Iterate every profile in the browser's user data dir (Default, Profile 1,
  // Profile 2, …). Each profile is its own cookie store; for browsers with
  // separate work/personal profiles, the cookies you want often live in a
  // non-Default profile.
  const profiles = listProfiles(browser.userDataDir);
  if (profiles.length === 0) profiles.push('Default');
  console.log(`[chromeCookieImport] will sweep profiles: ${profiles.join(', ')}`);

  const allCookies: CDPCookie[] = [];
  const seen = new Set<string>();
  const perProfile: Record<string, number> = {};
  let modeUsed: 'new' | 'old' = 'old';
  let lastError: any = null;
  for (const profile of profiles) {
    let cookies: CDPCookie[] = [];
    try {
      cookies = await fetchCookiesViaCDP({
        browserExe: browser.exe,
        userDataDir: browser.userDataDir,
        headlessMode: 'old',
        profileDirectory: profile,
      });
      modeUsed = 'old';
    } catch (errOld: any) {
      try {
        cookies = await fetchCookiesViaCDP({
          browserExe: browser.exe,
          userDataDir: browser.userDataDir,
          headlessMode: 'new',
          profileDirectory: profile,
        });
        modeUsed = 'new';
      } catch (errNew: any) {
        console.warn(`[chromeCookieImport] profile ${profile} both modes failed:`, errNew?.message);
        lastError = errNew;
        continue;
      }
    }
    perProfile[profile] = cookies.length;
    console.log(`[chromeCookieImport] profile=${profile} returned ${cookies.length} cookies`);
    for (const c of cookies) {
      const key = `${c.domain}|${c.path}|${c.name}`;
      if (seen.has(key)) continue;
      seen.add(key);
      allCookies.push(c);
    }
  }
  if (allCookies.length === 0 && lastError) {
    throw new Error(
      `CDP launch failed across all profiles. Last error: ${lastError?.message ?? lastError}`,
    );
  }
  const cookies = allCookies;
  console.log(
    `[chromeCookieImport] CDP total ${cookies.length} unique cookies (browser=${browser.kind}, profiles=${profiles.length})`,
  );

  const result: ImportResult = {
    imported: 0,
    skipped: 0,
    errors: [],
    diagnostics: {
      source: 'cdp',
      browser: browser.kind,
      headless: modeUsed,
      total: cookies.length,
      ...Object.fromEntries(Object.entries(perProfile).map(([p, n]) => [`profile_${p}`, n])),
    },
  };
  const electronSession = session.fromPartition(PARTITION);

  const filter = opts.domainFilter && opts.domainFilter.length > 0 ? opts.domainFilter : null;
  const setErrorCounts: Record<string, number> = {};
  for (const c of cookies) {
    if (filter && !filter.some((d) => c.domain.includes(d))) {
      result.skipped++;
      continue;
    }
    const isWildcard = c.domain.startsWith('.');
    const hostNoDot = isWildcard ? c.domain.slice(1) : c.domain;
    const scheme = c.secure ? 'https://' : 'http://';
    const url = `${scheme}${hostNoDot}${c.path.startsWith('/') ? c.path : '/' + c.path}`;
    const sameSite: 'unspecified' | 'no_restriction' | 'lax' | 'strict' = !c.sameSite
      ? 'unspecified'
      : c.sameSite === 'None'
        ? 'no_restriction'
        : c.sameSite === 'Lax'
          ? 'lax'
          : 'strict';
    try {
      await electronSession.cookies.set({
        url,
        name: c.name,
        value: c.value,
        domain: isWildcard ? c.domain : hostNoDot,
        path: c.path || '/',
        secure: !!c.secure,
        httpOnly: !!c.httpOnly,
        expirationDate: !c.session && c.expires > 0 ? c.expires : undefined,
        sameSite,
      });
      result.imported++;
    } catch (err: any) {
      result.skipped++;
      const tag = (err?.message ?? String(err)).split('\n')[0].slice(0, 80);
      setErrorCounts[tag] = (setErrorCounts[tag] || 0) + 1;
      if (result.errors.length < 5) {
        result.errors.push(`${c.domain}/${c.name}: ${err?.message ?? String(err)}`);
      }
    }
  }
  if (Object.keys(setErrorCounts).length > 0 && result.diagnostics) {
    for (const [tag, n] of Object.entries(setErrorCounts)) {
      result.diagnostics[`set_err:${tag}`] = n;
    }
  }
  return result;
}

export async function importChromeCookies(opts: ImportOptions = {}): Promise<ImportResult> {
  const userDataDir = chromeUserDataDir();
  if (!userDataDir || !fs.existsSync(userDataDir)) {
    throw new Error(`Chrome user data directory not found (looked at: ${userDataDir})`);
  }

  const masterKey = readLocalStateMasterKey(userDataDir);

  // Cookies DB lives in Default/Network/Cookies (newer Chrome) or Default/Cookies (older).
  // Copy to temp first so we don't fight Chrome for the file lock.
  const candidates = [
    path.join(userDataDir, 'Default', 'Network', 'Cookies'),
    path.join(userDataDir, 'Default', 'Cookies'),
  ];
  const cookiesPath = candidates.find((p) => fs.existsSync(p));
  if (!cookiesPath) {
    throw new Error(`No Cookies DB found under ${userDataDir}/Default`);
  }
  const tmp = path.join(os.tmpdir(), `workspacer-chrome-cookies-${Date.now()}.db`);
  fs.copyFileSync(cookiesPath, tmp);

  const diagnostics: Record<string, number> = {};
  const bump = (k: string) => {
    diagnostics[k] = (diagnostics[k] || 0) + 1;
  };
  const result: ImportResult = { imported: 0, skipped: 0, errors: [], diagnostics };
  try {
    const db = new Database(tmp, { readonly: true, fileMustExist: true });

    let sql =
      'SELECT host_key, name, encrypted_value, value, path, expires_utc, is_secure, is_httponly, samesite FROM cookies';
    const params: any[] = [];
    if (opts.domainFilter && opts.domainFilter.length > 0) {
      const clauses = opts.domainFilter.map(() => 'host_key LIKE ?').join(' OR ');
      sql += ` WHERE ${clauses}`;
      for (const d of opts.domainFilter) params.push(`%${d}%`);
    }

    const rows = db.prepare(sql).all(...params) as CookieRow[];
    db.close();

    const electronSession = session.fromPartition(PARTITION);
    for (const row of rows) {
      let value = row.value;
      if (!value && row.encrypted_value && row.encrypted_value.length > 0) {
        const prefix = row.encrypted_value.slice(0, 3).toString('utf-8');
        bump(`prefix:${prefix.replace(/[^\w]/g, '?')}`);
        const decrypted = decryptCookieValue(row.encrypted_value, masterKey);
        if (decrypted === null) {
          bump('decrypt_failed');
          result.skipped++;
          continue;
        }
        value = decrypted;
        bump('decrypt_ok');
      } else if (value) {
        bump('plaintext');
      }
      if (value === undefined || value === null) {
        bump('no_value');
        result.skipped++;
        continue;
      }

      // Build a URL that matches the cookie's host + path so Electron knows
      // which origin to attach it to. host_key starts with "." for
      // wildcard / subdomain cookies; we strip that for the URL but keep
      // it on `domain` so the wildcard semantics survive.
      const isWildcard = row.host_key.startsWith('.');
      const hostNoDot = isWildcard ? row.host_key.slice(1) : row.host_key;
      const scheme = row.is_secure ? 'https://' : 'http://';
      const url = `${scheme}${hostNoDot}${row.path.startsWith('/') ? row.path : '/' + row.path}`;

      const expirationDate = chromeTimeToUnixSeconds(row.expires_utc);
      const sameSite: 'unspecified' | 'no_restriction' | 'lax' | 'strict' =
        row.samesite === -1
          ? 'unspecified'
          : row.samesite === 0
            ? 'no_restriction'
            : row.samesite === 1
              ? 'lax'
              : 'strict';

      try {
        await electronSession.cookies.set({
          url,
          name: row.name,
          value,
          domain: isWildcard ? row.host_key : hostNoDot,
          path: row.path || '/',
          secure: !!row.is_secure,
          httpOnly: !!row.is_httponly,
          expirationDate: expirationDate > 0 ? expirationDate : undefined,
          sameSite,
        });
        result.imported++;
      } catch (err: any) {
        result.skipped++;
        // Cap stored errors so we don't blow up if 5000 cookies all fail.
        if (result.errors.length < 5) {
          result.errors.push(`${row.host_key}/${row.name}: ${err?.message ?? String(err)}`);
        }
      }
    }
  } finally {
    try {
      fs.unlinkSync(tmp);
    } catch {}
  }

  return result;
}
