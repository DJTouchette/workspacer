//! The stacked diff of design v2's Changes: a parsed multi-file diff cut into
//! one section per file, then laid out as fixed-height list slots (file
//! header, folded unchanged lines, diff lines in one column or side by
//! side), with the hunks indexed for `]c` / `[c`. Pure, so the layout and
//! the intra-line emphasis are unit-tested without a window.
use std::ops::Range;
use wks_native::diff::{Diff, Kind};

/// What happened to a file, as its diff header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileChange {
    Modified,
    Added,
    Deleted,
    Renamed,
}

/// One file of a stacked diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Section {
    /// The file's path (the new one for renames), repository-relative.
    pub path: String,
    /// The path it was renamed from.
    pub old_path: Option<String>,
    pub change: FileChange,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
    /// Its rows in [`Diff::rows`], header lines included.
    pub rows: Range<usize>,
}

/// One fixed-height slot of the stacked list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// Space between two files.
    Gap,
    /// The first slot of a file's header (the header itself is drawn over
    /// the list so it can stick to the top).
    Header(usize),
    /// The header's remaining slots.
    HeaderPad(usize),
    /// Unchanged lines git left out before a hunk; `context` is the hunk
    /// header's trailing text (usually the enclosing function).
    Fold {
        file: usize,
        hidden: u32,
        context: String,
    },
    /// One diff row, single column.
    Line { file: usize, row: usize },
    /// Old and new side by side; context rows sit on both sides.
    Pair {
        file: usize,
        old: Option<usize>,
        new: Option<usize>,
    },
    /// A file with nothing to draw as lines (binary, mode change, empty).
    Placeholder { file: usize, text: &'static str },
}

/// The slots of a stacked diff and where its files and hunks start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    pub items: Vec<Item>,
    /// The `Header` slot of each file.
    pub headers: Vec<usize>,
    /// One past each file's last slot.
    pub ends: Vec<usize>,
    /// The first slot of each hunk (its fold, or its first line).
    pub hunks: Vec<usize>,
    /// The `Line` slot with the longest text (sizes a scrolling list).
    pub widest: usize,
}

/// `diff --git a/x b/y` → `y` (unquoted paths only; others fall back to the
/// `+++` / `---` lines).
fn git_header_path(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("diff --git ")?;
    let at = rest.rfind(" b/")?;
    Some(&rest[at + 3..])
}

fn file_line_path(line: &str) -> Option<&str> {
    let path = line
        .strip_prefix("+++ ")
        .or_else(|| line.strip_prefix("--- "))?
        .trim_end();
    if path == "/dev/null" {
        return None;
    }
    Some(
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path),
    )
}

/// Cut a parsed diff into one section per file, in order.
pub(crate) fn sections(diff: &Diff) -> Vec<Section> {
    let rows = &diff.rows;
    let mut starts: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == Kind::Meta && r.text.starts_with("diff --git "))
        .map(|(ix, _)| ix)
        .collect();
    if starts.first() != Some(&0) && !rows.is_empty() {
        starts.insert(0, 0);
    }
    let mut out = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(rows.len());
        let mut section = Section {
            path: String::new(),
            old_path: None,
            change: FileChange::Modified,
            added: 0,
            removed: 0,
            binary: false,
            rows: start..end,
        };
        let (mut minus, mut plus) = (None::<String>, None::<String>);
        let mut header_path = None;
        for row in &rows[start..end] {
            match row.kind {
                Kind::Added => section.added += 1,
                Kind::Removed => section.removed += 1,
                Kind::Meta => {
                    let t = row.text.as_str();
                    if let Some(path) = git_header_path(t) {
                        header_path = Some(path.to_owned());
                    } else if t.starts_with("new file mode") {
                        section.change = FileChange::Added;
                    } else if t.starts_with("deleted file mode") {
                        section.change = FileChange::Deleted;
                    } else if let Some(from) = t.strip_prefix("rename from ") {
                        section.change = FileChange::Renamed;
                        section.old_path = Some(from.to_owned());
                    }
                }
                Kind::File => {
                    let path = file_line_path(&row.text).map(str::to_owned);
                    if row.text.starts_with("--- ") {
                        if path.is_none() {
                            section.change = FileChange::Added;
                        }
                        minus = path;
                    } else {
                        if path.is_none() {
                            section.change = FileChange::Deleted;
                        }
                        plus = path;
                    }
                }
                Kind::Note if row.text.starts_with("Binary files ") => section.binary = true,
                _ => {}
            }
        }
        section.path = plus
            .clone()
            .or(header_path)
            .or_else(|| minus.clone())
            .unwrap_or_default();
        if section.change == FileChange::Modified
            && let (Some(minus), Some(plus)) = (&minus, &plus)
            && minus != plus
        {
            section.change = FileChange::Renamed;
            section.old_path = Some(minus.clone());
        }
        out.push(section);
    }
    out
}

