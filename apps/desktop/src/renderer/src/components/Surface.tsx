import React from 'react';

/**
 * `Surface` — the one container primitive. Every card-shaped box should be a
 * `<Surface>` rather than a hand-rolled bordered div.
 *
 * ## The rule: a surface separates itself with a border OR a fill, never both.
 *
 * WHY: the density problem in this UI was never the tokens, it was nesting.
 * Cards nested three and four deep (agent card → action zone → picker → option
 * row) and *every* level drew a 1px border *and* a background fill. That put
 * ~20 outlined rectangles on screen at rest, and the eye reads each outline as
 * a separate object, so a screen with four agents on it looked like twenty
 * things. Halving the border count is not enough — the fix has to make the
 * onion impossible to rebuild.
 *
 * So the two elevation levels own *different* separation channels:
 *
 *   raised → fill + lighting, NO border.  The outermost box of a card, sitting
 *            on --wks-bg-base. Depth comes from the fill plus a light-from-
 *            above bevel (see `ensureSurfaceStyles`), never from an outline.
 *   flat   → 1px border, NO fill.  Anything *inside* a raised surface:
 *            sections, list items, option rows. It reads as an outline drawn on
 *            the parent's own fill, so fills never stack on fills.
 *
 * Because the channels are disjoint, `raised > flat` is the only composition
 * that reads, and it bottoms out after one step. **Surfaces nest at most two
 * deep.** If you find yourself wanting a third level, you want a divider rule
 * (`borderTop`) or plain padding inside the surface you already have — a rule
 * inside a surface is not a nested surface.
 *
 * `tone` carries status (needs-approval, error, running) without introducing a
 * second outline: on `raised` it is a 2px leading rail, on `flat` it simply
 * tints the single border the surface already has.
 *
 * If you are about to add `border` *and* `background` to a Surface's `style`,
 * stop — that is the exact regression this component exists to prevent.
 */

export type SurfaceElevation = 'flat' | 'raised';
export type SurfacePad = 'none' | 'sm' | 'md' | 'lg';
export type SurfaceRadius = 'sm' | 'md' | 'lg';

/** Padding presets, all on the 4 / 6 / 8 / 10 / 12 / 16 / 20 rhythm. */
const PAD: Record<SurfacePad, string | undefined> = {
  none: undefined,
  sm: '6px 8px',
  md: '8px 12px',
  lg: '12px 16px',
};

const STYLE_ID = 'wks-surface-styles';

/**
 * Hover/edge declarations live in a stylesheet, never inline: the element's
 * inline style carries only the `--wks-surface-*` custom properties, so a
 * `:hover` rule can win on specificity alone — no `!important`, and a caller's
 * own `onMouseEnter` is left untouched. Runtime `<style>` injection is the
 * established pattern here (see `ensureKeyframes` in `claude-shared.tsx`).
 *
 * ## Why `raised` is lit rather than outlined
 *
 * The fill step cannot be trusted to carry a raised surface on its own. Several
 * themes put `--wks-bg-surface` within 2–3 RGB units of `--wks-bg-base`
 * (everforest `#2d353b` vs `#2f373d`, kanagawa `#1f1f28` vs `#21212b`) and
 * one-dark puts the surface *below* the base. A drop shadow did not cover for
 * it either: offset downward with no spread, it lit the bottom edge and left
 * the top and sides with nothing. Since a border is not available to `raised`,
 * the treatment models a light source instead of a rectangle:
 *
 *   1. `inset 0 1px 0` white lip — the highlight where light from above catches
 *      the card's top face. This is the layer that survives a 2-unit fill step,
 *      and being one edge it never closes into an outline the eye counts as a
 *      separate object.
 *   2. `0 0 2px` ambient halo — keeps the left/right edges of a tall card from
 *      dissolving into the base.
 *   3. `0 1px 2px` hairline drop, as before.
 *
 * White in a shadow is explicitly allowed (DESIGN_LANGUAGE.md §2) and the alpha
 * comes from `color-mix`, not a hand-rolled rgba. In light themes the lip is a
 * no-op (white on a white card) — there the fill step is already 11 units and
 * the ambient halo does the separating.
 */
