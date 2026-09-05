/**
 * The heading for one section of the Inspector's Usage tab.
 *
 * The tab answers two different questions in a fixed order: what allowance the
 * shared provider account has left, then what THIS session spent. Both halves
 * are percentages and token counts, so the only thing separating them is the
 * label, and a label that is styled differently on each half reads as two
 * unrelated widgets rather than one ordered answer. One component, used twice.
 */
import React from 'react';
import { claudeColors as colors } from './claude-shared';

export const UsageSectionHeading: React.FC<{
  title: string;
  /** The sentence under the title. It carries the SCOPE of the numbers below,
   *  which is the whole reason the two sections are labelled at all. */
  children?: React.ReactNode;
  /** Trailing control on the title row, right aligned. */
  action?: React.ReactNode;
}> = ({ title, children, action }) => (
  <>
    <div
      style={{
        display: 'flex',
        alignItems: 'baseline',
        justifyContent: 'space-between',
        gap: 8,
        marginBottom: 2,
      }}
    >
      <div style={{ fontSize: '0.72rem', fontWeight: 650, color: colors.textBright }}>{title}</div>
      {action}
    </div>
    {children !== undefined && (
      <div
        style={{
          fontSize: '0.66rem',
          color: colors.muted,
          marginBottom: 8,
          lineHeight: 1.45,
        }}
      >
        {children}
      </div>
    )}
  </>
);
