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
  options: [
    { label: 'npm', description: 'the default' },
    { label: 'pnpm' },
  ],
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
    const input = screen.getByPlaceholderText('Or type a custom answer...');
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
    const input = screen.getByPlaceholderText('Or type a custom answer...');
    fireEvent.change(input, { target: { value: '  bun  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onAnswer).toHaveBeenCalledWith({ text: 'bun' });
  });

  it('a multi-question set omits the custom free-text field', () => {
    const q2: PendingQuestion = { question: 'Second?', options: [{ label: 'a' }] };
    render(<QuestionPicker questions={[question, q2]} onAnswer={vi.fn()} />);
    expect(screen.queryByPlaceholderText('Or type a custom answer...')).not.toBeInTheDocument();
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
    render(<NeedsYouDock approval={approval} questions={null} onApprove={onApprove} onAnswer={vi.fn()} />);
    expect(screen.getByText('Approval needed')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Allow'));
    expect(onApprove).toHaveBeenCalledWith('yes');
  });

  it('a pending question wins over a racing approval (picker shown, approval hidden)', () => {
    render(
      <NeedsYouDock approval={approval} questions={[question]} onApprove={vi.fn()} onAnswer={vi.fn()} />,
    );
    expect(screen.getByText('Claude is asking you')).toBeInTheDocument();
    expect(screen.getByText('Which package manager?')).toBeInTheDocument();
    // The stale approval card must not render underneath the picker.
    expect(screen.queryByText('Permission Required: Bash')).not.toBeInTheDocument();
  });

  it('labels the count when several questions are docked together', () => {
    const q2: PendingQuestion = { question: 'Second?', options: [{ label: 'a' }] };
    render(
      <NeedsYouDock approval={null} questions={[question, q2]} onApprove={vi.fn()} onAnswer={vi.fn()} />,
    );
    expect(screen.getByText('2 questions')).toBeInTheDocument();
  });
});
