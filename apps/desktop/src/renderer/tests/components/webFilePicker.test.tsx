import React from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import WebFolderPicker from '../../src/components/WebFolderPicker';

afterEach(cleanup);
it('selects server file paths without invoking a local upload picker', async () => {
  const resolve=vi.fn();
  window.electronAPI={fsListDir:vi.fn().mockResolvedValue({path:'/server/project',home:'/server',parent:'/server',dirs:[]}),filePickerList:vi.fn().mockResolvedValue({path:'/server/project',parent:'/server',home:'/server',entries:[{name:'source.ts',path:'/server/project/source.ts',isDir:false}]})} as any;
  render(<WebFolderPicker/>);
  act(()=>window.dispatchEvent(new CustomEvent('web:pick-files',{detail:{defaultPath:'/server/project',resolve}})));
  fireEvent.click(await screen.findByLabelText('source.ts'));
  fireEvent.click(screen.getByText('Use selected files'));
  expect(resolve).toHaveBeenCalledWith(['/server/project/source.ts']);
});
it('settles the pending chooser when the user cancels', async () => {
  const resolve=vi.fn();
  window.electronAPI={fsListDir:vi.fn().mockResolvedValue({path:'/server',home:'/server',parent:'/',dirs:[]})} as any;
  render(<WebFolderPicker/>);
  act(()=>window.dispatchEvent(new CustomEvent('web:pick-folder',{detail:{resolve}})));
  fireEvent.click(await screen.findByText('Cancel'));
  expect(resolve).toHaveBeenCalledWith(null);
});
