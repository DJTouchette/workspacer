/**
 * The `usage.pollOnBoot` checkbox — the user-facing end of the boot-time
 * account-usage poll.
 *
 * Two claims, and the first is the one a config toggle usually gets wrong: the
 * default. The key ships true in config_defaults.json, but Settings renders
 * from whatever the merged config hands it, and a config written before the key
 * existed carries no `usage` section at all. A `?? false` there would show
 * every existing install a switched-off control for a feature that is running.
 *
 * The second is the patch shape. `save` deep-merges, and the section is its own
 * top-level block — writing it under `claude` (where keep-warm lives, and where
 * this control sits on screen) would persist a key nothing reads.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import React from 'react';
import SessionSection from '../../src/components/settings/SessionSection';
import { __resetProviderDetectionCache } from '../../src/hooks/useProviderDetection';
import type { Config } from '../../src/hooks/useConfig';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

beforeEach(() => {
  cleanup();
  __resetProviderDetectionCache();
  api.claudeListModels = vi.fn().mockResolvedValue({ aliases: [], seen: [], defaultModel: '' });
  api.providerListModels = vi.fn().mockResolvedValue([]);
  api.keepWarmHeartbeats = vi.fn().mockResolvedValue([]);
  api.providerCheckAll = vi.fn().mockResolvedValue([]);
});

const LABEL = 'Poll account usage on start';

function renderSection(config: Partial<Config>) {
  const save = vi.fn().mockResolvedValue({});
  render(<SessionSection config={config as Config} save={save} />);
  return { save, box: () => screen.getByLabelText(LABEL) as HTMLInputElement };
}

describe('SessionSection — poll account usage on start', () => {
  it('is on for a config that predates the key', () => {
    const { box } = renderSection({});
    expect(box().checked).toBe(true);
  });

  it('is on for an explicit true', () => {
    expect(renderSection({ usage: { pollOnBoot: true } }).box().checked).toBe(true);
  });

  it('is off for an explicit false', () => {
    expect(renderSection({ usage: { pollOnBoot: false } }).box().checked).toBe(false);
  });

  it('writes usage.pollOnBoot — the section claudemon’s spawn env is read from', () => {
    const { save, box } = renderSection({ usage: { pollOnBoot: true } });
    fireEvent.click(box());
    expect(save).toHaveBeenCalledWith({ usage: { pollOnBoot: false } });
  });

  it('turns it back on without disturbing the rest of the section', () => {
    const { save, box } = renderSection({ usage: { pollOnBoot: false } });
    fireEvent.click(box());
    expect(save).toHaveBeenCalledWith({ usage: { pollOnBoot: true } });
    // Not nested under claude, where the neighbouring keep-warm control writes.
    expect(save.mock.calls[0][0]).not.toHaveProperty('claude');
  });
});
