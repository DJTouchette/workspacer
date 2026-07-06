import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { ApprovalPrompt } from '../../src/components/claude/ApprovalPrompt';
import { QuestionPicker } from '../../src/components/claude/QuestionPicker';
import { NeedsYouDock } from '../../src/components/claude/NeedsYouDock';
import type { PendingApproval, PendingQuestion } from '../../src/types/claudeSession';

/**
 * The "needs you" surfaces — approval cards and question pickers docked above
 * the composer. These are the exact controls ClaudePane binds to
 * claudeApprove() (approvals) and the PTY answer write (questions), so the
 * tests pin the payloads each control emits.
 */

const approval: PendingApproval = {
  toolName: 'Bash',
  toolInput: { command: 'npm test' },
  timestamp: 1000,
};

const question: PendingQuestion = {
  question: 'Which package manager?',
  header: 'Setup',
  options: [{ label: 'npm', description: 'the default' }, { label: 'pnpm' }],
};

describe('ApprovalPrompt', () => {
  it('renders the tool name and serialized input', () => {
    render(<ApprovalPrompt approval={approval} onRespond={vi.fn()} />);
    expect(screen.getByText('Permission Required: Bash')).toBeInTheDocument();
    expect(screen.getByText(/npm test/)).toBeInTheDocument();
  });

  it('Allow emits "yes"', () => {
    const onRespond = vi.fn();
    render(<ApprovalPrompt approval={approval} onRespond={onRespond} />);
    fireEvent.click(screen.getByText('Allow'));
    expect(onRespond).toHaveBeenCalledWith('yes');
  });

  it('Deny emits "no"', () => {
    const onRespond = vi.fn();
    render(<ApprovalPrompt approval={approval} onRespond={onRespond} />);
    fireEvent.click(screen.getByText('Deny'));
    expect(onRespond).toHaveBeenCalledWith('no');
  });
});

describe('QuestionPicker', () => {
  it('renders the header, prompt, and each option', () => {
    render(<QuestionPicker questions={[question]} onAnswer={vi.fn()} />);
    expect(screen.getByText('Setup')).toBeInTheDocument();
    expect(screen.getByText('Which package manager?')).toBeInTheDocument();
    expect(screen.getByText('npm')).toBeInTheDocument();
    expect(screen.getByText('pnpm')).toBeInTheDocument();
  });

  it('selecting an option answers with its 1-based index', () => {
    const onAnswer = vi.fn();
    render(<QuestionPicker questions={[question]} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByText('pnpm'));
    expect(onAnswer).toHaveBeenCalledWith({ option: 2 });
  });

  it('a single-question picker offers a custom free-text answer', () => {
    const onAnswer = vi.fn();
    render(<QuestionPicker questions={[question]} onAnswer={onAnswer} />);
    const input = screen.getByPlaceholderText('Or type your own answer…');
    fireEvent.change(input, { target: { value: 'yarn' } });
    fireEvent.click(screen.getByText('Send'));
    expect(onAnswer).toHaveBeenCalledWith({ text: 'yarn' });
  });

  it('the custom-answer Send is disabled until text is entered', () => {
    render(<QuestionPicker questions={[question]} onAnswer={vi.fn()} />);
    expect(screen.getByText('Send')).toBeDisabled();
  });

  it('Enter in the custom field submits the trimmed text', () => {
    const onAnswer = vi.fn();
    render(<QuestionPicker questions={[question]} onAnswer={onAnswer} />);
    const input = screen.getByPlaceholderText('Or type your own answer…');
    fireEvent.change(input, { target: { value: '  bun  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onAnswer).toHaveBeenCalledWith({ text: 'bun' });
  });

  it('a multi-question set steps one question at a time and submits answers together', () => {
    const onAnswer = vi.fn();
    const q2: PendingQuestion = {
      question: 'Second?',
      options: [{ label: 'aye' }, { label: 'nay' }],
    };
    render(<QuestionPicker questions={[question, q2]} onAnswer={onAnswer} />);
    // Only the first question is visible, with a progress readout.
    expect(screen.getByText('Which package manager?')).toBeInTheDocument();
    expect(screen.queryByText('Second?')).not.toBeInTheDocument();
    expect(screen.getByText('1 of 2')).toBeInTheDocument();
    // Answering the first advances (no submit yet) …
    fireEvent.click(screen.getByText('pnpm'));
    expect(onAnswer).not.toHaveBeenCalled();
    expect(screen.getByText('Second?')).toBeInTheDocument();
    // … answering the last submits the whole set as raw per-question answers.
    fireEvent.click(screen.getByText('aye'));
    expect(onAnswer).toHaveBeenCalledWith({ answers: ['2', '1'] });
  });

  it('the back chevron revisits an answered question with its pick highlighted', () => {
    const onAnswer = vi.fn();
    const q2: PendingQuestion = { question: 'Second?', options: [{ label: 'aye' }] };
    render(<QuestionPicker questions={[question, q2]} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByText('pnpm'));
    fireEvent.click(screen.getByTitle('Previous question'));
    expect(screen.getByText('Which package manager?')).toBeInTheDocument();
    // Changing the pick and re-answering both questions submits the new set.
    fireEvent.click(screen.getByText('npm'));
    fireEvent.click(screen.getByText('aye'));
    expect(onAnswer).toHaveBeenCalledWith({ answers: ['1', '1'] });
  });

  it('a custom answer mid-set advances like an option pick', () => {
    const onAnswer = vi.fn();
    const q2: PendingQuestion = { question: 'Second?', options: [{ label: 'aye' }] };
    render(<QuestionPicker questions={[question, q2]} onAnswer={onAnswer} />);
    const input = screen.getByPlaceholderText('Or type your own answer…');
    fireEvent.change(input, { target: { value: 'yarn' } });
    fireEvent.click(screen.getByText('Next'));
    expect(onAnswer).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('aye'));
    expect(onAnswer).toHaveBeenCalledWith({ answers: ['yarn', '1'] });
  });
});

describe('NeedsYouDock', () => {
  it('renders nothing when neither an approval nor a question is pending', () => {
    const { container } = render(
      <NeedsYouDock approval={null} questions={null} onApprove={vi.fn()} onAnswer={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('shows the approval card and routes Allow to onApprove', () => {
    const onApprove = vi.fn();
    render(
      <NeedsYouDock
        approval={approval}
        questions={null}
        onApprove={onApprove}
        onAnswer={vi.fn()}
      />,
    );
    expect(screen.getByText('Approval needed')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Allow'));
    expect(onApprove).toHaveBeenCalledWith('yes');
  });

  it('a pending question wins over a racing approval (picker shown, approval hidden)', () => {
    render(
      <NeedsYouDock
        approval={approval}
        questions={[question]}
        onApprove={vi.fn()}
        onAnswer={vi.fn()}
      />,
    );
    expect(screen.getByText('Claude is asking you')).toBeInTheDocument();
    expect(screen.getByText('Which package manager?')).toBeInTheDocument();
    // The stale approval card must not render underneath the picker.
    expect(screen.queryByText('Permission Required: Bash')).not.toBeInTheDocument();
  });

  it('labels the count when several questions are docked together', () => {
    const q2: PendingQuestion = { question: 'Second?', options: [{ label: 'a' }] };
    render(
      <NeedsYouDock
        approval={null}
        questions={[question, q2]}
        onApprove={vi.fn()}
        onAnswer={vi.fn()}
      />,
    );
    expect(screen.getByText('2 questions')).toBeInTheDocument();
  });
});
