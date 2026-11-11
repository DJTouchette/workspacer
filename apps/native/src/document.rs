//! What a Markdown document needs before the renderer sees it. The renderer
//! reads CommonMark with tables; documents in a repository also carry front
//! matter and footnotes, which it would print as stray text. These are
//! rewritten into Markdown it does draw. Only the rendered preview uses the
//! result: the source view, saving and line links keep the file as written.
use std::borrow::Cow;

/// The fragment a footnote reference links to; the viewer scrolls it to the
/// footnotes this module appends (see [`FOOTNOTES_HEADING`]).
pub const FOOTNOTES_ANCHOR: &str = "wks-footnotes";
/// The heading the appended footnotes sit under.
pub const FOOTNOTES_HEADING: &str = "Footnotes";

/// Most pictures loaded for one document; the rest stay labels.
pub const MAX_IMAGES: usize = 12;
/// Pictures are scaled down to this many pixels wide before they are sent to
/// the window: twice the reading column, enough for a sharp HiDPI draw.
pub const MAX_IMAGE_WIDTH: u32 = 1600;

/// The pictures `source` shows that are files on the session's machine: the
/// source exactly as the document writes it, and the absolute path it means
/// from the document at `document`. Web addresses are left out on purpose:
/// opening a document must not fetch anything from the network. Code is
/// skipped, repeats are listed once, and at most [`MAX_IMAGES`] are returned.
pub fn images(document: &str, source: &str) -> Vec<(String, String)> {
    use crate::links::{DocumentLink, FileKind, Link, classify_in_document};
    let mut found: Vec<(String, String)> = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for line in source.lines() {
        if let Some((mark, length)) = fence(line) {
            match open {
                Some((opened, least)) if opened == mark && length >= least => open = None,
                None => open = Some((mark, length)),
                Some(_) => {}
            }
            continue;
        }
        if open.is_some() {
            continue;
        }
        for written in image_sources(line) {
            if found.len() == MAX_IMAGES {
                return found;
            }
            if found.iter().any(|(seen, _)| *seen == written) {
                continue;
            }
            if let DocumentLink::Other(Link::File(target)) =
                classify_in_document(document, &written)
                && target.kind == FileKind::Image
            {
                found.push((written, target.path));
            }
        }
    }
    found
}

/// Image sources written on one line outside inline code: `![alt](source)`,
/// `![alt](<source> "title")` and `<img src="source">`.
fn image_sources(line: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut rest = line;
    let mut code: Option<usize> = None;
    while !rest.is_empty() {
        if rest.starts_with('`') {
            let ticks = rest.len() - rest.trim_start_matches('`').len();
            match code {
                Some(opened) if opened == ticks => code = None,
                None => code = Some(ticks),
                Some(_) => {}
            }
            rest = &rest[ticks..];
            continue;
        }
        if code.is_none() {
            if let Some(after) = rest.strip_prefix("![")
                && let Some((_, target)) = after.split_once("](")
                && let Some((inside, tail)) = target.split_once(')')
            {
                let inside = inside.trim();
                let source = match inside.strip_prefix('<') {
                    Some(angled) => angled.split_once('>').map_or(angled, |(s, _)| s),
                    // A title follows the source after a space.
                    None => inside.split([' ', '\t']).next().unwrap_or(inside),
                };
                if !source.is_empty() {
                    sources.push(source.to_owned());
                }
                rest = tail;
                continue;
            }
            if rest.len() >= 4
                && rest.is_char_boundary(4)
                && rest[..4].eq_ignore_ascii_case("<img")
                && let Some((tag, tail)) = rest.split_once('>')
            {
                for quote in ['"', '\''] {
                    let attribute = format!("src={quote}");
                    if let Some(at) = tag.to_ascii_lowercase().find(&attribute)
                        && let Some((source, _)) = tag[at + attribute.len()..].split_once(quote)
                        && !source.is_empty()
                    {
                        sources.push(source.to_owned());
                        break;
                    }
                }
                rest = tail;
                continue;
            }
        }
        let next = rest.chars().next().unwrap();
        rest = &rest[next.len_utf8()..];
    }
    sources
}

/// The document as the preview renders it:
/// - YAML front matter (`---` … `---` at the very top) becomes a `yaml` code
///   block, where it would otherwise run together as a paragraph under a rule;
/// - footnotes (`[^id]` with `[^id]: text` somewhere) become numbered links
///   to a Footnotes list at the end, in the order they are first used.
///
/// Code blocks and inline code are left as written.
pub fn render_source(source: &str) -> Cow<'_, str> {
    let (front, body) = front_matter(source);
    let footnoted = footnotes(body);
    match (front, footnoted) {
        (None, None) => Cow::Borrowed(source),
        (front, footnoted) => {
            let mut out = String::with_capacity(source.len() + 64);
            if let Some(front) = front {
                let fence = fence_for(front);
                out.push_str(&format!("{fence}yaml\n{front}\n{fence}\n\n"));
            }
            out.push_str(footnoted.as_deref().unwrap_or(body));
            Cow::Owned(out)
        }
    }
}

