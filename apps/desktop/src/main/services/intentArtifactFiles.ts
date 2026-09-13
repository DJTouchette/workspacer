import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { runIntentWindowsFiles } from './intentWindowsFiles';

export interface IntentArtifactDirectory {
  leafPath: (leaf: string) => string;
  close: () => void;
}
/** Every component is opened relative to its already-pinned parent. No symlink
 * is followed, including while creating previously missing ancestor directories.
 */
export function openIntentArtifactDirectory(
  directory: string,
  create = false,
): IntentArtifactDirectory {
  if (process.platform !== 'linux')
    throw new Error('Secure artifact storage is not available on this host platform');
  if (
    !path.isAbsolute(directory) ||
    path.resolve(directory) !== directory ||
    directory.includes('\0')
  )
    throw new Error('Invalid artifact directory');
  const flags = fs.constants.O_RDONLY | fs.constants.O_DIRECTORY | fs.constants.O_NOFOLLOW;
  let fd = fs.openSync('/', flags);
  try {
    for (const component of directory.split('/').filter(Boolean)) {
      const nextPath = `/proc/self/fd/${fd}/${component}`;
      let next: number;
      try {
        next = fs.openSync(nextPath, flags);
      } catch (error) {
        if (!create || (error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        try {
          fs.mkdirSync(nextPath, { mode: 0o700 });
        } catch (mkdirError) {
          if ((mkdirError as NodeJS.ErrnoException).code !== 'EEXIST') throw mkdirError;
        }
        next = fs.openSync(nextPath, flags);
      }
      fs.closeSync(fd);
      fd = next;
    }
    const pinned = fd;
    return {
      leafPath: (leaf) => {
        if (!leaf || leaf === '.' || leaf === '..' || /[/\\\0]/.test(leaf))
          throw new Error('Invalid artifact filename');
        return `/proc/self/fd/${pinned}/${leaf}`;
      },
      close: () => fs.closeSync(pinned),
    };
  } catch (error) {
    fs.closeSync(fd);
    throw error;
  }
}
export function withIntentArtifactDirectory<T>(
  directory: string,
  create: boolean,
  run: (access: IntentArtifactDirectory) => T,
): T {
  const access = openIntentArtifactDirectory(directory, create);
  try {
    return run(access);
  } finally {
    access.close();
  }
}
/** Hold both descriptors until the SQLite transaction commits or rolls back. */
export function openIntentArtifactFile(
  directory: string,
  leaf: string,
  write = false,
): { fd: number; close: () => void; remove: () => void } {
  if (!/^[a-f0-9-]{36}\.(?:bin|diff)$/.test(leaf))
    throw new Error('Invalid saved artifact filename');
  const parent = openIntentArtifactDirectory(directory, write);
  try {
    const filename = parent.leafPath(leaf);
    const fd = fs.openSync(
      filename,
      (write
        ? fs.constants.O_WRONLY | fs.constants.O_CREAT | fs.constants.O_EXCL
        : fs.constants.O_RDONLY) | fs.constants.O_NOFOLLOW,
      0o600,
    );
    return {
      fd,
      close: () => {
        try {
          fs.closeSync(fd);
        } finally {
          parent.close();
        }
      },
      remove: () => fs.unlinkSync(filename),
    };
  } catch (error) {
    parent.close();
    throw error;
  }
}

const digest = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
function readDescriptor(fd: number, maxBytes: number): Buffer {
  const stat = fs.fstatSync(fd);
  if (!stat.isFile() || stat.nlink !== 1 || stat.size > maxBytes)
    throw new Error('Artifact exceeds its byte limit or is not an ordinary file');
  const buffer = Buffer.alloc(maxBytes + 1);
  let length = 0;
  while (length < buffer.length) {
    const n = fs.readSync(fd, buffer, length, buffer.length - length, null);
    if (!n) break;
    length += n;
  }
  if (length > maxBytes) throw new Error('Artifact exceeds its byte limit');
  return buffer.subarray(0, length);
}
/** Platform-neutral operations never hand a Windows pathname back to a caller
 * after releasing its native directory locks. Every Windows operation owns its
 * directory/leaf handles until its bounded I/O is complete.
 */
export function readIntentFiles(directory: string, leaves: string[], maxBytes: number): Buffer[] {
  if (
    !Number.isSafeInteger(maxBytes) ||
    maxBytes < 0 ||
    maxBytes > 16 * 1024 * 1024 ||
    leaves.length > 256 ||
    maxBytes * leaves.length > 32 * 1024 * 1024
  )
    throw new Error('Invalid artifact read limit');
  if (!leaves.length) return [];
  if (process.platform === 'win32')
    return runIntentWindowsFiles({
      action: 'read',
      directory,
      leaves,
      limit: maxBytes,
    }).values!.map((value) => Buffer.from(value, 'base64'));
  return withIntentArtifactDirectory(directory, false, (parent) =>
    leaves.map((leaf) => {
      const fd = fs.openSync(
        parent.leafPath(leaf),
        fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW,
      );
      try {
        return readDescriptor(fd, maxBytes);
      } finally {
        fs.closeSync(fd);
      }
    }),
  );
}
export function readIntentFile(directory: string, leaf: string, maxBytes: number): Buffer {
  return readIntentFiles(directory, [leaf], maxBytes)[0];
}

/** New files use an exclusive create. Windows replacements use one exclusive
 * file handle for the precondition and write; partial I/O failures are reported
 * to the caller's already-durable uncertain-write receipt. Linux keeps its
 * existing pinned-directory temporary-file and rename protocol.
 */
export function writeIntentFile(
  directory: string,
  leaf: string,
  bytes: Buffer,
  expectedSha256: string | null = null,
  maxBytes = 512 * 1024,
): void {
  if (bytes.length > maxBytes || maxBytes > 16 * 1024 * 1024 || maxBytes < 0)
    throw new Error('Artifact exceeds its byte limit');
  if (process.platform === 'win32') {
    runIntentWindowsFiles({
      action: 'write',
      directory,
      leaf,
      data: bytes.toString('base64'),
      expected: expectedSha256,
      limit: maxBytes,
    });
    return;
  }
  withIntentArtifactDirectory(directory, true, (parent) => {
    const target = parent.leafPath(leaf);
    const create = (filename: string, mode: number) => {
      const fd = fs.openSync(
        filename,
        fs.constants.O_WRONLY |
          fs.constants.O_CREAT |
          fs.constants.O_EXCL |
          fs.constants.O_NOFOLLOW,
        mode,
      );
      try {
        fs.writeFileSync(fd, bytes);
        fs.fsyncSync(fd);
      } finally {
        fs.closeSync(fd);
      }
    };
    if (expectedSha256 === null) {
      create(target, 0o600);
      return;
    }
    const temporary = parent.leafPath(`intent-${randomUUID()}.tmp`);
    const fd = fs.openSync(target, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    try {
      const before = readDescriptor(fd, maxBytes);
      if (digest(before) !== expectedSha256)
        throw new Error('Project document changed during promotion');
      create(temporary, fs.fstatSync(fd).mode & 0o777);
      const current = fs.openSync(target, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
      try {
        if (digest(readDescriptor(current, maxBytes)) !== expectedSha256)
          throw new Error('Project document changed during promotion');
      } finally {
        fs.closeSync(current);
      }
      fs.renameSync(temporary, target);
    } finally {
      fs.closeSync(fd);
      try {
        fs.unlinkSync(temporary);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      }
    }
  });
}

export function removeIntentFile(
  directory: string,
  leaf: string,
  expectedSha256: string,
  maxBytes = 512 * 1024,
): void {
  if (process.platform === 'win32') {
    runIntentWindowsFiles({
      action: 'remove',
      directory,
      leaf,
      expected: expectedSha256,
      limit: maxBytes,
    });
    return;
  }
  withIntentArtifactDirectory(directory, false, (parent) => {
    const filename = parent.leafPath(leaf);
    const fd = fs.openSync(filename, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    try {
      if (digest(readDescriptor(fd, maxBytes)) !== expectedSha256)
        throw new Error('Artifact changed before cleanup');
      fs.unlinkSync(filename);
    } finally {
      fs.closeSync(fd);
    }
  });
}
