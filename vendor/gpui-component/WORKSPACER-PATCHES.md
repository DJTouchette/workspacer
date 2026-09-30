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
- `unordered_list_marker`: an optional visual bullet override. The default
  preserves upstream behavior; the native transcript chooses a round bullet.

Changes are confined to `src/text/{style,inline,node}.rs`. Reapply them explicitly
when upgrading GPUI Component. Do not modify the user's Cargo registry cache.
