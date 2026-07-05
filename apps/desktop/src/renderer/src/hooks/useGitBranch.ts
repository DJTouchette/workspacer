import { useEffect, useState } from 'react';

/**
 * The current git branch of a working directory, for ambient display (fleet
 * cards). Module-level cache with a TTL so a virtualized deck scrolling 50
 * cards doesn't shell out `git status` per mount — one query per cwd per
 * minute, shared across all cards on that cwd. Non-repos (or a missing git
 * backend) cache `null` and stay quiet.
 */
const TTL_MS = 60_000;
const cache = new Map<string, { at: number; branch: string | null }>();
const inflight = new Map<string, Promise<string | null>>();

function fetchBranch(cwd: string): Promise<string | null> {
  const pending = inflight.get(cwd);
  if (pending) return pending;
  const p = Promise.resolve()
    .then(() => window.electronAPI.gitStatus?.(cwd))
    .then((s) => s?.branch ?? null)
    .catch(() => null)
    .then((branch) => {
      cache.set(cwd, { at: Date.now(), branch });
      inflight.delete(cwd);
      return branch;
    });
  inflight.set(cwd, p);
  return p;
}

export function useGitBranch(cwd: string | undefined): string | null {
  const [branch, setBranch] = useState<string | null>(() =>
    cwd ? (cache.get(cwd)?.branch ?? null) : null,
  );

  useEffect(() => {
    if (!cwd) {
      setBranch(null);
      return;
    }
    const cached = cache.get(cwd);
    setBranch(cached?.branch ?? null);
    if (cached && Date.now() - cached.at < TTL_MS) return;
    let alive = true;
    fetchBranch(cwd).then((b) => {
      if (alive) setBranch(b);
    });
    return () => {
      alive = false;
    };
  }, [cwd]);

  return branch;
}
