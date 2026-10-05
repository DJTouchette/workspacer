//! Unified diffs (`git diff` output) as numbered rows for the review view:
//! each line keeps its old and new line numbers, as a Git client shows them.

/// Rows shown before the view asks the user to open the file instead.
pub const MAX_ROWS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `diff --git`, `index`, mode and rename lines.
    Meta,
    /// `--- a/…` / `+++ b/…`.
    File,
    /// `@@ -a,b +c,d @@ context`.
    Hunk,
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`, `Binary files … differ`.
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    /// The line without its `+`/`-`/` ` marker.
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub rows: Vec<Row>,
    pub added: usize,
    pub removed: usize,
    /// Rows beyond [`MAX_ROWS`] were not kept.
    pub truncated: bool,
    pub binary: bool,
}

/// `@@ -12,3 +14,5 @@` → (12, 14).
fn hunk_start(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let mut parts = ranges.split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let first = |s: &str| s.split(',').next()?.parse::<u32>().ok();
    Some((first(old)?, first(new)?))
}

pub fn parse(text: &str) -> Diff {
    let mut diff = Diff::default();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in text.lines() {
        if diff.rows.len() >= MAX_ROWS {
            diff.truncated = true;
            break;
        }
        let row = |kind, old, new, text: &str| Row {
            kind,
            old,
            new,
            text: text.to_owned(),
        };
        if let Some((o, n)) = hunk_start(line) {
            (old, new, in_hunk) = (o, n, true);
            diff.rows.push(row(Kind::Hunk, None, None, line));
            continue;
        }
        if line.starts_with("diff --git ") {
            in_hunk = false;
        }
        if !in_hunk {
            let kind = if line.starts_with("--- ") || line.starts_with("+++ ") {
                Kind::File
            } else if line.starts_with("Binary files ") {
                diff.binary = true;
                Kind::Note
            } else {
                Kind::Meta
            };
            diff.rows.push(row(kind, None, None, line));
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                diff.added += 1;
                diff.rows
                    .push(row(Kind::Added, None, Some(new), &line[1..]));
                new += 1;
            }
            Some(b'-') => {
                diff.removed += 1;
                diff.rows
                    .push(row(Kind::Removed, Some(old), None, &line[1..]));
                old += 1;
            }
            Some(b'\\') => diff.rows.push(row(Kind::Note, None, None, line)),
            Some(b' ') => {
                diff.rows
                    .push(row(Kind::Context, Some(old), Some(new), &line[1..]));
                old += 1;
                new += 1;
            }
            // An empty line inside a hunk is a blank context line whose
            // leading space was stripped by an editor or transport.
            None => {
                diff.rows.push(row(Kind::Context, Some(old), Some(new), ""));
                old += 1;
                new += 1;
            }
            _ => {
                in_hunk = false;
                diff.rows.push(row(Kind::Meta, None, None, line));
            }
        }
    }
    diff
}

/// Git's one-letter status for a porcelain row, preferring the working tree.
pub fn status_letter(staged: &str, unstaged: &str) -> char {
    let pick = |s: &str| s.chars().next().filter(|c| *c != ' ');
    match (pick(staged), pick(unstaged)) {
        (Some('?'), _) | (_, Some('?')) => '?',
        (_, Some('U')) | (Some('U'), _) => 'U',
        (_, Some(c)) => c,
        (Some(c), None) => c,
        (None, None) => ' ',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,5 @@ fn main() {
 keep
-old
+new
+more

\\ No newline at end of file
";

    #[test]
    fn rows_carry_old_and_new_line_numbers() {
        let diff = parse(SAMPLE);
        let kinds: Vec<_> = diff.rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Meta,
                Kind::Meta,
                Kind::File,
                Kind::File,
                Kind::Hunk,
                Kind::Context,
                Kind::Removed,
                Kind::Added,
                Kind::Added,
                Kind::Context,
                Kind::Note,
            ]
        );
        assert_eq!((diff.rows[5].old, diff.rows[5].new), (Some(10), Some(10)));
        assert_eq!((diff.rows[6].old, diff.rows[6].new), (Some(11), None));
        assert_eq!((diff.rows[7].old, diff.rows[7].new), (None, Some(11)));
        assert_eq!((diff.rows[8].old, diff.rows[8].new), (None, Some(12)));
        assert_eq!((diff.rows[9].old, diff.rows[9].new), (Some(12), Some(13)));
        assert_eq!(diff.rows[7].text, "new");
        assert_eq!((diff.added, diff.removed), (2, 1));
        assert!(!diff.truncated && !diff.binary);
    }

    #[test]
    fn binary_and_untracked_diffs() {
        let binary = parse(
            "diff --git a/x.png b/x.png\nnew file mode 100644\nBinary files /dev/null and b/x.png differ\n",
        );
        assert!(binary.binary);
        let untracked = parse("--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(untracked.rows.last().unwrap().new, Some(2));
        assert_eq!(untracked.added, 2);
    }

    #[test]
    fn huge_diffs_are_truncated_not_lost_silently() {
        let mut text = String::from("@@ -1,1 +1,30000 @@\n");
        for i in 0..30_000 {
            text.push_str(&format!("+{i}\n"));
        }
        let diff = parse(&text);
        assert!(diff.truncated);
        assert_eq!(diff.rows.len(), MAX_ROWS);
    }

    #[test]
    fn status_letters_prefer_the_working_tree() {
        assert_eq!(status_letter("?", "?"), '?');
        assert_eq!(status_letter("M", " "), 'M');
        assert_eq!(status_letter("A", "M"), 'M');
        assert_eq!(status_letter(" ", "D"), 'D');
        assert_eq!(status_letter("R", " "), 'R');
        assert_eq!(status_letter("U", "U"), 'U');
    }
}
