//! Where a background shell's output really goes.
//!
//! A command like `deploy.ps1 *> $log` or `make | tee build.log` sends its
//! output past the CLI's task file, which then stays empty for as long as the
//! work runs. This reads the command — the CLI reports a shell task's whole
//! command line as its `description` — for its last stdout redirect, so the
//! log panel can follow that file instead.
//!
//! Parsing is best-effort and pure. Bash and PowerShell spellings are both
//! understood (`>`, `>>`, `1>`, `&>`, `*>`, `| tee`, `| Tee-Object`,
//! `| Out-File`, `| Add-Content`, `| Set-Content`,
//! `-RedirectStandardOutput`), and so are variables the command itself
//! assigns (`$log = "…"; … *> $log`, `log=…; … > "$log"`), environment
//! variables (`$env:X`, `$X`, `%X%`) and a leading `~`. A target that cannot
//! be resolved without guessing — an unknown variable, a subexpression, a
//! relative path with no session cwd — yields nothing.
//!
//! The path is the daemon's own derivation from the CLI's frames; clients
//! only ever name the task. A read additionally requires the file to have
//! been written since the task started (see [`redirect_file`]).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::background_tasks::{BackgroundTask, OutputError};

/// A file written this long before the task's recorded start still counts
/// (the daemon stamps `started_at` when it first sees the task).
const START_SLACK_MS: i64 = 5_000;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    /// A redirect of stdout (`>`, `>>`, `1>`, `&>`, `*>` …): the next word
    /// is its target.
    Stdout,
    /// A redirect that does not carry stdout (`2>`, `<`): the next word is
    /// consumed and ignored.
    Other,
    Pipe,
    /// `;`, `&&`, `||`, `&`, a newline: a new statement starts.
    Sep,
}

fn lex(cmd: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut word = String::new();
    let mut chars = cmd.chars().peekable();
    // Quotes stay in the word; expansion reads them.
    let mut quote: Option<char> = None;
    let flush = |word: &mut String, toks: &mut Vec<Tok>| {
        if !word.is_empty() {
            toks.push(Tok::Word(std::mem::take(word)));
        }
    };
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            word.push(c);
            if c == '`' && q == '"' {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                word.push(c);
            }
            '`' => {
                word.push(c);
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            '>' => {
                // The word so far names the stream: none, `1` or `*` is
                // stdout, another number is another stream, and anything else
                // is a word the operator ends (`echo hi>out`).
                let stream = std::mem::take(&mut word);
                let numbered = !stream.is_empty() && stream.chars().all(|c| c.is_ascii_digit());
                let stdout = !numbered || stream == "1";
                if !numbered && !stream.is_empty() && stream != "*" {
                    toks.push(Tok::Word(stream));
                }
                if chars.peek() == Some(&'>') {
                    chars.next();
                }
                if chars.peek() == Some(&'&') {
                    // `>&1`, `2>&1`: a stream duplicate, no file.
                    chars.next();
                    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                        chars.next();
                    }
                    continue;
                }
                toks.push(if stdout { Tok::Stdout } else { Tok::Other });
            }
            '<' => {
                flush(&mut word, &mut toks);
                toks.push(Tok::Other);
            }
            '&' => {
                flush(&mut word, &mut toks);
                match chars.peek() {
                    Some('>') => {
                        chars.next();
                        if chars.peek() == Some(&'>') {
                            chars.next();
                        }
                        toks.push(Tok::Stdout);
                    }
                    Some('&') => {
                        chars.next();
                        toks.push(Tok::Sep);
                    }
                    _ => toks.push(Tok::Sep),
                }
            }
            '|' => {
                flush(&mut word, &mut toks);
                if chars.peek() == Some(&'|') {
                    chars.next();
                    toks.push(Tok::Sep);
                } else {
                    toks.push(Tok::Pipe);
                }
            }
            ';' | '\n' | '\r' => {
                flush(&mut word, &mut toks);
                toks.push(Tok::Sep);
            }
            c if c.is_whitespace() => flush(&mut word, &mut toks),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut toks);
    toks
}

fn home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|h| !h.is_empty())
}

/// Variables the command assigned so far (`log`, `env:X`).
#[derive(Default)]
struct Vars(HashMap<String, String>);

impl Vars {
    fn get(&self, name: &str) -> Option<String> {
        if let Some(v) = self.0.get(name) {
            return Some(v.clone());
        }
        // PowerShell names are case-insensitive.
        let lower = name.to_ascii_lowercase();
        if let Some((_, v)) = self.0.iter().find(|(k, _)| k.to_ascii_lowercase() == lower) {
            return Some(v.clone());
        }
        if let Some(env) = name
            .strip_prefix("env:")
            .or_else(|| name.strip_prefix("ENV:"))
        {
            return std::env::var(env).ok();
        }
        if lower == "home" {
            return home();
        }
        std::env::var(name).ok()
    }
}

fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Unquote and expand one word. `None` when it uses anything this cannot
/// evaluate (a subexpression, an unknown variable).
fn expand(raw: &str, vars: &Vars) -> Option<String> {
    let mut out = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut quote: Option<char> = None;
    if chars.first() == Some(&'~') && matches!(chars.get(1), None | Some('/') | Some('\\')) {
        out.push_str(&home()?);
        i = 1;
    }
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), c) => out.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (_, '`') => {
                i += 1;
                if let Some(&next) = chars.get(i) {
                    out.push(next);
                }
            }
            (_, '$') => {
                let rest = &chars[i + 1..];
                let (name, used) = match rest.first() {
                    Some('{') => {
                        let end = rest.iter().position(|c| *c == '}')?;
                        (rest[1..end].iter().collect::<String>(), end + 1)
                    }
                    Some('(') => return None,
                    Some(_) => {
                        let mut n = 0;
                        // `$env:NAME`: the scope prefix is part of the name.
                        if rest.len() > 4
                            && rest[..4]
                                .iter()
                                .collect::<String>()
                                .eq_ignore_ascii_case("env:")
                        {
                            n = 4;
                        }
                        while rest.get(n).is_some_and(|c| ident(*c)) {
                            n += 1;
                        }
                        (rest[..n].iter().collect::<String>(), n)
                    }
                    None => (String::new(), 0),
                };
                if name.is_empty() || name.eq_ignore_ascii_case("env:") {
                    out.push('$');
                } else {
                    out.push_str(&vars.get(&name)?);
                    i += used;
                }
            }
            (_, '%') => {
                // `%NAME%` when NAME is set; a lone percent is literal.
                let rest = &chars[i + 1..];
                let set = rest.iter().position(|c| *c == '%').and_then(|end| {
                    let name: String = rest[..end].iter().collect();
                    (!name.is_empty() && name.chars().all(ident))
                        .then(|| std::env::var(&name).ok())
                        .flatten()
                        .map(|v| (v, end + 1))
                });
                match set {
                    Some((value, used)) => {
                        out.push_str(&value);
                        i += used;
                    }
                    None => out.push('%'),
                }
            }
            (_, c) => out.push(c),
        }
        i += 1;
    }
    (!out.is_empty()).then_some(out)
}

/// `name=value` / `$name=value` as one word (bash or PowerShell).
fn inline_assignment(word: &str) -> Option<(String, &str)> {
    let (name, value) = word.split_once('=')?;
    let name = name.strip_prefix('$').unwrap_or(name);
    let bare = name
        .strip_prefix("env:")
        .or_else(|| name.strip_prefix("ENV:"))
        .unwrap_or(name);
    (!bare.is_empty() && !bare.starts_with(|c: char| c.is_ascii_digit()) && bare.chars().all(ident))
        .then(|| (name.to_string(), value))
}

fn is_null(target: &str) -> bool {
    matches!(
        target.to_ascii_lowercase().as_str(),
        "/dev/null" | "nul" | "null" | "$null" | "nul:"
    )
}

/// Flags of the file-writing cmdlets that take a value which is not the path.
fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-encoding" | "-width" | "-variable" | "-value" | "-inputobject" | "-stream" | "-delimiter"
    )
}

/// The file a piped `tee` / `Tee-Object` / `Out-File` / `Add-Content` /
/// `Set-Content` writes, as its raw word.
fn pipe_target(args: &[&String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let lower = arg.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "-filepath" | "-path" | "-literalpath" | "-pspath" | "-lp"
        ) {
            return iter.next().map(|s| s.to_string());
        }
        if lower.starts_with('-') {
            if takes_value(&lower) {
                iter.next();
            }
            continue;
        }
        return Some(arg.to_string());
    }
    None
}