/// `@@ -12,3 +14,5 @@ fn main()` → (12, 14, "fn main()").
fn hunk_header(text: &str) -> Option<(u32, u32, &str)> {
    let rest = text.strip_prefix("@@ ")?;
    let (ranges, context) = rest.split_once(" @@")?;
    let mut parts = ranges.split(' ');
    let start = |s: Option<&str>, sign: char| -> Option<u32> {
        s?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((
        start(parts.next(), '-')?,
        start(parts.next(), '+')?,
        context.trim(),
    ))
}

/// Lay a diff's sections out as list slots. `collapsed(path)` folds a file
/// to its header; `header_slots` is how many slots a header covers.
pub(crate) fn layout(
    diff: &Diff,
    sections: &[Section],
    split: bool,
    collapsed: &dyn Fn(&str) -> bool,
    header_slots: usize,
) -> Layout {
    let rows = &diff.rows;
    let mut out = Layout::default();
    let mut widest = (0usize, 0usize);
    for (file, section) in sections.iter().enumerate() {
        if file > 0 {
            out.items.push(Item::Gap);
        }
        out.headers.push(out.items.len());
        out.items.push(Item::Header(file));
        for _ in 1..header_slots.max(1) {
            out.items.push(Item::HeaderPad(file));
        }
        if collapsed(&section.path) {
            out.ends.push(out.items.len());
            continue;
        }
        let body = out.items.len();
        let (mut last_old, mut last_new) = (0u32, 0u32);
        // Split view: removed rows wait for the added rows they pair with.
        let mut removed: Vec<usize> = Vec::new();
        let mut added: Vec<usize> = Vec::new();
        let flush = |out: &mut Layout, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
            let n = removed.len().max(added.len());
            for k in 0..n {
                out.items.push(Item::Pair {
                    file,
                    old: removed.get(k).copied(),
                    new: added.get(k).copied(),
                });
            }
            removed.clear();
            added.clear();
        };
        for ix in section.rows.clone() {
            let row = &rows[ix];
            match row.kind {
                Kind::Meta | Kind::File => continue,
                Kind::Note if row.text.starts_with("Binary files ") => continue,
                _ => {}
            }
            if split && !matches!(row.kind, Kind::Added | Kind::Removed) {
                flush(&mut out, &mut removed, &mut added);
            }
            match row.kind {
                Kind::Hunk => {
                    let Some((old, new, context)) = hunk_header(&row.text) else {
                        continue;
                    };
                    let hidden = if new > 0 {
                        new.saturating_sub(last_new + 1)
                    } else {
                        old.saturating_sub(last_old + 1)
                    };
                    out.hunks.push(out.items.len());
                    if hidden > 0 {
                        out.items.push(Item::Fold {
                            file,
                            hidden,
                            context: context.to_owned(),
                        });
                    }
                }
                Kind::Added | Kind::Removed if split => {
                    if row.kind == Kind::Removed {
                        // A removal after additions starts a new pairing.
                        if !added.is_empty() {
                            flush(&mut out, &mut removed, &mut added);
                        }
                        removed.push(ix);
                    } else {
                        added.push(ix);
                    }
                }
                _ => {
                    if row.text.len() > widest.1 {
                        widest = (out.items.len(), row.text.len());
                    }
                    out.items.push(if split {
                        Item::Pair {
                            file,
                            old: Some(ix),
                            new: Some(ix),
                        }
                    } else {
                        Item::Line { file, row: ix }
                    });
                }
            }
            if let Some(n) = row.old {
                last_old = n;
            }
            if let Some(n) = row.new {
                last_new = n;
            }
        }
        if split {
            flush(&mut out, &mut removed, &mut added);
        }
        if out.items.len() == body {
            out.items.push(Item::Placeholder {
                file,
                text: if section.binary {
                    "Binary file not shown"
                } else if section.change == FileChange::Renamed {
                    "Renamed without changes"
                } else {
                    "No text changes"
                },
            });
        }
        out.ends.push(out.items.len());
    }
    out.widest = widest.0;
    out
}

