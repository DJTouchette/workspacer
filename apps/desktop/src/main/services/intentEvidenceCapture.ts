import { execFile } from 'node:child_process';
import fs, { realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { gitArgs } from '../lib/gitExec';
import { canonicalRoot, isSecretPath, isWithin } from '../lib/pathConfinement';
import { parsePorcelain } from './gitService';
import type { IntentGitEvidence } from '../shared/intentEvidence';
import { readIntentFile } from './intentArtifactFiles';

export const INTENT_CAPTURE_LIMITS = { bytes: 256 * 1024, files: 100, timeoutMs: 15_000 };
export type IntentGitCapture = Omit<IntentGitEvidence, 'artifactId' | 'sha256' | 'bytes'> & {
  artifact: string;
};
const exec = promisify(execFile);

/** No request body reaches this function: the store resolves a linked LOCAL session. */
export async function captureIntentGit(cwd: string): Promise<IntentGitCapture> {
  if (!path.isAbsolute(cwd) || cwd.includes('\0'))
    throw new Error('Invalid host execution directory');
  const canonicalCwd = realpathSync(cwd);
  const deadline = Date.now() + INTENT_CAPTURE_LIMITS.timeoutMs;
  const baseEnv = { ...process.env };
  for (const key of Object.keys(baseEnv))
    if (key.toUpperCase().startsWith('GIT_')) delete baseEnv[key];
  Object.assign(baseEnv, {
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: process.platform === 'win32' ? 'NUL' : '/dev/null',
    GIT_ATTR_NOSYSTEM: '1',
    GIT_NO_LAZY_FETCH: '1',
    GIT_OPTIONAL_LOCKS: '0',
  });
  let isolatedEnv: NodeJS.ProcessEnv | undefined;
  const git = async (directory: string, args: string[], original = false) => {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('Git evidence capture time limit exceeded');
    const { stdout } = await exec(
      'git',
      ['--no-replace-objects', '--literal-pathspecs', ...gitArgs(args)],
      {
        cwd: directory,
        maxBuffer: INTENT_CAPTURE_LIMITS.bytes,
        timeout: remaining,
        env: original ? baseEnv : (isolatedEnv ?? baseEnv),
        windowsHide: true,
      },
    );
    return stdout;
  };
  const repositoryRoot = realpathSync(
    (await git(canonicalCwd, ['rev-parse', '--show-toplevel'])).trim(),
  );
  const headCommit = (await git(repositoryRoot, ['rev-parse', '--verify', 'HEAD^{commit}'])).trim();
  if (!/^[a-f0-9]{40,64}$/.test(headCommit)) throw new Error('Git HEAD is unavailable');
  const objects = realpathSync(
    (
      await git(repositoryRoot, ['rev-parse', '--path-format=absolute', '--git-path', 'objects'])
    ).trim(),
  );
  const indexPath = (
    await git(repositoryRoot, ['rev-parse', '--path-format=absolute', '--git-path', 'index'])
  ).trim();
  const readIndex = (): Buffer | undefined => {
    if (process.platform === 'win32')
      return readIntentFile(path.dirname(indexPath), path.basename(indexPath), 16 * 1024 * 1024);
    let fd: number;
    try {
      fd = fs.openSync(indexPath, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
      throw error;
    }
    try {
      const stat = fs.fstatSync(fd);
      if (!stat.isFile() || stat.size > 16 * 1024 * 1024)
        throw new Error('Git index exceeds the capture limit');
      const bytes = Buffer.alloc(stat.size + 1);
      let length = 0;
      while (length < bytes.length) {
        const n = fs.readSync(fd, bytes, length, bytes.length - length, null);
        if (!n) break;
        length += n;
      }
      if (length !== stat.size) throw new Error('Git index changed during capture');
      return bytes.subarray(0, length);
    } finally {
      fs.closeSync(fd);
    }
  };
  const index = readIndex();
  const temporary = fs.mkdtempSync(path.join(tmpdir(), 'workspacer-intent-git-'));
  try {
    // Working-tree diffs can run clean/process filters, even with --no-textconv.
    // Freeze minimal metadata instead of trusting mutable repository/global config.
    fs.mkdirSync(path.join(temporary, 'objects'));
    fs.mkdirSync(path.join(temporary, 'refs'));
    fs.writeFileSync(path.join(temporary, 'HEAD'), headCommit + '\n');
    fs.writeFileSync(
      path.join(temporary, 'config'),
      `[core]\nrepositoryformatversion=${headCommit.length === 64 ? 1 : 0}\nbare=false\n${headCommit.length === 64 ? '[extensions]\nobjectFormat=sha256\n' : ''}`,
    );
    if (index) fs.writeFileSync(path.join(temporary, 'index'), index);
    isolatedEnv = {
      ...baseEnv,
      GIT_DIR: temporary,
      GIT_COMMON_DIR: temporary,
      GIT_WORK_TREE: repositoryRoot,
      GIT_INDEX_FILE: path.join(temporary, 'index'),
      GIT_OBJECT_DIRECTORY: objects,
    };
    const statusArgs = [
      'status',
      '--porcelain=v1',
      '-z',
      '--untracked-files=all',
      '--ignore-submodules=all',
    ];
    const status = await git(repositoryRoot, statusArgs);
    const files = parsePorcelain(status);
    if (files.length > INTENT_CAPTURE_LIMITS.files)
      throw new Error('Git evidence exceeds the 100-file capture limit');
    const omissions = [
      'This captures tracked changes against the current HEAD; earlier commits and test results are not verified.',
    ];
    omissions.push(
      'Repository and global Git configuration, content filters, and submodule contents are excluded. Captured working bytes may differ from a filtered representation.',
    );
    const allowed = (file: string) => {
      const absolute = path.resolve(repositoryRoot, file);
      const parent = canonicalRoot(path.dirname(absolute));
      if (
        !file ||
        path.isAbsolute(file) ||
        file.includes('\uFFFD') ||
        !isWithin(absolute, canonicalCwd) ||
        !parent ||
        !isWithin(parent, canonicalCwd)
      )
        return false;
      if (
        file
          .split('/')
          .some((part) =>
            /^(?:\.git|\.env(?:\..*)?|\.ssh|\.aws|credentials(?:\.json)?|id_(?:rsa|ed25519)|.*\.(?:pem|p12|pfx|key))$/i.test(
              part,
            ),
          )
      )
        return false;
      return !isSecretPath(path.join(parent, path.basename(absolute)));
    };
    const changedFiles: string[] = [];
    for (const file of files) {
      if (file.staged === '?' || file.unstaged === '?') {
        omissions.push(`Untracked file content omitted: ${file.path}`);
        continue;
      }
      if (!allowed(file.path) || (file.orig_path && !allowed(file.orig_path))) {
        omissions.push(`Restricted or outside-execution path omitted: ${file.path}`);
        continue;
      }
      changedFiles.push(file.path);
      if (file.orig_path) changedFiles.push(file.orig_path);
    }
    const paths = [...new Set(changedFiles)];
    const diffArgs = [
      'diff',
      '--no-textconv',
      '--no-color',
      '--no-renames',
      '--ignore-submodules=all',
    ];
    const diff = async () => {
      if (!paths.length) return '';
      const staged = await git(repositoryRoot, [
        ...diffArgs,
        '--cached',
        headCommit,
        '--',
        ...paths,
      ]);
      const unstaged = await git(repositoryRoot, [...diffArgs, '--', ...paths]);
      const result = `Staged changes against HEAD ${headCommit}\n${staged}\nUnstaged changes against index\n${unstaged}`;
      if (Buffer.byteLength(result, 'utf8') > INTENT_CAPTURE_LIMITS.bytes)
        throw new Error('Git evidence exceeds the artifact byte limit');
      return result;
    };
    const artifact = await diff();
    // Status alone cannot detect a second edit to an already-dirty file. Compare the bytes too.
    if (
      (await diff()) !== artifact ||
      (await git(repositoryRoot, statusArgs)) !== status ||
      (await git(repositoryRoot, ['rev-parse', '--verify', 'HEAD^{commit}'], true)).trim() !==
        headCommit ||
      !Buffer.from(readIndex() ?? []).equals(Buffer.from(index ?? [])) ||
      realpathSync(cwd) !== canonicalCwd
    )
      throw new Error('Execution changed during evidence capture. Try again when edits settle.');
    return {
      cwd: canonicalCwd,
      repositoryRoot,
      headCommit,
      capturedAt: new Date().toISOString(),
      scope: 'tracked-working-tree-against-head',
      changedFiles: paths,
      omissions,
      artifact,
    };
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}
