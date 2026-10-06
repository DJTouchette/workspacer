# Workspacer's GPUI Component patch

This is the crates.io `gpui-component` 0.5.1 source, with its Apache 2.0 license
and locales. The native Cargo manifest pins this copy through a crates.io patch.

Two small TextViewStyle additions keep the native transcript in the component's
existing Markdown renderer, preserving selection across paragraphs, lists,
tables and code, and its asynchronous parse cache:

- `on_link_click`: an optional application callback instead of the default
  system URL opener. Only real link clicks invoke it; selection retains the
  upstream behavior. Callback identity is intentionally excluded from style
  comparisons to avoid reparsing on every render. Native view IDs include the
  session, and the callback checks current session ownership and cwd on click.
  When `on_link_click` is set, Markdown/HTML images are not loaded by the
  component at all: `node.rs` renders an inline image label (alt text or file
  name) whose click goes to the same callback with the image's link, or its
  source when unlinked. Upstream `img(url)` would read a path from the client's
  own disk (wrong machine for a remote agent) or fetch any URL unasked, and its
  click handler called `cx.open_url` directly with any scheme.
- `unordered_list_marker`: an optional visual bullet override. The default
  preserves upstream behavior; the native transcript chooses a round bullet.

- `prose`: optional `ProseColors` for desktop-chat parity. When set, bold and
  italic use a bright color, inline code uses an accent color/background and
  the theme mono font (per-run family in `inline.rs`), bullets/ordered markers,
  headings (h1/h2 underlined) and rules use app colors, and fenced code gets a
  bordered panel with a language header. Tables render through
  `render_prose_table` (rounded frame, tinted bold header, hairline rules,
  zebra stripes, smaller cells sized by content share but never narrower than
  their longest word, shaped in the cell's font (bold header, mono code with
  its thin-space margins) and capped at 12em (about twenty code characters) so
  URLs and long paths still wrap; cells are blocks
  with text alignment and clipping, not flex rows, so text wraps inside its
  column) and blockquotes get a slim marker-colored rail with muted italic
  text. `None` keeps upstream rendering.

  A table whose word minimums fit fills the width with no scroll affordance.
  When they cannot fit (many columns, a narrow chat or preview, a large
  interface size) `ProseTableFrame` keeps them and scrolls the rows sideways
  inside the fixed frame (desktop parity: `overflow-x: auto`): trackpad or
  Shift+wheel, an always-visible draggable scrollbar strip under the rows,
  and Left/Right once the frame is focused (click or Tab; it is a tab stop
  only while it scrolls). A plain vertical wheel keeps scrolling the page
  (`restrict_scroll_to_axis`). Each table keeps its scroll handle in keyed
  element state (first cell's source offset). Whether it overflows comes from
  the previous layout; a mismatch triggers one deferred redraw.

  In prose mode inline code is drawn by `Inline` as rounded (4px), padded quads
  per visual line instead of square text-run backgrounds, and the Markdown
  parser surrounds each code span with thin spaces (U+2009) for margin; those
  are stripped from copied selections (`format/markdown.rs`, `text_view.rs`).
  `inline_code_family` (optional, prose mode) replaces the theme mono font for
  inline code spans and for their table-width measurement. GPUI shapes a line
  at one size, so Workspacer passes a JetBrains Mono twin whose glyphs are
  drawn at 90% to match the desktop's 0.9em inline code. `None` keeps the
  theme mono font. It takes part in style equality, so changing it reparses.

  GPUI's `TextLayout::position_for_index` ignores text alignment while line
  painting applies it, so in right/center-aligned text (table cells) the
  quads are shifted by the same per-line alignment offset.

- `TextView::handle()` returns a `TextViewHandle` over the view's state:
  top-level heading text with block indices (`Node::heading` in `node.rs`),
  top-level block kinds (`Node::kind_name`, used by native UI tests), and
  scrolling of the `scrollable` virtual list. The native file viewer uses
  it for keyboard scrolling and same-document `#heading` links in rendered
  Markdown files. It reads parsed state only; parsing is unchanged.

Two upstream bug fixes in the same files:

- A non-list root passed `is_last = true` to every block, so chat Markdown had
  no paragraph gaps; only the final block is now last (`node.rs`).
- Style updates now re-highlight with the new style's `highlight_theme`
  instead of the theme captured at creation (`text_view.rs`), so switching
  appearance recolors code already on screen.

One performance patch outside the text renderer:

- `highlighter/highlighter.rs` shares each language's compiled tree-sitter
  queries (combined injections/locals/highlights plus its injection
  languages' highlights) across every `SyntaxHighlighter`, keyed by the
  requested language name; `LanguageRegistry::register` clears the cache.
  Upstream recompiled them for every fenced code block on every first parse,
  which was most of the native chat's cost of switching conversations (about
  86ms of UI-thread work per switch in a release build, measured with
  `make bench-native-switch`).

Text changes are confined to `src/text/{style,inline,node,text_view}.rs` and `src/text/format/markdown.rs`. Reapply them explicitly
when upgrading GPUI Component. Do not modify the user's Cargo registry cache.

Native interaction polish also patches:

- `src/button/button.rs`: hovered buttons use their variant/theme foreground,
  replacing an accidental hardcoded red foreground.
- `src/styled.rs`: focus rings use 80 percent of the theme ring color so keyboard
  focus remains visible. The native client supplies its palette accent as ring.
- `src/input/search.rs`: `InputState::open_search` (the `Search` action's
  handler, made callable) and `InputState::search_query` (the open panel's
  query). The native file viewer's Ctrl+F switches a rendered Markdown
  document to its source and opens search in one key press; dispatching the
  `Search` action could not, because the source input is not in the
  rendered dispatch tree until the next frame. Search behavior is unchanged.

- Selection under the glyphs (`text/inline.rs`): upstream `Inline::paint`
  painted the text and then the selection quads on top, so any opaque
  `theme.selection` (native once used its row-highlight color) hid the
  selected text entirely — selected code blocks became blank rectangles.
  Selection is now painted after inline-code backgrounds and before the text,
  matching the order `input/element.rs` already uses. Native also keeps the
  selection color translucent, per palette.

- Selection in reading order (`text/inline.rs`, `text/text_view.rs`):
  upstream `Inline::layout_selections` selected every character inside the
  rectangle spanned by the drag's start and the pointer
  (`selection_bounds`/`point_in_text_selection`). A rectangle has no
  direction, so a drag up and to the right (or down and to the left) took
  in the upper line from the leftmost of the two x positions, text the
  pointer never reached, and copied it. `TextViewState::selection_points`
  now hands each `Inline` both points, and `selection_carets` turns each
  into a caret (after every character before the point in reading order:
  lines above, then the point's own line up to the character middle). The
  selection is the span between the carets. This works across paragraphs,
  list items and table cells of one TextView: a point above an `Inline`
  is caret 0 and a point below is its end.
  `visual_box` also corrects GPUI's soft-wrap affinity: `TextLayout::
  position_for_index` reports a wrap index at the end of the line it ends,
  so the first character of every wrapped line was hit-tested on the line
  above and dragging back to a wrapped line's start missed it. A selection
  that starts at a wrap paints from the next line's start for the same
  reason. Regression tests: `text::inline::tests` here and the native
  `chat_selection_follows_the_drag_in_reading_order`.

Reapply these interaction changes explicitly when upgrading the component.
