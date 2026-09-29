//! Compatibility for Go's regexp syntax used by persisted skipUnlessMatch guards.
//! ASCII Perl classes/boundaries are explicit; Unicode properties retain Rust's
//! Unicode tables (which can be newer than the former Go toolchain's tables).
use anyhow::{Result, bail, ensure};
use regex_syntax::ast::{self, Ast, ClassSet, ClassSetItem};

pub(super) fn compile(pattern: &str) -> Result<regex::Regex> {
    let normalized = normalize(pattern)?;
    let mut ast = ast::parse::ParserBuilder::new()
        .octal(true)
        .nest_limit(1000)
        .build()
        .parse(&normalized)?;
    rewrite(&mut ast, 1000)?;
    Ok(regex::RegexBuilder::new(&ast.to_string())
        .octal(true)
        .nest_limit(1000)
        .build()?)
}

// Go classes have no nested sets or intersection operators. Preserve those
// spellings as literals before the Rust AST parser can reinterpret them.
fn normalize(pattern: &str) -> Result<String> {
    let mut out = String::new();
    let mut offset = 0;
    let mut class = false;
    let mut first = false;
    while offset < pattern.len() {
        let rest = &pattern[offset..];
        let c = rest.chars().next().unwrap();
        if c == '\\' {
            let Some(next) = rest[1..].chars().next() else {
                bail!("trailing regex escape")
            };
            if next == 'Q' {
                ensure!(
                    !class,
                    "quoted literals are not valid inside a Go character class"
                );
                let quoted = &rest[2..];
                if let Some(end) = quoted.find("\\E") {
                    out.push_str(&regex::escape(&quoted[..end]));
                    offset += 2 + end + 2;
                } else {
                    out.push_str(&regex::escape(quoted));
                    offset = pattern.len();
                }
                continue;
            }
            if matches!(next, '<' | '>') {
                // Go quotes this punctuation; Rust treats it as a boundary.
                out.push(next);
                offset += 2;
                if class {
                    first = false;
                }
                continue;
            }
            if next == 'b' && !class {
                if let Some(suffix) = ["{start}", "{end}", "{start-half}", "{end-half}"]
                    .into_iter()
                    .find(|suffix| rest[2..].starts_with(suffix))
                {
                    // In Go these are a boundary followed by literal braces,
                    // not the extra zero-width assertions Rust offers.
                    out.push_str(r"\b");
                    out.push_str(&regex::escape(suffix));
                    offset += 2 + suffix.len();
                    continue;
                }
            }
            ensure!(
                !matches!(next, 'u' | 'U'),
                "Go regexp does not support Unicode codepoint escape {next}"
            );
            if ('1'..='7').contains(&next) {
                ensure!(
                    rest.as_bytes()
                        .get(2)
                        .is_some_and(|b| (b'0'..=b'7').contains(b))
                        && rest
                            .as_bytes()
                            .get(3)
                            .is_some_and(|b| (b'0'..=b'7').contains(b)),
                    "Go regexp does not support backreferences"
                );
            }
            out.push('\\');
            out.push(next);
            offset += 1 + next.len_utf8();
            if class {
                first = false;
            }
            continue;
        }
        if class {
            if c == ']' && !first {
                class = false;
                out.push(c);
                offset += 1;
                continue;
            }
            if c == '^' && first && out.ends_with('[') {
                out.push(c);
                offset += 1;
                continue;
            }
            if rest.starts_with("[:") {
                let end = rest
                    .find(":]")
                    .ok_or_else(|| anyhow::anyhow!("unclosed POSIX class"))?;
                out.push_str(&rest[..end + 2]);
                offset += end + 2;
                first = false;
                continue;
            }
            if c == '[' || c == '&' || c == '~' || (c == ']' && first) || (c == '-' && first) {
                out.push('\\');
            }
            if rest.starts_with("--") && !first {
                bail!("invalid Go character range")
            }
            out.push(c);
            offset += c.len_utf8();
            first = false;
            continue;
        }
        if c == '[' {
            class = true;
            first = true;
        }
        // Captures are not observed by these boolean guards. Converting valid
        // named groups also preserves Go's allowance of duplicate/digit names.
        if let Some(prefix) = ["(?P<", "(?<"]
            .into_iter()
            .find(|prefix| rest.starts_with(prefix))
        {
            let end = rest
                .find('>')
                .ok_or_else(|| anyhow::anyhow!("unclosed capture name"))?;
            let name = &rest[prefix.len()..end];
            ensure!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid Go capture name"
            );
            out.push_str("(?:");
            offset += end + 1;
            continue;
        }
        out.push(c);
        offset += c.len_utf8();
    }
    Ok(out)
}
fn parse(pattern: &str) -> Result<Ast> {
    Ok(ast::parse::Parser::new().parse(pattern)?)
}
fn perl(class: &ast::ClassPerl) -> Result<Box<ast::ClassBracketed>> {
    let body = match class.kind {
        ast::ClassPerlKind::Digit => "0-9",
        ast::ClassPerlKind::Word => "0-9A-Za-z_",
        ast::ClassPerlKind::Space => r"\t\n\f\r ",
    };
    let ast = parse(&format!("[{}{body}]", if class.negated { "^" } else { "" }))?;
    match &ast {
        Ast::ClassBracketed(class) => Ok(class.clone()),
        _ => unreachable!(),
    }
}
fn flags(flags: &ast::Flags) -> Result<()> {
    for item in &flags.items {
        if let ast::FlagsItemKind::Flag(flag) = item.kind {
            ensure!(
                matches!(
                    flag,
                    ast::Flag::CaseInsensitive
                        | ast::Flag::MultiLine
                        | ast::Flag::DotMatchesNewLine
                        | ast::Flag::SwapGreed
                ),
                "unsupported Go regexp flag"
            );
        }
    }
    Ok(())
}
fn unicode(class: &mut ast::ClassUnicode) -> Result<Option<Box<ast::ClassBracketed>>> {
    let name = match &mut class.kind {
        ast::ClassUnicodeKind::OneLetter(letter) => letter.to_string(),
        ast::ClassUnicodeKind::Named(name) => {
            if let Some(positive) = name.strip_prefix('^') {
                let positive = positive.to_owned();
                *name = positive;
                class.negated = !class.negated;
            }
            name.clone()
        }
        ast::ClassUnicodeKind::NamedValue { .. } => {
            bail!("Go regexp accepts Unicode categories/scripts, not property assignments")
        }
    };
    let canonical = unicode_names()
        .get(&canonical_unicode_name(&name))
        .ok_or_else(|| anyhow::anyhow!("unknown Go Unicode category or script {name}"))?;
    if matches!(canonical.as_str(), "Cs" | "Surrogate") {
        // Surrogates cannot occur as UTF-8 scalar values in MatchString.
        // Go accepts the property; Rust omits it from its Unicode namespace.
        let replacement = parse(if class.negated {
            r"[\x00-\x{10FFFF}]"
        } else {
            r"[\x00&&\x01]"
        })?;
        return match &replacement {
            Ast::ClassBracketed(class) => Ok(Some(class.clone())),
            _ => unreachable!(),
        };
    }
    class.kind = ast::ClassUnicodeKind::Named(canonical.clone());
    Ok(None)
}
fn set(set: &mut ClassSet) -> Result<()> {
    match set {
        ClassSet::Item(item) => class_item(item),
        ClassSet::BinaryOp(_) => bail!("Go regexp has no class set operators"),
    }
}
fn class_item(item: &mut ClassSetItem) -> Result<()> {
    match item {
        ClassSetItem::Perl(class) => *item = ClassSetItem::Bracketed(perl(class)?),
        ClassSetItem::Unicode(class) => {
            if let Some(replacement) = unicode(class)? {
                *item = ClassSetItem::Bracketed(replacement);
            }
        }
        ClassSetItem::Bracketed(class) => set(&mut class.kind)?,
        ClassSetItem::Union(union) => {
            for item in &mut union.items {
                class_item(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn rewrite(ast: &mut Ast, budget: u32) -> Result<()> {
    match ast {
        Ast::ClassPerl(class) => *ast = Ast::ClassBracketed(perl(class)?),
        Ast::ClassBracketed(class) => set(&mut class.kind)?,
        Ast::ClassUnicode(class) => {
            if let Some(replacement) = unicode(class)? {
                *ast = Ast::ClassBracketed(replacement);
            }
        }
        Ast::Assertion(assertion) => match assertion.kind {
            ast::AssertionKind::WordBoundary => *ast = parse(r"(?-u:\b)")?,
            ast::AssertionKind::NotWordBoundary => *ast = parse(r"(?-u:\B)")?,
            ast::AssertionKind::StartLine
            | ast::AssertionKind::EndLine
            | ast::AssertionKind::StartText
            | ast::AssertionKind::EndText => {}
            _ => bail!("unsupported Go boundary assertion"),
        },
        Ast::Flags(value) => flags(&value.flags)?,
        Ast::Group(group) => {
            if let Some(value) = group.flags() {
                flags(value)?;
            }
            rewrite(&mut group.ast, budget)?;
        }
        Ast::Repetition(repeat) => {
            let count = match repeat.op.kind {
                ast::RepetitionKind::Range(
                    ast::RepetitionRange::Exactly(n) | ast::RepetitionRange::AtLeast(n),
                ) => n,
                ast::RepetitionKind::Range(ast::RepetitionRange::Bounded(_, n)) => n,
                _ => 1,
            };
            ensure!(count <= budget, "Go regexp repetition exceeds 1000");
            rewrite(
                &mut repeat.ast,
                if count == 0 { 1000 } else { budget / count },
            )?;
        }
        Ast::Alternation(value) => {
            for item in &mut value.asts {
                rewrite(item, budget)?;
            }
        }
        Ast::Concat(value) => {
            for item in &mut value.asts {
                rewrite(item, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn canonical_unicode_name(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
fn unicode_names() -> &'static std::collections::BTreeMap<String, String> {
    static NAMES: std::sync::OnceLock<std::collections::BTreeMap<String, String>> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("go_regex_names.json"))
            .expect("captured Go Unicode names");
        fixture["accepted"]
            .as_array()
            .expect("Go property list")
            .iter()
            .map(|name| {
                let name = name.as_str().expect("Go property name");
                (canonical_unicode_name(name), name.to_owned())
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_namespace_is_pinned_by_successful_go_compilation() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("go_regex_names.json")).unwrap();
        assert!(
            fixture["meta"]["goVersion"]
                .as_str()
                .unwrap()
                .starts_with("go")
        );
        assert_eq!(fixture["meta"]["unicodeVersion"], "15.0.0");
        let names = fixture["accepted"].as_array().unwrap();
        assert!(names.len() >= 200);
        for name in names {
            let name = name.as_str().unwrap();
            compile(&format!(r"\p{{{name}}}"))
                .unwrap_or_else(|e| panic!("Go accepted {name}: {e:#}"));
        }
        for (name, accepted) in fixture["probes"].as_object().unwrap() {
            assert_eq!(
                compile(&format!(r"\p{{{name}}}")).is_ok(),
                accepted.as_bool().unwrap(),
                "{name}"
            );
        }
    }
    #[test]
    fn primary_go_regexp_guard_matrix() {
        // Expected validity/matches captured with Go's stdlib regexp. This is
        // a test oracle only, never a runtime or build dependency on Go.
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("go_regex_cases.json")).unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() >= 63, "guard compatibility matrix shrank");
        for case in cases {
            let pattern = case["pattern"].as_str().unwrap();
            if case["invalid"] == true {
                assert!(
                    compile(pattern).is_err(),
                    "accepted invalid Go pattern {pattern:?}"
                );
            } else {
                let text = case["text"].as_str().unwrap();
                let regex =
                    compile(pattern).unwrap_or_else(|error| panic!("{pattern:?}: {error:#}"));
                assert_eq!(
                    regex.is_match(text),
                    case["matches"].as_bool().unwrap(),
                    "pattern={pattern:?}, text={text:?}"
                );
            }
        }
    }
}