/// The last stdout redirect target of `command`, expanded but not yet
/// resolved against a directory.
pub fn redirect_target(command: &str) -> Option<String> {
    let toks = lex(command);
    let mut vars = Vars::default();
    let mut found = None;
    let mut i = 0;
    let mut stmt_start = true;
    while i < toks.len() {
        match &toks[i] {
            Tok::Sep => stmt_start = true,
            Tok::Pipe => {
                // `… | tee [flags] FILE`
                let words: Vec<&String> = toks[i + 1..]
                    .iter()
                    .map_while(|t| match t {
                        Tok::Word(w) => Some(w),
                        _ => None,
                    })
                    .collect();
                let cmdlet = words.first().map(|w| w.to_ascii_lowercase());
                if matches!(
                    cmdlet.as_deref(),
                    Some(
                        "tee"
                            | "tee-object"
                            | "out-file"
                            | "add-content"
                            | "set-content"
                            | "ac"
                            | "sc"
                    )
                ) {
                    if let Some(target) = pipe_target(&words[1..])
                        .and_then(|raw| expand(&raw, &vars))
                        .filter(|t| !is_null(t))
                    {
                        found = Some(target);
                    }
                    i += words.len();
                }
                stmt_start = false;
            }
            Tok::Stdout => {
                if let Some(Tok::Word(raw)) = toks.get(i + 1) {
                    if let Some(target) = expand(raw, &vars).filter(|t| !is_null(t)) {
                        found = Some(target);
                    }
                    i += 1;
                }
            }
            Tok::Other => {
                if matches!(toks.get(i + 1), Some(Tok::Word(_))) {
                    i += 1;
                }
            }
            Tok::Word(word) => {
                let ends = |at: usize| matches!(toks.get(at), None | Some(Tok::Sep));
                if stmt_start {
                    // `$log = "…"` (three words) or `log=…` / `$log="…"`.
                    if let (Some(name), Some(Tok::Word(eq)), Some(Tok::Word(value))) =
                        (word.strip_prefix('$'), toks.get(i + 1), toks.get(i + 2))
                    {
                        if eq == "=" && ends(i + 3) {
                            if let Some(value) = expand(value, &vars) {
                                vars.0.insert(name.to_string(), value);
                            }
                            i += 3;
                            continue;
                        }
                    }
                    if let Some((name, value)) = inline_assignment(word) {
                        if let Some(value) = expand(value, &vars) {
                            vars.0.insert(name, value);
                        }
                    }
                }
                if word.eq_ignore_ascii_case("-RedirectStandardOutput") {
                    if let Some(target) = toks
                        .get(i + 1)
                        .and_then(|t| match t {
                            Tok::Word(raw) => expand(raw, &vars),
                            _ => None,
                        })
                        .filter(|t| !is_null(t))
                    {
                        found = Some(target);
                    }
                    i += 1;
                }
                stmt_start = false;
            }
        }
        i += 1;
    }
    found
}

/// `/c/Users/…` (Git Bash on Windows) as `C:\Users\…`.
#[cfg(windows)]
fn native_spelling(target: &str) -> String {
    let b = target.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
        return format!(
            "{}:\\{}",
            (b[1] as char).to_ascii_uppercase(),
            target[3..].replace('/', "\\")
        );
    }
    target.to_string()
}

#[cfg(not(windows))]
fn native_spelling(target: &str) -> String {
    target.to_string()
}

/// The redirect target as a path, relative ones joined to `cwd`.
pub fn resolve(target: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let path = PathBuf::from(native_spelling(target));
    if path.is_absolute() {
        return Some(path);
    }
    let cwd = Path::new(cwd.filter(|c| !c.is_empty())?);
    cwd.is_absolute().then(|| cwd.join(path))
}

