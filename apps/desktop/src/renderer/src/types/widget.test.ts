import { describe, it, expect } from 'vitest';
import {
  WIDGET_BOARD_WIDTH,
  WIDGET_CELL,
  WIDGET_COLUMNS,
  WIDGET_GAP,
  WIDGET_PAD,
  WIDGET_SIZES,
  WIDGET_SPANS,
  clampWidgetSize,
  widgetKey,
} from './widget';

describe('widget size classes', () => {
  // The three classes are iPhone's own, expressed in a two-column board. If
  // these spans drift, a "medium" stops being a full-width row and every board
  // silently relayouts.
  it('span the iPhone footprints in a two-column grid', () => {
    expect(WIDGET_SPANS.small).toEqual({ cols: 1, rows: 1 });
    expect(WIDGET_SPANS.medium).toEqual({ cols: WIDGET_COLUMNS, rows: 1 });
    expect(WIDGET_SPANS.large).toEqual({ cols: WIDGET_COLUMNS, rows: 2 });
  });

  it('orders smallest first, which is what the size picker relies on', () => {
    expect(WIDGET_SIZES).toEqual(['small', 'medium', 'large']);
  });

  // The rail derives its width from these so a small widget renders square.
  // Hardcoding the rail width again is exactly the drift this guards.
  it('derive a board width that makes a small widget square', () => {
    expect(WIDGET_BOARD_WIDTH).toBe(
      WIDGET_COLUMNS * WIDGET_CELL + WIDGET_GAP * (WIDGET_COLUMNS - 1) + WIDGET_PAD * 2,
    );
    const columnWidth = (WIDGET_BOARD_WIDTH - WIDGET_PAD * 2 - WIDGET_GAP) / WIDGET_COLUMNS;
    expect(columnWidth).toBe(WIDGET_CELL);
  });
});

describe('widgetKey', () => {
  it('namespaces host widgets so a plugin cannot collide with one', () => {
    expect(widgetKey({ widget: 'git' })).toBe('host:git');
    expect(widgetKey({ plugin: 'acme.thing', widget: 'git' })).toBe('acme.thing:git');
  });
});

describe('clampWidgetSize', () => {
  it('keeps a supported size unchanged', () => {
    expect(clampWidgetSize('large', ['small', 'large'])).toBe('large');
  });

  // A placement outlives the widget it names: a plugin update can drop a class,
  // and config.yaml is hand-editable. Rendering at a footprint the widget never
  // designed for is worse than quietly using one it did.
  it('falls back to the largest supported size no bigger than requested', () => {
    expect(clampWidgetSize('large', ['small', 'medium'])).toBe('medium');
    expect(clampWidgetSize('medium', ['small'])).toBe('small');
  });

  it('falls back upward when nothing smaller is supported', () => {
    expect(clampWidgetSize('small', ['medium', 'large'])).toBe('medium');
  });

  it('tolerates an unsorted supported list', () => {
    expect(clampWidgetSize('large', ['large', 'small'])).toBe('large');
    expect(clampWidgetSize('medium', ['large', 'small'])).toBe('small');
  });

  it('degrades to small rather than throwing on an empty list', () => {
    expect(clampWidgetSize('large', [])).toBe('small');
  });
});
