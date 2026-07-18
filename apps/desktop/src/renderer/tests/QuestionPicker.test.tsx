import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { QuestionPicker } from '../src/components/claude/QuestionPicker';
import type { PendingQuestion } from '../src/types/claudeSession';

/**
 * Regression: a multi-question set must emit `answerKinds` parallel to
 * `answers`, so a free-text answer that happens to be a number (typing "3")
 * is tagged `text` and never gets numerically remapped to option #3 by the
 * daemon. See answered_input in services/claudemon (Rust).
 */
describe('<QuestionPicker> multi-question numeric free-text', () => {
  const questions: PendingQuestion[] = [
    {
      question: 'Lucky number?',
      options: [{ label: 'One' }, { label: 'Two' }, { label: 'Three' }],
    },
    {
      question: 'Pick a color',
      options: [{ label: 'Red' }, { label: 'Blue' }],
    },
  ];

  it('tags a numeric free-text answer as text and an option pick as option', () => {
    const onAnswer = vi.fn();
    render(<QuestionPicker questions={questions} onAnswer={onAnswer} />);

    // Q1: type the literal number "3" as free text, then advance.
    const input = screen.getByPlaceholderText('Or type your own answer…') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '3' } });
    fireEvent.click(screen.getByText('Next'));

    // Q2: pick option #2 ("Blue"), which finishes the set.
    fireEvent.click(screen.getByText('Blue'));

    expect(onAnswer).toHaveBeenCalledTimes(1);
    const payload = onAnswer.mock.calls[0][0];
    expect(payload.answers).toEqual(['3', '2']);
    expect(payload.answerKinds).toEqual(['text', 'option']);
  });
});
