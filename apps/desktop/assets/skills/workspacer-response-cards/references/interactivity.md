<!-- workspacer:generated response-cards skill -->
# Card interactivity

You never write JavaScript. Workspacer runs one fixed script inside the card
that implements the behaviours below; a `<script>` of your own is stripped and
could not run even if it were not.

## Expanding — no attributes needed

```html
<details><summary>Why this matters</summary><p>The long version.</p></details>
```

Use this for anything secondary. A card that opens showing eight rows and hides
the reasoning behind each one reads far better than a wall.

## Filtering

```html
<input type="search" data-wks-filter="findings" placeholder="Filter findings…">
<table id="findings">
  <tbody>
    <tr data-wks-filter-item><td>auth.ts</td><td>Token never expires</td></tr>
    <tr data-wks-filter-item><td>db.ts</td><td>Missing index</td></tr>
  </tbody>
</table>
```

- `data-wks-filter="<id>"` on the input names the element to filter inside.
- `data-wks-filter-item` marks each filterable element (usually a `<tr>`).
- Matching is a case-insensitive substring of the element's own text.
- An element with `data-wks-filter-count` inside the scope receives the number
  of visible items.

## Sorting

```html
<table>
  <thead><tr>
    <th data-wks-sort="text">File</th>
    <th data-wks-sort="number">Lines</th>
  </tr></thead>
  <tbody><tr><td>auth.ts</td><td>412</td></tr></tbody>
</table>
```

Clicking the header sorts the first `<tbody>`; clicking again reverses.
`"number"` parses the leading number out of the cell, `"text"` compares
case-insensitively.

## Size

The card sizes itself to its content up to about 560px tall and then scrolls
inside itself. Keep the first screen worth reading: put the summary at the top
and the detail in `<details>`.
