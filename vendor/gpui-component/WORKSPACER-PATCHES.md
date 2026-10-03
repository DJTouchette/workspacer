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
  zebra stripes, smaller cells sized by content share; cells are blocks with
  text alignment and clipping, not flex rows, so text wraps inside its column) and blockquotes
  get a slim marker-colored rail with muted italic text. `None` keeps upstream
  rendering.

  In prose mode inline code is drawn by `Inline` as rounded (4px), padded quads
  per visual line instead of square text-run backgrounds, and the Markdown
  parser surrounds each code span with thin spaces (U+2009) for margin; those
  are stripped from copied selections (`format/markdown.rs`, `text_view.rs`).

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

Text changes are confined to `src/text/{style,inline,node,text_view}.rs` and `src/text/format/markdown.rs`. Reapply them explicitly
when upgrading GPUI Component. Do not modify the user's Cargo registry cache.

Native interaction polish also patches:

- `src/button/button.rs`: hovered buttons use their variant/theme foreground,
  replacing an accidental hardcoded red foreground.
- `src/styled.rs`: focus rings use 80 percent of the theme ring color so keyboard
  focus remains visible. The native client supplies its palette accent as ring.

Reapply these interaction changes explicitly when upgrading the component.
