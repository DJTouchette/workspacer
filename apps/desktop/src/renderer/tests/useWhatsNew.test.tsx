import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useWhatsNew, baseVersion, whatsNewBody } from '../src/hooks/useWhatsNew';
import { CHANGELOG } from '../src/lib/changelog.generated';

/**
 * The "what's new" notice, whose failure modes are all annoyance: firing on a
 * fresh install, firing every launch, or never firing at all.
 */

vi.mock('../src/lib/notificationBus', () => ({ postNotification: vi.fn() }));
const posted = async () => vi.mocked((await import('../src/lib/notificationBus')).postNotification);

const KEY = 'wks.lastSeenVersion';

beforeEach(async () => {
  localStorage.clear();
  (await posted()).mockClear();
});

describe('useWhatsNew', () => {
  it('says nothing on a fresh install, but records the version', async () => {
    renderHook(() => useWhatsNew('0.148.0'));
    expect(await posted()).not.toHaveBeenCalled();
    expect(localStorage.getItem(KEY)).toBe('0.148.0');
  });

  it('posts once when the version changed, and not again', async () => {
    localStorage.setItem(KEY, '0.147.0');
    const { rerender } = renderHook(() => useWhatsNew('0.148.0'));
    expect(await posted()).toHaveBeenCalledTimes(1);
    expect((await posted()).mock.calls[0][0].title).toBe('Updated to v0.148.0');

    // A re-render, and a fresh mount (relaunch), must both stay quiet.
    rerender();
    renderHook(() => useWhatsNew('0.148.0'));
    expect(await posted()).toHaveBeenCalledTimes(1);
  });

  it('treats a nightly as its base version, so the stamp does not re-nag daily', async () => {
    localStorage.setItem(KEY, '0.148.0');
    renderHook(() => useWhatsNew('0.148.0-nightly.202608090414'));
    expect(await posted()).not.toHaveBeenCalled();
    expect(baseVersion('0.148.0-nightly.202608090414')).toBe('0.148.0');
  });

  it('says nothing when there is no version yet', async () => {
    renderHook(() => useWhatsNew(''));
    expect(await posted()).not.toHaveBeenCalled();
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('summarizes the release it names', () => {
    const newest = CHANGELOG.find((r) => !r.unreleased)!;
    const body = whatsNewBody(newest.version);
    expect(body).toContain(newest.sections[0].title.toLowerCase());
    // The lead-in bold is unwrapped rather than shown as literal asterisks.
    expect(body).not.toContain('**');
  });

  it('degrades to no body rather than a wrong one for an unknown version', () => {
    expect(whatsNewBody('99.99.99')).toBe('');
  });
});
