import { expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ExamplesGalleryDialog from '../../src/components/ExamplesGalleryDialog';

it('shows a failed catalog request instead of only an empty gallery', async () => {
  window.electronAPI = {
    listExamplePlugins: vi.fn().mockRejectedValue(new Error('HTTP 401')),
  } as unknown as typeof window.electronAPI;
  render(<ExamplesGalleryDialog installedIds={[]} onClose={() => {}} />);
  expect(await screen.findByText('Could not load examples: HTTP 401')).toBeTruthy();
});
