<!-- workspacer:generated response-cards skill -->
# Three worked cards

Copy the shape. Each one is preceded, in a real reply, by a sentence or two of
ordinary prose — the card is the structured half of an answer, not the whole of
it.

## 1. Review findings

```wks-html-card
{
  "v": 1,
  "title": "Review: 3 findings in the auth change",
  "bodyHtml": "<table><thead><tr><th data-wks-sort=\"text\">Severity</th><th data-wks-sort=\"text\">File</th><th>Finding</th></tr></thead><tbody><tr><td>High</td><td><code>auth/token.ts</code></td><td>Refresh tokens are issued without an expiry, so a leaked one is valid forever.<details><summary>Why</summary><p><code>signRefresh()</code> omits <code>expiresIn</code>, and the verifier only checks the signature.</p></details></td></tr><tr><td>Medium</td><td><code>auth/session.ts</code></td><td>The session cookie is set without <code>SameSite</code>.</td></tr><tr><td>Low</td><td><code>auth/index.ts</code></td><td>Two exports are unused since the rewrite.</td></tr></tbody></table>",
  "fallback": "Three findings. High: auth/token.ts issues refresh tokens with no expiry (signRefresh omits expiresIn and the verifier only checks the signature), so a leaked token is valid forever. Medium: auth/session.ts sets the session cookie without SameSite. Low: auth/index.ts has two exports unused since the rewrite.",
  "actions": [
    { "kind": "view_diff", "label": "Diff auth/token.ts", "path": "auth/token.ts" }
  ]
}
```

## 2. Filterable comparison

```wks-html-card
{
  "v": 1,
  "title": "Queue libraries compared",
  "bodyHtml": "<label>Filter queues <input type=\"search\" data-wks-filter=\"queues\" placeholder=\"Filter…\"></label><table id=\"queues\"><caption><span data-wks-filter-count>3</span> shown</caption><thead><tr><th data-wks-sort=\"text\">Library</th><th data-wks-sort=\"number\">Weekly downloads</th><th>Backend</th><th>Notes</th></tr></thead><tbody><tr data-wks-filter-item><td>BullMQ</td><td>1200000</td><td>Redis</td><td>Best docs; needs Redis 6+.</td></tr><tr data-wks-filter-item><td>pg-boss</td><td>90000</td><td>Postgres</td><td>No new infrastructure.</td></tr><tr data-wks-filter-item><td>Graphile Worker</td><td>60000</td><td>Postgres</td><td>Lowest latency of the two Postgres options.</td></tr></tbody></table>",
  "fallback": "BullMQ (1.2M weekly, Redis) has the best docs but needs Redis 6+. pg-boss (90k weekly, Postgres) adds no new infrastructure. Graphile Worker (60k weekly, Postgres) is the lowest-latency Postgres option. If you already run Redis, BullMQ; otherwise Graphile Worker."
}
```

## 3. Checklist

```wks-html-card
{
  "v": 1,
  "title": "Node 24 migration — 5 steps",
  "bodyHtml": "<ul><li><label><input type=\"checkbox\"> Bump <code>engines.node</code> to <code>>=24</code></label></li><li><label><input type=\"checkbox\"> Rebuild native modules (<code>better-sqlite3</code>)</label></li><li><label><input type=\"checkbox\"> Replace the two <code>util.isArray</code> calls</label></li><li><label><input type=\"checkbox\"> Re-run the e2e suite on the CI image</label></li><li><label><input type=\"checkbox\"> Update the release workflow's setup-node</label></li></ul><details><summary>Why the native rebuild is first</summary><p>Native modules may require rebuilding for the target Node ABI.</p></details>",
  "fallback": "Five steps: 1) bump engines.node to >=24; 2) rebuild native modules (better-sqlite3) — do this first, native modules may require rebuilding for the target Node ABI; 3) replace the two util.isArray calls; 4) re-run the e2e suite on the CI image; 5) update the release workflow's setup-node.",
  "actions": [
    { "kind": "fill_composer", "label": "Draft step 1", "text": "Bump engines.node to >=24 and rebuild better-sqlite3, then run the unit suite." }
  ]
}
```