/// A fence longer than any run of backticks inside `text`.
fn fence_for(text: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    "`".repeat(longest.max(2) + 1)
}

/// Splits leading front matter from the rest: the text between an opening
/// `---` on the first line and the next `---` or `...` line.
fn front_matter(source: &str) -> (Option<&str>, &str) {
    let Some(rest) = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
    else {
        return (None, source);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if text == "---" || text == "..." {
            let front = rest[..offset].trim_end_matches(['\n', '\r']);
            // An empty block, or one with a blank first line, is a rule and
            // a paragraph, not front matter.
            if front.trim().is_empty() || front.starts_with(['\n', '\r']) {
                return (None, source);
            }
            return (Some(front), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, source)
}

/// `Some(fence char, length)` when `line` opens or closes a fenced code block.
fn fence(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let text = &line[indent..];
    let mark = text.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = text.chars().take_while(|c| *c == mark).count();
    (length >= 3).then_some((mark, length))
}

/// `[^id]: text` at the start of a line: the id and the text after it.
fn definition(line: &str) -> Option<(&str, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = line[indent..].strip_prefix("[^")?;
    let (id, text) = rest.split_once("]:")?;
    (!id.is_empty() && !id.contains(char::is_whitespace) && !id.contains(['[', ']']))
        .then_some((id, text.trim_start()))
}

/// Rewrites footnotes, or `None` when the document defines none.
fn footnotes(body: &str) -> Option<String> {
    // Pass 1: lift the definitions out, keeping every other line.
    let mut kept: Vec<&str> = Vec::new();
    let mut definitions: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut open: Option<(char, usize)> = None;
    let mut defining = false;
    for line in body.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if let Some((mark, length)) = fence(text) {
            match open {
                Some((opened, least)) if opened == mark && length >= least => open = None,
                None => open = Some((mark, length)),
                Some(_) => {}
            }
            defining = false;
            kept.push(line);
            continue;
        }
        if open.is_some() {
            kept.push(line);
            continue;
        }
        if let Some((id, first)) = definition(text) {
            definitions.push((id, vec![first]));
            defining = true;
            continue;
        }
        // A definition continues over indented lines, and over blank lines
        // between them.
        if defining && (text.starts_with("    ") || text.starts_with('\t')) {
            definitions.last_mut().unwrap().1.push(text.trim_start());
            continue;
        }
        if defining && text.trim().is_empty() {
            definitions.last_mut().unwrap().1.push("");
            continue;
        }
        defining = false;
        kept.push(line);
    }
    if definitions.is_empty() {
        return None;
    }

    // Pass 2: number the references in the order they are first used.
    let mut order: Vec<&str> = Vec::new();
    let mut out = String::with_capacity(body.len() + 128);
    let mut open: Option<(char, usize)> = None;
    for line in kept {
        let text = line.trim_end_matches(['\n', '\r']);
        if let Some((mark, length)) = fence(text) {
            match open {
                Some((opened, least)) if opened == mark && length >= least => open = None,
                None => open = Some((mark, length)),
                Some(_) => {}
            }
            out.push_str(line);
            continue;
        }
        if open.is_some() {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        let mut code: Option<usize> = None;
        while !rest.is_empty() {
            // Inline code: copy through to the matching run of backticks.
            if rest.starts_with('`') {
                let ticks = rest.len() - rest.trim_start_matches('`').len();
                match code {
                    Some(opened) if opened == ticks => code = None,
                    None => code = Some(ticks),
                    Some(_) => {}
                }
                out.push_str(&rest[..ticks]);
                rest = &rest[ticks..];
                continue;
            }
            if code.is_none()
                && let Some(after) = rest.strip_prefix("[^")
                && let Some((id, tail)) = after.split_once(']')
                && let Some(known) = definitions.iter().find(|(d, _)| *d == id)
            {
                let number = match order.iter().position(|o| *o == id) {
                    Some(at) => at + 1,
                    None => {
                        order.push(known.0);
                        order.len()
                    }
                };
                out.push_str(&format!("[\\[{number}\\]](#{FOOTNOTES_ANCHOR})"));
                rest = tail;
                continue;
            }
            let next = rest.chars().next().unwrap();
            out.push(next);
            rest = &rest[next.len_utf8()..];
        }
    }

    // The list: used footnotes by number, then any never referred to.
    for (id, _) in &definitions {
        if !order.contains(id) {
            order.push(id);
        }
    }
    out.truncate(out.trim_end().len());
    out.push_str(&format!("\n\n---\n\n###### {FOOTNOTES_HEADING}\n\n"));
    for (number, id) in order.iter().enumerate() {
        let lines = &definitions.iter().find(|(d, _)| d == id).unwrap().1;
        // Trailing blank lines belonged to the gap after the definition.
        let end = lines
            .iter()
            .rposition(|l| !l.is_empty())
            .map_or(0, |i| i + 1);
        for (row, line) in lines[..end].iter().enumerate() {
            match (row, line.is_empty()) {
                (0, _) => out.push_str(&format!("{}. {line}\n", number + 1)),
                (_, true) => out.push('\n'),
                (_, false) => out.push_str(&format!("   {line}\n")),
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_documents_are_borrowed_unchanged() {
        for source in [
            "# Title\n\nText with a [link](a.md) and `[^code]`.\n",
            "---\n\nA rule, then text.\n",
            "---\nnever closed\n",
            "Text [^missing] with no definition.\n",
            "",
        ] {
            assert!(
                matches!(render_source(source), Cow::Borrowed(_)),
                "{source:?}"
            );
        }
    }

    #[test]
    fn front_matter_becomes_a_yaml_block() {
        assert_eq!(
            render_source("---\ntitle: Notes\ntags: [a, b]\n---\n\n# Notes\n"),
            "```yaml\ntitle: Notes\ntags: [a, b]\n```\n\n\n# Notes\n"
        );
        // Windows line ends, the `...` terminator, and a fence in a value.
        assert_eq!(
            render_source("---\r\na: 1\r\n...\r\nBody\r\n"),
            "```yaml\na: 1\n```\n\nBody\r\n"
        );
        assert!(render_source("---\ncode: \"```\"\n---\nx").starts_with("````yaml\n"));
        // Only at the very top.
        assert!(matches!(
            render_source("Intro\n\n---\na: 1\n---\n"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn footnotes_are_numbered_in_order_of_use_and_listed_at_the_end() {
        let source = "\
Second[^b] comes first, then first[^a], then second again[^b].

[^a]: The first note.
[^b]: The second note,
    over two lines.

    And a second paragraph.

After.
";
        assert_eq!(
            render_source(source),
            "\
Second[\\[1\\]](#wks-footnotes) comes first, then first[\\[2\\]](#wks-footnotes), then second again[\\[1\\]](#wks-footnotes).

After.

---

###### Footnotes

1. The second note,
   over two lines.

   And a second paragraph.
2. The first note.
"
        );
    }

    #[test]
    fn code_is_left_alone_and_unused_notes_are_still_listed() {
        let source = "\
Use `[^a]` like this[^a].

```md
[^a]: not a definition
text[^a]
```

[^a]: Real.
[^unused]: Never referred to.
";
        let rendered = render_source(source);
        assert!(rendered.starts_with("Use `[^a]` like this[\\[1\\]](#wks-footnotes).\n"));
        assert!(rendered.contains("```md\n[^a]: not a definition\ntext[^a]\n```\n"));
        assert!(rendered.ends_with("###### Footnotes\n\n1. Real.\n2. Never referred to.\n"));
    }

    #[test]
    fn images_are_the_documents_own_files_never_web_addresses() {
        let source = "\
![Diagram](img/flow.png) and ![again](img/flow.png \"same\")
![spaced](<shots/first run.jpg>) ![web](https://example.com/a.png)
<img src=\"../logo.webp\" width=\"40\"> <IMG alt='x' src='/abs/icon.gif'>
`![code](img/skip.png)` [not an image](notes.md) ![doc](notes.md)
```
![fenced](img/skip.png)
```
";
        assert_eq!(
            images("/repo/docs/guide.md", source),
            [
                (
                    "img/flow.png".to_owned(),
                    "/repo/docs/img/flow.png".to_owned()
                ),
                (
                    "shots/first run.jpg".to_owned(),
                    "/repo/docs/shots/first run.jpg".to_owned()
                ),
                ("../logo.webp".to_owned(), "/repo/logo.webp".to_owned()),
                ("/abs/icon.gif".to_owned(), "/abs/icon.gif".to_owned()),
            ]
        );
        let many: String = (0..40).map(|n| format!("![p](p{n}.png)\n")).collect();
        assert_eq!(images("/d/a.md", &many).len(), MAX_IMAGES);
    }

    #[test]
    fn front_matter_and_footnotes_combine() {
        let rendered = render_source("---\nx: 1\n---\nBody[^n]\n\n[^n]: Note.\n");
        assert_eq!(
            rendered,
            "```yaml\nx: 1\n```\n\nBody[\\[1\\]](#wks-footnotes)\n\n---\n\n###### Footnotes\n\n1. Note.\n"
        );
    }
}
