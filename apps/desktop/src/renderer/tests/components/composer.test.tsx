import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { Composer } from '../../src/components/claude/Composer';
import type { AttachedFile } from '../../src/components/claude/fileAttachment';

/**
 * Composer is the send-pipeline's front door: it owns the enabled/disabled
 * state of the submit control and the Enter-vs-Shift+Enter contract that every
 * text send flows through. ClaudePane wires its `onSend` to the claudemon
 * /message path, so these tests pin the exact conditions under which a send
 * fires.
 */

function renderComposer(props: Partial<React.ComponentProps<typeof Composer>> = {}) {
  const onSend = vi.fn();
  const onChange = vi.fn();
  const onPickFiles = vi.fn();
  const onRemoveFile = vi.fn();
  const onSlashPick = vi.fn();
  const attachedFiles: AttachedFile[] = props.attachedFiles ?? [];
  render(
    <Composer
      value={props.value ?? ''}
      onChange={onChange}
      onSend={onSend}
      onPickFiles={onPickFiles}
      attachedFiles={attachedFiles}
      onRemoveFile={onRemoveFile}
      onSlashPick={onSlashPick}
      {...props}
    />,
  );
  return { onSend, onChange, onPickFiles, onRemoveFile, onSlashPick };
}

const SLASH_ITEMS = [
  { id: 'a', label: 'review', hint: 'code review', kind: 'skill' },
  { id: 'b', label: 'refactor', hint: 'clean up', kind: 'skill' },
  { id: 'c', label: 'commit', hint: 'write a message', kind: 'prompt' },
];

const textarea = () => screen.getByRole('textbox') as HTMLTextAreaElement;

describe('Composer', () => {
  it('placeholder names the active agent backend', () => {
    renderComposer({ agentName: 'Codex' });
    expect(textarea().placeholder).toMatch(/Give Codex something to do/);
  });

  it('Enter sends when there is text', () => {
    const { onSend } = renderComposer({ value: 'ship it' });
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('Shift+Enter inserts a newline instead of sending', () => {
    const { onSend } = renderComposer({ value: 'line one' });
    fireEvent.keyDown(textarea(), { key: 'Enter', shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it('does not treat an IME candidate-commit Enter (keyCode 229) as a send', () => {
    const { onSend } = renderComposer({ value: '日本語' });
    // jsdom won't set isComposing/keyCode from the init dict, so drive the
    // nativeEvent the component reads directly.
    const ta = textarea();
    const ev = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true });
    Object.defineProperty(ev, 'isComposing', { value: true });
    Object.defineProperty(ev, 'keyCode', { value: 229 });
    ta.dispatchEvent(ev);
    expect(onSend).not.toHaveBeenCalled();
  });

  it('send button is disabled with an empty composer and no attachments', () => {
    renderComposer({ value: '   ' });
    expect(screen.getByLabelText('Send message')).toBeDisabled();
  });

  it('send button enables once there is non-whitespace text', () => {
    renderComposer({ value: 'hello' });
    const btn = screen.getByLabelText('Send message');
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
  });

  it('clicking the enabled send button fires onSend', () => {
    const { onSend } = renderComposer({ value: 'hello' });
    fireEvent.click(screen.getByLabelText('Send message'));
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('send button enables on attachments alone (no text), and the placeholder flips to the files prompt', () => {
    const files: AttachedFile[] = [{ path: '/repo/a.png', name: 'a.png', label: 'Image' }];
    renderComposer({ value: '', attachedFiles: files, agentName: 'Claude' });
    expect(screen.getByLabelText('Send message')).not.toBeDisabled();
    expect(textarea().placeholder).toMatch(/What should Claude do with these files/);
  });

  it('hides the send button when showSendButton is false (Enter still the send path)', () => {
    renderComposer({ value: 'hi', showSendButton: false });
    expect(screen.queryByLabelText('Send message')).not.toBeInTheDocument();
  });

  it('typing routes through onChange', () => {
    const { onChange } = renderComposer({ value: '' });
    fireEvent.change(textarea(), { target: { value: 'draft' } });
    expect(onChange).toHaveBeenCalledWith('draft');
  });

  it('the attach (+) button opens the file picker', () => {
    const { onPickFiles } = renderComposer();
    fireEvent.click(screen.getByTitle('Attach files'));
    expect(onPickFiles).toHaveBeenCalledTimes(1);
  });
});

describe('Composer / command picker', () => {
  it('opens a filtered picker when the input is a bare "/token"', () => {
    renderComposer({ value: '/re', slashItems: SLASH_ITEMS });
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    const opts = screen.getAllByRole('option');
    // "re" matches review + refactor (label prefix), not commit.
    expect(opts).toHaveLength(2);
    expect(opts.map((o) => o.textContent)).toEqual([
      expect.stringContaining('review'),
      expect.stringContaining('refactor'),
    ]);
  });

  it('does not open without slashItems (a leading "/" is just text)', () => {
    const { onSend } = renderComposer({ value: '/review' });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('closes once the token gains a space (arguments) — Enter then sends', () => {
    const { onSend, onSlashPick } = renderComposer({
      value: '/review src',
      slashItems: SLASH_ITEMS,
    });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSlashPick).not.toHaveBeenCalled();
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('Enter picks the highlighted item and does NOT send', () => {
    const { onSend, onSlashPick } = renderComposer({ value: '/re', slashItems: SLASH_ITEMS });
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSlashPick).toHaveBeenCalledWith('a'); // review, first match
    expect(onSend).not.toHaveBeenCalled();
  });

  it('ArrowDown moves the highlight before Enter picks', () => {
    const { onSlashPick } = renderComposer({ value: '/re', slashItems: SLASH_ITEMS });
    fireEvent.keyDown(textarea(), { key: 'ArrowDown' });
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSlashPick).toHaveBeenCalledWith('b'); // refactor, second match
  });

  it('Tab also picks the highlighted item', () => {
    const { onSlashPick } = renderComposer({ value: '/commit', slashItems: SLASH_ITEMS });
    fireEvent.keyDown(textarea(), { key: 'Tab' });
    expect(onSlashPick).toHaveBeenCalledWith('c');
  });

  it('pointerdown on an option picks it', () => {
    const { onSlashPick } = renderComposer({ value: '/', slashItems: SLASH_ITEMS });
    const opts = screen.getAllByRole('option');
    fireEvent.pointerDown(opts[2]);
    expect(onSlashPick).toHaveBeenCalledWith('c');
  });

  it('Escape dismisses the picker; a subsequent Enter falls through to send', () => {
    const { onSend, onSlashPick } = renderComposer({ value: '/re', slashItems: SLASH_ITEMS });
    fireEvent.keyDown(textarea(), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    fireEvent.keyDown(textarea(), { key: 'Enter' });
    expect(onSlashPick).not.toHaveBeenCalled();
    expect(onSend).toHaveBeenCalledTimes(1);
  });
});
