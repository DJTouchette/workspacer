/**
 * Settings → "Usage schedule", the control end of the hub-owned preference.
 *
 * What is worth testing here is not the two buttons; it is the three ways this
 * control can LIE, each of which has bitten this app before:
 *
 *  - showing a default nobody chose when the hub could not be asked,
 *  - reporting a save the hub refused as done,
 *  - leaving the Overview on the previous curve after a successful save, so the
 *    setting reads as inert.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import UsageScheduleRow from '../../src/components/settings/UsageScheduleRow';

const refreshUsageReport = vi.hoisted(() => vi.fn(async () => {}));
vi.mock('../../src/hooks/useUsageReport', () => ({ refreshUsageReport }));

function api(overrides: Record<string, unknown>) {
  (window as unknown as { electronAPI: unknown }).electronAPI = overrides;
}

beforeEach(() => {
  refreshUsageReport.mockClear();
});
afterEach(() => {
  cleanup();
  // The shared setup defines electronAPI non-configurable, so it is reset by
  // assignment rather than deleted.
  api({});
});

describe('UsageScheduleRow', () => {
  it('shows the stored choice and switches it through the hub', async () => {
    const set = vi.fn(async () => ({
      ok: true as const,
      state: { schedule: 'five_day' as const, configurable: true },
    }));
    api({
      usagePacingSchedule: vi.fn(async () => ({ schedule: 'seven_day', configurable: true })),
      setUsagePacingSchedule: set,
    });
    render(<UsageScheduleRow />);

    const week = await screen.findByRole('button', { name: /work week/i });
    const every = screen.getByRole('button', { name: /every day/i });
    await waitFor(() => expect(every).toBeEnabled());

    await userEvent.click(week);
    expect(set).toHaveBeenCalledWith('five_day');
    // The hint must follow the stored value, so the user can see WHICH week
    // they are on without opening the Overview.
    await screen.findByText(/climbs Monday to Friday/i);
    // And the visible report is re-read, or the change looks like it did
    // nothing for up to a minute.
    expect(refreshUsageReport).toHaveBeenCalledOnce();
  });

  it('renders unavailable, not a default, when the hub cannot answer', async () => {
    api({
      usagePacingSchedule: vi.fn(async () => null),
      setUsagePacingSchedule: vi.fn(),
    });
    render(<UsageScheduleRow />);

    await screen.findByText(/does not offer the usage-schedule setting/i);
    for (const name of [/work week/i, /every day/i]) {
      expect(screen.getByRole('button', { name })).toBeDisabled();
    }
    // Nothing may look selected: a highlighted option would assert a schedule
    // this hub never reported.
    expect(screen.queryByText(/climbs Monday to Friday/i)).toBeNull();
  });

  it('says a hub with no preference file cannot store one', async () => {
    api({
      usagePacingSchedule: vi.fn(async () => ({ schedule: '', configurable: false })),
      setUsagePacingSchedule: vi.fn(),
    });
    render(<UsageScheduleRow />);
    await screen.findByText(/started without a preference file/i);
    expect(screen.getByRole('button', { name: /work week/i })).toBeDisabled();
  });

  it('keeps the previous choice and shows the reason when a save fails', async () => {
    const set = vi.fn(async () => ({ ok: false as const, error: 'requires host authority' }));
    api({
      usagePacingSchedule: vi.fn(async () => ({ schedule: 'seven_day', configurable: true })),
      setUsagePacingSchedule: set,
    });
    render(<UsageScheduleRow />);

    const week = await screen.findByRole('button', { name: /work week/i });
    await waitFor(() => expect(week).toBeEnabled());
    await userEvent.click(week);

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toMatch(/requires host authority/);
    // Still on the old schedule, and the Overview was NOT re-read for a change
    // that did not happen.
    await screen.findByText(/rises evenly across all seven days/i);
    expect(refreshUsageReport).not.toHaveBeenCalled();
  });

  it('is inert rather than broken on a client whose preload has no such method', async () => {
    api({});
    render(<UsageScheduleRow />);
    await waitFor(() => expect(screen.getByRole('button', { name: /every day/i })).toBeDisabled());
    await screen.findByText(/does not offer the usage-schedule setting/i);
  });
});