/// The changed span of each removed/added row that pairs with a row on the
/// other side: what differs once the shared prefix and suffix are set
/// aside, as byte ranges on char boundaries. `None` where nothing pairs,
/// where the lines share nothing, or where one is a pure insertion into the
/// other (that side gets no emphasis).
pub(crate) fn emphasis(diff: &Diff) -> Vec<Option<Range<usize>>> {
    let rows = &diff.rows;
    let mut out = vec![None; rows.len()];
    let mut ix = 0;
    while ix < rows.len() {
        if rows[ix].kind != Kind::Removed {
            ix += 1;
            continue;
        }
        let start = ix;
        while ix < rows.len() && rows[ix].kind == Kind::Removed {
            ix += 1;
        }
        let middle = ix;
        while ix < rows.len() && rows[ix].kind == Kind::Added {
            ix += 1;
        }
        for k in 0..(middle - start).min(ix - middle) {
            let (old, new) = (start + k, middle + k);
            if let Some((a, b)) = changed_spans(&rows[old].text, &rows[new].text) {
                out[old] = a;
                out[new] = b;
            }
        }
    }
    out
}

/// The differing middles of two lines. `None` when they share no prefix or
/// suffix at all (the whole line changed: its tint says so already).
#[allow(clippy::type_complexity)]
fn changed_spans(a: &str, b: &str) -> Option<(Option<Range<usize>>, Option<Range<usize>>)> {
    let prefix = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or_else(|| a.len().min(b.len()), |((i, _), _)| i);
    // Keep the prefix on a boundary of both strings.
    let prefix = (0..=prefix)
        .rev()
        .find(|&i| a.is_char_boundary(i) && b.is_char_boundary(i))
        .unwrap_or(0);
    let (ra, rb) = (&a[prefix..], &b[prefix..]);
    let suffix = ra
        .chars()
        .rev()
        .zip(rb.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum::<usize>();
    let shared = prefix + suffix;
    // Indentation alone is not a shared line.
    let indent = a.len() - a.trim_start().len();
    if shared == 0 || (suffix == 0 && prefix <= indent) {
        return None;
    }
    let span = |s: &str| {
        let r = prefix..s.len() - suffix;
        (!r.is_empty()).then_some(r)
    };
    Some((span(a), span(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wks_native::diff::parse;

    const TWO: &str = "diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
-    start();
+    restore_workspace();
+    start();
 }
@@ -40,3 +41,3 @@ fn later() {
 a
-b
+c
diff --git a/tests/session.rs b/tests/session.rs
new file mode 100644
--- /dev/null
+++ b/tests/session.rs
@@ -0,0 +1,2 @@
+#[test]
+fn works() {}
";

    #[test]
    fn sections_follow_the_git_headers() {
        let diff = parse(TWO);
        let s = sections(&diff);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].path, "src/main.rs");
        assert_eq!(s[0].change, FileChange::Modified);
        assert_eq!((s[0].added, s[0].removed), (3, 2));
        assert_eq!(s[1].path, "tests/session.rs");
        assert_eq!(s[1].change, FileChange::Added);
        assert_eq!((s[1].added, s[1].removed), (2, 0));
    }

    #[test]
    fn renames_and_deletions_are_named() {
        let diff = parse(
            "diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-x
",
        );
        let s = sections(&diff);
        assert_eq!(s[0].change, FileChange::Renamed);
        assert_eq!(s[0].path, "new.rs");
        assert_eq!(s[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(s[1].change, FileChange::Deleted);
        assert_eq!(s[1].path, "gone.rs");
        let l = layout(&diff, &s, false, &|_| false, 2);
        assert!(matches!(
            l.items[2],
            Item::Placeholder {
                text: "Renamed without changes",
                ..
            }
        ));
    }

    #[test]
    fn git_headers_are_hidden_and_gaps_fold() {
        let diff = parse(TWO);
        let s = sections(&diff);
        let l = layout(&diff, &s, false, &|_| false, 2);
        // No Meta / File / Hunk row is drawn as a line.
        for item in &l.items {
            if let Item::Line { row, .. } = item {
                assert!(matches!(
                    diff.rows[*row].kind,
                    Kind::Context | Kind::Added | Kind::Removed | Kind::Note
                ));
            }
        }
        assert_eq!(l.headers, [0, 13]);
        assert_eq!(l.items[0], Item::Header(0));
        assert_eq!(l.items[1], Item::HeaderPad(0));
        // The first hunk starts at line 1: nothing folded before it.
        assert_eq!(l.hunks[0], 2);
        assert!(matches!(l.items[2], Item::Line { .. }));
        // The second skips lines 5..=40 of the new file.
        assert_eq!(
            l.items[l.hunks[1]],
            Item::Fold {
                file: 0,
                hidden: 36,
                context: "fn later() {".into(),
            }
        );
        assert_eq!(l.items[12], Item::Gap);
        assert_eq!(l.ends, [12, 17]);
        assert_eq!(l.hunks.len(), 3);
    }

    #[test]
    fn split_pairs_removals_with_additions() {
        let diff = parse(TWO);
        let s = sections(&diff);
        let l = layout(&diff, &s[..1], true, &|_| false, 2);
        let pairs: Vec<_> = l
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Pair { old, new, .. } => Some((
                    old.map(|r| diff.rows[r].text.trim().to_owned()),
                    new.map(|r| diff.rows[r].text.trim().to_owned()),
                )),
                _ => None,
            })
            .collect();
        let s = |t: &str| Some(t.to_owned());
        assert_eq!(
            pairs,
            [
                (s("fn main() {"), s("fn main() {")),
                (s("start();"), s("restore_workspace();")),
                (None, s("start();")),
                (s("}"), s("}")),
                (s("a"), s("a")),
                (s("b"), s("c")),
            ]
        );
    }

    #[test]
    fn collapsed_files_keep_only_their_header() {
        let diff = parse(TWO);
        let s = sections(&diff);
        let l = layout(&diff, &s, false, &|p| p == "src/main.rs", 2);
        assert_eq!(
            &l.items[..4],
            &[
                Item::Header(0),
                Item::HeaderPad(0),
                Item::Gap,
                Item::Header(1)
            ]
        );
        assert_eq!(l.hunks.len(), 1);
    }

    #[test]
    fn emphasis_marks_the_changed_words() {
        let diff = parse(
            "diff --git a/a b/a
--- a/a
+++ b/a
@@ -1,2 +1,2 @@
-    let x = start(1);
-totally different
+    let x = start(2);
+nothing alike here
",
        );
        let e = emphasis(&diff);
        let row = |i: usize| diff.rows[i].text.as_str();
        let (old, new) = (4, 6);
        assert_eq!(&row(old)[e[old].clone().unwrap()], "1");
        assert_eq!(&row(new)[e[new].clone().unwrap()], "2");
        // No shared prefix or suffix: no emphasis, the tint says enough.
        assert_eq!(e[5], None);
        assert_eq!(e[7], None);
        // A pure insertion emphasises only the added side.
        assert_eq!(
            changed_spans("foo()", "foo(x)"),
            Some((None, Some(4..5)))
        );
        // Multi-byte text stays on char boundaries.
        assert_eq!(changed_spans("é1é", "é2é"), Some((Some(2..3), Some(2..3))));
    }
}
