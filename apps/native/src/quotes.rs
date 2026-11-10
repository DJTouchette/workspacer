//! Quoted passages in a message: text selected in the chat that a reply
//! carries ahead of its own words.
//!
//! A quote travels as a Markdown blockquote at the head of the message, one
//! `> ` line per quoted line, each quote separated from the next (and from
//! the reply) by a blank line. Every provider reads that as a quote, and
//! [`split`] finds it again to draw it as one.

/// The most a single quote keeps: enough for a long passage, not a whole
/// transcript pasted back at the agent.
pub const MAX_CHARS: usize = 4000;

/// A selection as a quote: trimmed, without trailing space on its lines, cut
/// at [`MAX_CHARS`] with an ellipsis. `None` when nothing but space was
/// selected.
pub fn clean(selection: &str) -> Option<String> {
    let lines: Vec<&str> = selection.trim().lines().map(str::trim_end).collect();
    let text = lines.join("\n");
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text,
    })
}

/// The head of a message carrying `quotes`: each as a blockquote followed by
/// a blank line. Empty when there are none.
pub fn prefix(quotes: &[String]) -> String {
    let mut head = String::new();
    for quote in quotes {
        for line in quote.lines() {
            head.push('>');
            if !line.is_empty() {
                head.push(' ');
                head.push_str(line);
            }
            head.push('\n');
        }
        head.push('\n');
    }
    head
}

/// The quotes a message opens with, and the rest of it. A message that does
/// not start with a blockquote has none and is returned whole.
pub fn split(text: &str) -> (Vec<String>, &str) {
    let mut quotes = Vec::new();
    let mut rest = text;
    loop {
        let mut quote = Vec::new();
        let mut after = rest;
        while let Some(line) = after.split_inclusive('\n').next() {
            let Some(body) = line.trim_end_matches(['\n', '\r']).strip_prefix('>') else {
                break;
            };
            quote.push(body.strip_prefix(' ').unwrap_or(body));
            after = &after[line.len()..];
        }
        if quote.is_empty() {
            break;
        }
        quotes.push(quote.join("\n"));
        // The blank lines that end a quote belong to neither side.
        rest = after.trim_start_matches(['\n', '\r']);
    }
    (quotes, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_trimmed_and_blank_ones_are_refused() {
        assert_eq!(clean("  one  \n  two\t\n\n"), Some("one\n  two".into()));
        assert_eq!(clean(" \n\t"), None);
    }

    #[test]
    fn a_long_selection_is_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_CHARS + 10);
        let cut = clean(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn quotes_round_trip_ahead_of_the_reply() {
        let quotes = vec!["first line\n\nafter a gap".to_owned(), "second".to_owned()];
        let message = format!("{}What about this?\n> not a quote", prefix(&quotes));
        assert_eq!(
            message,
            "> first line\n>\n> after a gap\n\n> second\n\nWhat about this?\n> not a quote"
        );
        let (found, rest) = split(&message);
        assert_eq!(found, quotes);
        assert_eq!(rest, "What about this?\n> not a quote");
    }

    #[test]
    fn a_message_without_a_leading_quote_is_left_whole() {
        assert_eq!(split("hello\n> later"), (vec![], "hello\n> later"));
        assert_eq!(prefix(&[]), "");
    }

    #[test]
    fn a_quote_alone_leaves_no_reply() {
        assert_eq!(split("> only\r\n"), (vec!["only".to_owned()], ""));
    }
}
