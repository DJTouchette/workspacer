import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import JobsSection from '../src/components/settings/JobsSection';
import { POWER_DOWN_PLACEHOLDER, POWER_DOWN_COMMAND } from '../src/components/settings/JobsSection';
import type { HubJobView } from '../../main/shared/ipcTypes';

/**
 * The safety rule for the power-down template, at the seam where it actually
 * matters: what Settings -> Jobs hands to jobs.upsert. The command it pre-fills
 * runs a script the user has not written yet, so the job must land switched
 * off, and the one click that would arm it has to be refused while the script
 * path is still a blank.
 */

const api = () => window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

function stubJobs(jobs: HubJobView[]) {
  api().jobsList = vi.fn().mockResolvedValue({ jobs });
  api().jobsUpsert = vi.fn().mockResolvedValue({ ok: true });
  api().jobsHistory = vi.fn().mockResolvedValue({ runs: [] });
  api().jobsRun = vi.fn().mockResolvedValue({ ok: true });
  api().jobsRemove = vi.fn().mockResolvedValue({ ok: true });
}

const placeholderJob: HubJobView = {
  id: 'pd1',
  name: 'Power down when the fleet is quiet',
  enabled: false,
  trigger: { kind: 'interval', everyMinutes: 5 },
  action: { kind: 'shell', shell: { command: POWER_DOWN_COMMAND } },
};

describe('Settings -> Jobs, power-down template', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('creates the job switched off, with the blank still in it', async () => {
    stubJobs([]);
    render(<JobsSection />);

    fireEvent.click(await screen.findByText('Power down when quiet'));

    // The blank is spelled out where the user is looking, not buried.
    expect(await screen.findByText(POWER_DOWN_PLACEHOLDER)).toBeInTheDocument();
    expect(screen.getByText(/Nothing here powers a machine down for you/)).toBeInTheDocument();

    fireEvent.click(screen.getByText('Create'));

    await waitFor(() => expect(api().jobsUpsert).toHaveBeenCalled());
    const sent = api().jobsUpsert.mock.calls[0][0];
    expect(sent.enabled).toBe(false);
    expect(sent.action.shell.command).toContain(POWER_DOWN_PLACEHOLDER);
  });

  it('refuses to enable a job whose script path is still the blank', async () => {
    stubJobs([placeholderJob]);
    render(<JobsSection />);

    const checkbox = await screen.findByTitle('Enable');
    fireEvent.click(checkbox);

    expect(
      await screen.findByText(new RegExp(`This job still runs ${POWER_DOWN_PLACEHOLDER}`)),
    ).toBeInTheDocument();
    expect(api().jobsUpsert).not.toHaveBeenCalled();
  });

  it('enables it once a real script path is in', async () => {
    stubJobs([
      {
        ...placeholderJob,
        action: {
          kind: 'shell',
          shell: { command: 'workspacer fleet quiescence --quiet && /home/me/bin/sleep.sh' },
        },
      },
    ]);
    render(<JobsSection />);

    fireEvent.click(await screen.findByTitle('Enable'));

    await waitFor(() => expect(api().jobsUpsert).toHaveBeenCalled());
    expect(api().jobsUpsert.mock.calls[0][0].enabled).toBe(true);
  });

  it('offers the templates once there are already jobs', async () => {
    // They used to be an empty-state affordance only, which is the one moment
    // nobody needs a second job.
    stubJobs([placeholderJob]);
    render(<JobsSection />);
    expect(await screen.findByText('Start from a template:')).toBeInTheDocument();
    expect(screen.getByText('Power down when quiet')).toBeInTheDocument();
  });
});