function ensureSurfaceStyles(): void {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const el = document.createElement('style');
  el.id = STYLE_ID;
  el.textContent = `
    .wks-surface {
      box-sizing: border-box;
      transition: background-color 0.12s, border-color 0.12s, box-shadow 0.12s;
    }
    .wks-surface--raised {
      background: var(--wks-surface-fill);
      border: none;
      /* top lip, ambient halo, hairline drop — see ensureSurfaceStyles docs */
      box-shadow:
        inset 0 1px 0 color-mix(in srgb, #fff 7%, transparent),
        0 0 2px var(--wks-shadow),
        0 1px 2px var(--wks-shadow);
    }
    .wks-surface--flat {
      background: transparent;
      border: 1px solid var(--wks-surface-edge);
      box-shadow: none;
    }
    .wks-surface--interactive { cursor: pointer; }
    .wks-surface--interactive.wks-surface--raised:hover {
      background: var(--wks-surface-fill-hover);
    }
    .wks-surface--interactive.wks-surface--flat:hover {
      border-color: var(--wks-surface-edge-hover);
    }
  `;
  document.head.appendChild(el);
}
ensureSurfaceStyles();

export interface SurfaceProps extends React.HTMLAttributes<HTMLDivElement> {
  /**
   * `'raised'` = fill, no border — the outermost box of a card.
   * `'flat'` = border, no fill — anything nested inside one. Default `'flat'`.
   */
  elevation?: SurfaceElevation;
  /**
   * Padding preset. Default `'none'` — cards that pad their own header/body
   * sections should keep doing that.
   */
  pad?: SurfacePad;
  /** Corner radius token. Default `'md'`. */
  radius?: SurfaceRadius;
  /**
   * Status colour, as a bare `var(--wks-*)` expression (or an allowed constant
   * palette value). Tints the surface's one separation channel — a leading rail
   * when `raised`, the border itself when `flat`. It never adds a second one.
   */
  tone?: string;
  /**
   * Selection state. On `raised` it shifts the fill to `--wks-bg-selected`, so
   * a `tone` rail keeps showing status alongside it. On `flat` there is only
   * the border to work with, so selection takes it and overrides `tone`.
   */
  selected?: boolean;
  /**
   * Pointer cursor plus a hover step on the surface's own channel. Note: a
   * caller that also sets `style={{ background }}` (raised) or
   * `style={{ borderColor }}` (flat) will out-specify the hover rule.
   */
  interactive?: boolean;
}

/**
 * The shared card container. See the module comment for the border-or-fill
 * rule and why it exists.
 */
export const Surface = React.forwardRef<HTMLDivElement, SurfaceProps>(function Surface(
  {
    elevation = 'flat',
    pad = 'none',
    radius = 'md',
    tone,
    selected,
    interactive,
    className,
    style,
    children,
    ...rest
  },
  ref,
) {
  const raised = elevation === 'raised';

  // raised: fill carries selection, the rail carries status.
  // flat: one border, so selection wins the channel outright.
  const rail = raised ? (tone ?? (selected ? 'var(--wks-border-active)' : undefined)) : undefined;
  const edge = selected ? 'var(--wks-border-active)' : tone;

  const vars = {
    '--wks-surface-fill': selected ? 'var(--wks-bg-selected)' : 'var(--wks-bg-surface)',
    '--wks-surface-fill-hover': selected ? 'var(--wks-bg-selected)' : 'var(--wks-bg-hover)',
    '--wks-surface-edge': edge
      ? `color-mix(in srgb, ${edge} 45%, var(--wks-border-subtle))`
      : 'var(--wks-border-subtle)',
    '--wks-surface-edge-hover': edge
      ? `color-mix(in srgb, ${edge} 70%, var(--wks-border-subtle))`
      : 'var(--wks-border)',
  } as React.CSSProperties;

  return (
    <div
      ref={ref}
      {...rest}
      className={[
        'wks-surface',
        raised ? 'wks-surface--raised' : 'wks-surface--flat',
        interactive ? 'wks-surface--interactive' : '',
        className ?? '',
      ]
        .filter(Boolean)
        .join(' ')}
      style={{
        ...vars,
        borderRadius: `var(--wks-radius-${radius})`,
        padding: PAD[pad],
        // Only claimed when a rail is drawn, so callers keep control otherwise.
        ...(rail ? { position: 'relative', overflow: 'hidden' } : null),
        ...style,
      }}
    >
      {rail && (
        <span
          aria-hidden
          style={{
            position: 'absolute',
            insetInlineStart: 0,
            top: 0,
            bottom: 0,
            width: 2,
            background: rail,
            pointerEvents: 'none',
          }}
        />
      )}
      {children}
    </div>
  );
});

export default Surface;