/// The file name shown on the wire (`redirectFile`): never a directory.
pub fn display_name(command: &str) -> Option<String> {
    let target = native_spelling(&redirect_target(command)?);
    Path::new(&target)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// The file a shell task redirects into, when it is one this task wrote:
/// a regular file (never a symlink) modified since the task started.
pub fn redirect_file(task: &BackgroundTask, cwd: Option<&str>) -> Result<PathBuf, OutputError> {
    if task.task_type == "local_agent" {
        return Err(OutputError::NoOutput);
    }
    let path = task
        .description
        .as_deref()
        .and_then(redirect_target)
        .and_then(|t| resolve(&t, cwd))
        .ok_or(OutputError::NoOutput)?;
    let meta = std::fs::symlink_metadata(&path).map_err(|_| {
        OutputError::Unavailable("the redirected log has not been written yet".into())
    })?;
    if !meta.file_type().is_file() {
        return Err(OutputError::Unavailable(
            "the redirect target is not a regular file".into(),
        ));
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    if modified.is_none_or(|m| m + START_SLACK_MS < task.started_at) {
        return Err(OutputError::Unavailable(
            "the redirect target has not been written since the task started".into(),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(cmd: &str) -> Option<String> {
        redirect_target(cmd)
    }

    #[test]
    fn bash_redirects() {
        assert_eq!(target("make > build.log").as_deref(), Some("build.log"));
        assert_eq!(
            target("make >>build.log 2>&1").as_deref(),
            Some("build.log")
        );
        assert_eq!(
            target("make 1> out.txt 2> err.txt").as_deref(),
            Some("out.txt")
        );
        assert_eq!(
            target("make &> /tmp/all.log").as_deref(),
            Some("/tmp/all.log")
        );
        assert_eq!(
            target("make 2>&1 | tee -a /tmp/b.log").as_deref(),
            Some("/tmp/b.log")
        );
        assert_eq!(target("make > /dev/null"), None);
        assert_eq!(
            target("make 2> err.txt"),
            None,
            "stderr alone leaves stdout in the task file"
        );
        assert_eq!(target("echo 'a > b'"), None, "quoted operators are text");
        assert_eq!(target("x > \"my log.txt\"").as_deref(), Some("my log.txt"));
    }

    #[test]
    fn the_last_redirect_wins() {
        assert_eq!(
            target("echo start > a.log; make > b.log; echo done").as_deref(),
            Some("b.log")
        );
    }

    #[test]
    fn powershell_redirects_and_cmdlets() {
        assert_eq!(
            target("build *> C:\\x\\b.log").as_deref(),
            Some("C:\\x\\b.log")
        );
        assert_eq!(
            target("build 2>$null *>> 'C:\\x\\c.log'").as_deref(),
            Some("C:\\x\\c.log")
        );
        assert_eq!(
            target("build | Tee-Object -FilePath C:\\x\\t.log -Append").as_deref(),
            Some("C:\\x\\t.log")
        );
        assert_eq!(
            target("build | Out-File -Encoding utf8 C:\\x\\o.log").as_deref(),
            Some("C:\\x\\o.log")
        );
        assert_eq!(
            target("Start-Process app -RedirectStandardOutput C:\\x\\s.log -Wait").as_deref(),
            Some("C:\\x\\s.log")
        );
        assert_eq!(target("build | Out-Null"), None);
    }

    #[test]
    fn variables_the_command_assigns_are_followed() {
        // The shape that motivated this: the whole log behind `*> $log`.
        let cmd =
            "$env:PATH = \"$env:PATH;x\"; $log = \"C:\\Users\\me\\scratch\\waf-deploy.log\"; \
                   & \"C:\\infra\\infraWin.ps1\" deploy backend *> $log; \"rc=$LASTEXITCODE\"";
        assert_eq!(
            target(cmd).as_deref(),
            Some("C:\\Users\\me\\scratch\\waf-deploy.log")
        );
        assert_eq!(
            display_name(cmd).as_deref(),
            Some(if cfg!(windows) {
                "waf-deploy.log"
            } else {
                "C:\\Users\\me\\scratch\\waf-deploy.log"
            })
        );
        assert_eq!(
            target("LOG=/tmp/x.log; make > \"$LOG\"").as_deref(),
            Some("/tmp/x.log")
        );
        assert_eq!(
            target("$l='/tmp/y.log'; make *> $l").as_deref(),
            Some("/tmp/y.log")
        );
        assert_eq!(
            target("make > $UNSET_FOR_SURE_7f3"),
            None,
            "unknown variables are not guessed"
        );
        assert_eq!(
            target("make > $(mktemp)"),
            None,
            "subexpressions are not evaluated"
        );
    }

    #[test]
    fn environment_and_home_expand() {
        let home = home().unwrap();
        assert_eq!(target("make > ~/b.log"), Some(format!("{home}/b.log")));
        assert_eq!(
            target("make > \"$HOME/c.log\""),
            Some(format!("{home}/c.log"))
        );
        // Test-only, a name no other test reads.
        std::env::set_var("WKS_REDIRECT_TEST_DIR", "/srv/logs");
        assert_eq!(
            target("make > $env:WKS_REDIRECT_TEST_DIR/d.log").as_deref(),
            Some("/srv/logs/d.log")
        );
        assert_eq!(
            target("make > %WKS_REDIRECT_TEST_DIR%/e.log").as_deref(),
            Some("/srv/logs/e.log")
        );
        assert_eq!(
            target("make > '$HOME/f.log'").as_deref(),
            Some("$HOME/f.log"),
            "single quotes are literal"
        );
    }

    #[test]
    fn relative_targets_need_a_cwd() {
        assert_eq!(resolve("b.log", None), None);
        let cwd = std::env::temp_dir();
        let cwd = cwd.to_str().unwrap();
        assert_eq!(
            resolve("b.log", Some(cwd)),
            Some(Path::new(cwd).join("b.log"))
        );
    }

    #[test]
    fn a_read_requires_a_file_this_task_wrote() {
        let dir = std::env::temp_dir().join(format!("wks-redirect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("run.log");
        std::fs::write(&log, "hello\n").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let mut task = BackgroundTask::new("b1", "local_bash", now);
        task.description = Some(format!("make > {}", log.display()));
        assert_eq!(redirect_file(&task, None).unwrap(), log);

        // A file untouched since well before the task began is not its log.
        task.started_at = now + 60_000;
        assert!(matches!(
            redirect_file(&task, None),
            Err(OutputError::Unavailable(_))
        ));

        task.started_at = now;
        task.description = Some(format!("make > {}", dir.join("missing.log").display()));
        assert!(matches!(
            redirect_file(&task, None),
            Err(OutputError::Unavailable(_))
        ));

        task.description = Some("make".into());
        assert_eq!(redirect_file(&task, None), Err(OutputError::NoOutput));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
