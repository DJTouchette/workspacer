import { describe, it, expect } from 'vitest';
import { parseUnifiedDiff } from './parseDiff';

const SIMPLE = `diff --git a/foo.ts b/foo.ts
index 1234567..89abcde 100644
--- a/foo.ts
+++ b/foo.ts
@@ -1,4 +1,4 @@
 const a = 1;
-const b = 2;
+const b = 3;
 const c = 4;
`;

describe('parseUnifiedDiff', () => {
  it('parses hunks with correct line numbers', () => {
    const d = parseUnifiedDiff(SIMPLE);
    expect(d.hunks).toHaveLength(1);
    expect(d.additions).toBe(1);
    expect(d.deletions).toBe(1);
    const lines = d.hunks[0].lines;
    expect(lines.map((l) => l.kind)).toEqual(['context', 'del', 'add', 'context']);
    expect(lines[0]).toMatchObject({ oldNo: 1, newNo: 1 });
    expect(lines[1]).toMatchObject({ oldNo: 2, newNo: null, text: 'const b = 2;' });
    expect(lines[2]).toMatchObject({ oldNo: null, newNo: 2, text: 'const b = 3;' });
    expect(lines[3]).toMatchObject({ oldNo: 3, newNo: 3 });
  });

  it('marks intraline emphasis on paired changed lines', () => {
    const d = parseUnifiedDiff(SIMPLE);
    const del = d.hunks[0].lines[1];
    const add = d.hunks[0].lines[2];
    // "const b = " is shared prefix, ";" shared suffix → only the digit emphasized
    expect(del.emph).toEqual([10, 11]);
    expect(add.emph).toEqual([10, 11]);
  });

  it('skips emphasis when lines share nothing', () => {
    const text = `--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-aaa
+zzz
`;
    const d = parseUnifiedDiff(text);
    expect(d.hunks[0].lines[0].emph).toBeUndefined();
    expect(d.hunks[0].lines[1].emph).toBeUndefined();
  });

  it('pairs runs of deletions with runs of additions first-to-first', () => {
    const text = `--- a/x
+++ b/x
@@ -1,2 +1,2 @@
-foo(1)
-bar(2)
+foo(9)
+bar(8)
`;
    const d = parseUnifiedDiff(text);
    const [del1, del2, add1, add2] = d.hunks[0].lines;
    expect(del1.emph).toEqual([4, 5]);
    expect(add1.emph).toEqual([4, 5]);
    expect(del2.emph).toEqual([4, 5]);
    expect(add2.emph).toEqual([4, 5]);
  });

  it('detects binary diffs', () => {
    const d = parseUnifiedDiff('diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n');
    expect(d.binary).toBe(true);
    expect(d.hunks).toHaveLength(0);
  });

  it('attaches no-newline markers to the previous line', () => {
    const text = `--- a/x
+++ b/x
@@ -1,1 +1,1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
`;
    const d = parseUnifiedDiff(text);
    expect(d.hunks[0].lines[0].noNewline).toBe(true);
    expect(d.hunks[0].lines[1].noNewline).toBe(true);
  });

  it('handles hunk headers without counts (single-line files)', () => {
    const text = `--- /dev/null
+++ b/x
@@ -0,0 +1 @@
+hello
`;
    const d = parseUnifiedDiff(text);
    expect(d.hunks[0].lines[0]).toMatchObject({ kind: 'add', newNo: 1 });
  });

  it('tracks the longest line for scroll sizing', () => {
    const long = 'x'.repeat(300);
    const d = parseUnifiedDiff(`--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-${long}\n+y\n`);
    expect(d.maxLineLength).toBe(300);
  });

  it('handles multi-file diffs by closing the hunk at the next file header', () => {
    const text = `diff --git a/a.ts b/a.ts
--- a/a.ts
+++ b/a.ts
@@ -1,1 +1,1 @@
-one
+two
diff --git a/b.ts b/b.ts
--- a/b.ts
+++ b/b.ts
@@ -5,1 +5,1 @@
-three
+four
`;
    const d = parseUnifiedDiff(text);
    expect(d.hunks).toHaveLength(2);
    expect(d.hunks[1].lines[0].oldNo).toBe(5);
    expect(d.additions).toBe(2);
  });
});
