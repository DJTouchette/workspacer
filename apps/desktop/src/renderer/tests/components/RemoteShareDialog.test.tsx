import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import RemoteShareDialog from '../../src/components/RemoteShareDialog';

vi.mock('qrcode.react', () => ({
  QRCodeSVG: ({ value }: { value: string }) => <div data-testid="qr">{value}</div>,
}));
vi.mock('../../src/components/LinkedMachinesSection', () => ({ default: () => null }));

describe('RemoteShareDialog web pairing', () => {
  it('offers full-control scope and correct phone/full-app links to the owner', async () => {
    window.electronAPI.getRemoteInfo = vi
      .fn()
      .mockResolvedValue({
        enabled: true,
        token: 'owner',
        remoteUrl: 'https://example.test/m',
        appUrl: 'https://example.test/app/',
        busUrl: 'wss://example.test/bus',
        pairingScope: 'operator',
        canManageTokens: true,
        canToggleSharing: false,
      });
    window.electronAPI.remoteTokenGetOrCreate = vi.fn(async (scope) => ({
      token: 'paired-' + scope,
      scope,
      created: '',
    }));
    window.electronAPI.remoteTokensList = vi.fn().mockResolvedValue([]);
    render(<RemoteShareDialog onClose={() => {}} />);
    await screen.findByText('Pairing scope');
    await waitFor(() =>
      expect(screen.getByTestId('qr')).toHaveTextContent(
        'https://example.test/m?token=paired-operator',
      ),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Full app' }));
    expect(screen.getByTestId('qr')).toHaveTextContent(
      'https://example.test/app/?token=paired-operator',
    );
    expect(screen.queryByRole('button', { name: 'Stop sharing' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Read-only/ }));
    await waitFor(() =>
      expect(screen.getByTestId('qr')).toHaveTextContent(
        'https://example.test/m?token=paired-view',
      ),
    );
  });
  it('does not mislabel a triage token as operator or attempt token administration', async () => {
    window.electronAPI.getRemoteInfo = vi
      .fn()
      .mockResolvedValue({
        enabled: true,
        token: 'triage',
        remoteUrl: 'https://example.test/m',
        appUrl: 'https://example.test/app/',
        busUrl: 'wss://example.test/bus',
        pairingScope: 'triage',
        canManageTokens: false,
        canToggleSharing: false,
      });
    const mint = vi.fn();
    window.electronAPI.remoteTokenGetOrCreate = mint;
    window.electronAPI.remoteTokensList = vi.fn();
    render(<RemoteShareDialog onClose={() => {}} />);
    await screen.findByText(/Current access: triage/);
    expect(mint).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Full app' })).not.toBeInTheDocument();
    expect(screen.queryByText('Pairing scope')).not.toBeInTheDocument();
  });
});
