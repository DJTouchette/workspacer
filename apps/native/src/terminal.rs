//! Per-agent interactive shells on the connected hub.
//!
//! Every shell is a real process owned by the hub's engine (`terminals.create`
//! spawns the host's login shell in the agent's folder); this client never
//! starts a process of its own. The controller keeps one shell per agent and
//! attaches to it while it is on screen. Output arrives as `pty.bytes.<id>`
//! events: the controller queues the bytes here, outside the per-frame view
//! snapshot, and the window drains them into its emulator. Attaching replays
//! the shell's retained screen, so reopening an agent's terminal shows the
//! same shell with its output.
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

/// Bytes queued for one shell before the window drains them. A window that
/// stops draining (minimized, no frame) cannot grow this without bound; the
/// controller re-attaches instead, which replays the current screen.
pub const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
/// Lines of scrollback each emulator keeps.
pub const SCROLLBACK_LINES: usize = 5_000;
/// Default size until the window has measured the terminal.
pub const DEFAULT_COLS: u16 = 100;
pub const DEFAULT_ROWS: u16 = 24;
/// Most agents with a remembered shell (persisted per hub).
pub const MAX_REMEMBERED: usize = 256;

/// Output waiting for the window, per shell id.
#[derive(Debug, Default)]
pub struct Feed {
    inner: Mutex<HashMap<String, Pending>>,
}

#[derive(Debug, Default)]
struct Pending {
    reset: bool,
    bytes: Vec<u8>,
}

/// What the window should apply to a shell's emulator.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Chunk {
    /// Start a fresh emulator first: an attach is about to replay the screen.
    pub reset: bool,
    pub bytes: Vec<u8>,
}

impl Feed {
    /// Queue output. `false` means the window fell too far behind and the
    /// queue was dropped; the caller must re-attach for a fresh replay.
    pub fn push(&self, shell: &str, bytes: &[u8]) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let pending = inner.entry(shell.to_owned()).or_default();
        if pending.bytes.len() + bytes.len() > MAX_PENDING_BYTES {
            pending.bytes.clear();
            pending.reset = true;
            return false;
        }
        pending.bytes.extend_from_slice(bytes);
        true
    }

    /// The next bytes are a replay: discard anything older.
    pub fn reset(&self, shell: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let pending = inner.entry(shell.to_owned()).or_default();
        pending.bytes.clear();
        pending.reset = true;
    }

    pub fn take(&self, shell: &str) -> Option<Chunk> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let pending = inner.get_mut(shell)?;
        if !pending.reset && pending.bytes.is_empty() {
            return None;
        }
        Some(Chunk {
            reset: std::mem::take(&mut pending.reset),
            bytes: std::mem::take(&mut pending.bytes),
        })
    }

    pub fn forget(&self, shell: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(shell);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Asking the hub to start the shell.
    #[default]
    Starting,
    /// Shell exists; waiting for the attach to replay its screen.
    Attaching,
    /// Output is streaming.
    Live,
    /// Hidden: the shell keeps running but its output is not streamed.
    Detached,
    /// The shell process ended. Starting again makes a new shell.
    Exited,
    /// The hub refused or the connection failed; see `error`.
    Failed,
}

/// One agent's shell, as the window sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Terminal {
    pub agent: String,
    pub cwd: String,
    pub shell: Option<String>,
    pub status: Status,
    pub error: Option<String>,
    /// Bumped on every attach; the window starts a fresh emulator for each.
    pub attach: u64,
}

impl Terminal {
    pub fn running(&self) -> bool {
        matches!(
            self.status,
            Status::Starting | Status::Attaching | Status::Live | Status::Detached
        )
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    /// Show `agent`'s shell, starting one in `cwd` when it has none (or its
    /// shell ended).
    Open {
        agent: String,
        cwd: String,
        cols: u16,
        rows: u16,
    },
    /// Stop streaming output; the shell keeps running.
    Hide {
        agent: String,
    },
    /// Keyboard/paste input, delivered in order.
    Input {
        agent: String,
        bytes: Vec<u8>,
    },
    Resize {
        agent: String,
        cols: u16,
        rows: u16,
    },
    /// End the agent's shell (the hub kills it and its foreground job) and
    /// start a fresh one in `cwd`.
    Restart {
        agent: String,
        cwd: String,
        cols: u16,
        rows: u16,
    },
    /// Shells remembered from an earlier run of this client on this hub.
    Adopt(BTreeMap<String, String>),
}

/// Where this client remembers each agent's shell on one hub, so a restart
/// re-attaches to the same shell instead of starting another (and keeps
/// those shell sessions out of the session list).
pub fn remembered_path(settings: &std::path::Path, scope: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    settings
        .with_file_name("native-terminals")
        .join(format!("{:x}.json", Sha256::digest(scope.as_bytes())))
}

pub fn load_remembered(path: &std::path::Path) -> BTreeMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok())
        .map(|mut map| {
            map.retain(|agent, shell| !agent.is_empty() && !shell.is_empty());
            while map.len() > MAX_REMEMBERED {
                map.pop_first();
            }
            map
        })
        .unwrap_or_default()
}

pub fn save_remembered(
    path: &std::path::Path,
    map: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(map)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// The hub's default shell for new terminals: the shared config's
/// `terminal.shell`, which desktop's Settings → Terminal writes too. Empty
/// means the hub host's own default (`$SHELL`, else the platform's).
pub fn configured_shell(config: &serde_json::Value) -> String {
    config["terminal"]["shell"]
        .as_str()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// `terminals.create` parameters: the configured shell travels verbatim (the
/// hub checks it against the host's login shells); none means the default.
pub fn create_params(cwd: &str, cols: u16, rows: u16, shell: &str) -> serde_json::Value {
    let mut params = serde_json::json!({"cwd": cwd, "cols": cols, "rows": rows});
    if !shell.is_empty() {
        params["shell"] = shell.into();
    }
    params
}

/// What Settings shows for the default shell: the configured value and the
/// shells the hub's host can start (`terminals.shells`, `null` from a hub
/// that cannot list them).
pub fn shell_settings(
    config: &serde_json::Value,
    listed: anyhow::Result<serde_json::Value>,
) -> serde_json::Value {
    let (shells, default, error) = match listed {
        Ok(listed) if listed["shells"].is_array() => {
            (listed["shells"].clone(), listed["default"].clone(), None)
        }
        Ok(_) => (
            serde_json::Value::Null,
            serde_json::Value::Null,
            Some("the hub returned no shell list".to_owned()),
        ),
        Err(error) => (
            serde_json::Value::Null,
            serde_json::Value::Null,
            Some(error.to_string()),
        ),
    };
    serde_json::json!({
        "shell": configured_shell(config),
        "shells": shells,
        "default": default.as_str().unwrap_or_default(),
        "listError": error,
    })
}

/// One row of the default-shell picker; `path` is what gets saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellChoice {
    pub path: String,
    pub label: String,
    pub detail: String,
}

/// The picker's rows: the hub's default first, each shell installed on the
/// hub's host, and a configured shell the hub does not list (set on desktop
/// or another machine), shown as configured rather than silently replaced.
pub fn shell_choices(settings: &serde_json::Value) -> Vec<ShellChoice> {
    let default = settings["default"].as_str().unwrap_or_default();
    let mut choices: Vec<ShellChoice> = settings["shells"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let path = row["path"].as_str()?;
            let label = row["label"].as_str().filter(|l| !l.is_empty())?;
            Some(ShellChoice {
                path: path.to_owned(),
                label: label.to_owned(),
                detail: if path.is_empty() { default } else { path }.to_owned(),
            })
        })
        .collect();
    if !choices.iter().any(|c| c.path.is_empty()) {
        choices.insert(
            0,
            ShellChoice {
                path: String::new(),
                label: "Hub default".into(),
                detail: default.to_owned(),
            },
        );
    }
    let configured = settings["shell"].as_str().unwrap_or_default();
    if !configured.is_empty() && !choices.iter().any(|c| c.path == configured) {
        choices.push(ShellChoice {
            path: configured.to_owned(),
            label: "Configured".into(),
            detail: configured.to_owned(),
        });
    }
    choices
}

/// xterm's encoding of a key press, or `None` for keys the terminal leaves to
/// the application (platform-modified shortcuts, bare modifiers).
pub fn key_bytes(
    key: &str,
    key_char: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    if platform {
        return None;
    }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let csi = |final_byte: char, normal: bool| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if application_cursor && normal {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let esc = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let bytes = match key {
        "enter" => esc(vec![b'\r']),
        "tab" if shift => b"\x1b[Z".to_vec(),
        "tab" => esc(vec![b'\t']),
        "escape" => esc(vec![0x1b]),
        "backspace" if control => esc(vec![0x08]),
        "backspace" => esc(vec![0x7f]),
        "space" if control => vec![0],
        // Windows reports the space bar as a named key with no `key_char`.
        "space" => esc(vec![b' ']),
        "up" => csi('A', true),
        "down" => csi('B', true),
        "right" => csi('C', true),
        "left" => csi('D', true),
        "home" => csi('H', true),
        "end" => csi('F', true),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => csi('P', false).replace_ss3(modifier),
        "f2" => csi('Q', false).replace_ss3(modifier),
        "f3" => csi('R', false).replace_ss3(modifier),
        "f4" => csi('S', false).replace_ss3(modifier),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "shift" | "control" | "alt" | "platform" | "function" | "capslock" => return None,
        _ => {
            if control {
                let c = key.chars().next().filter(|_| key.chars().count() == 1)?;
                let byte = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 & 0x1f,
                    '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '?' | '8' => 0x7f,
                    _ => return None,
                };
                return Some(esc(vec![byte]));
            }
            let text = key_char
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| (key.chars().count() == 1).then(|| key.to_owned()))?;
            esc(text.into_bytes())
        }
    };
    Some(bytes)
}

trait Ss3 {
    fn replace_ss3(self, modifier: u8) -> Vec<u8>;
}

impl Ss3 for Vec<u8> {
    /// F1–F4 are SS3 sequences unmodified, CSI with a modifier.
    fn replace_ss3(self, modifier: u8) -> Vec<u8> {
        if modifier > 1 {
            self
        } else {
            let mut bytes = self;
            if bytes.len() == 3 && bytes[1] == b'[' {
                bytes[1] = b'O';
            }
            bytes
        }
    }
}

/// Text pasted into the shell: bracketed when the program asked for it, with
/// the bracket terminator removed from the payload so pasted text cannot end
/// the paste early and run as typed input.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let clean = text.replace("\x1b[201~", "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

/// One emulator per attach of an agent's shell.
pub struct Emulator {
    parser: vt100::Parser,
    /// Scrollback rows above the live screen being viewed (0 = live).
    pub scroll: usize,
}

impl Emulator {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK_LINES),
            scroll: 0,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        // New output while reading history keeps the reader's place unless
        // they are at the live screen.
        if self.scroll > 0 {
            self.parser.set_scrollback(self.scroll);
            self.scroll = self.parser.screen().scrollback();
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows.max(1), cols.max(1));
    }

    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    /// Move through scrollback by `lines` (positive = older). Returns the new
    /// offset.
    pub fn scroll_by(&mut self, lines: isize) -> usize {
        let wanted = (self.scroll as isize + lines).max(0) as usize;
        self.parser.set_scrollback(wanted);
        self.scroll = self.parser.screen().scrollback();
        self.scroll
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// The visible text, one line per row (for copying and tests).
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    /// Styled runs per visible row.
    pub fn rows(&self) -> Vec<Vec<Run>> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        (0..rows)
            .map(|row| {
                let mut runs: Vec<Run> = Vec::new();
                for col in 0..cols {
                    let Some(cell) = screen.cell(row, col) else {
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut text = cell.contents();
                    if text.is_empty() {
                        text.push(' ');
                    }
                    let style = Style {
                        fg: cell.fgcolor(),
                        bg: cell.bgcolor(),
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                    };
                    let width = if cell.is_wide() { 2 } else { 1 };
                    match runs.last_mut() {
                        Some(last) if last.style == style => {
                            last.text.push_str(&text);
                            last.cells += width;
                        }
                        _ => runs.push(Run {
                            text,
                            cells: width,
                            style,
                        }),
                    }
                }
                runs
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: vt100::Color,
    pub bg: vt100::Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    /// Columns covered (wide characters take two).
    pub cells: u16,
    pub style: Style,
}

/// A background this light wants the light ANSI set (relative luminance),
/// whatever the theme is called.
pub fn is_light(background: u32) -> bool {
    let channel = |shift: u32| {
        let c = ((background >> shift) & 0xff) as f32 / 255.;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0) > 0.4
}

/// The standard xterm 256-colour palette entry for an index ≥ 16; indices
/// below 16 are theme colours the caller supplies.
pub fn xterm_color(index: u8) -> u32 {
    match index {
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
            (level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6)
        }
        232..=255 => {
            let v = 8 + (index - 232) as u32 * 10;
            (v << 16) | (v << 8) | v
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str, char: Option<&str>) -> Vec<u8> {
        key_bytes(key, char, false, false, false, false, false).unwrap()
    }

    #[test]
    fn keys_use_xterm_sequences() {
        assert_eq!(key("a", Some("a")), b"a");
        assert_eq!(key("a", Some("A")), b"A");
        assert_eq!(key("enter", None), b"\r");
        assert_eq!(key("backspace", None), b"\x7f");
        assert_eq!(key("up", None), b"\x1b[A");
        assert_eq!(
            key_bytes("up", None, false, false, false, false, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes("right", None, true, false, false, false, true).unwrap(),
            b"\x1b[1;5C"
        );
        assert_eq!(
            key_bytes("c", Some("c"), true, false, false, false, false).unwrap(),
            [3]
        );
        assert_eq!(
            key_bytes("b", Some("b"), false, true, false, false, false).unwrap(),
            b"\x1bb"
        );
        assert_eq!(key("f1", None), b"\x1bOP");
        assert_eq!(key("f5", None), b"\x1b[15~");
        assert_eq!(key("delete", None), b"\x1b[3~");
        assert_eq!(
            key_bytes("tab", None, false, false, true, false, false).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(key("é", Some("é")), "é".as_bytes());
        // Linux sends the space bar with key_char " ", Windows with none.
        assert_eq!(key("space", Some(" ")), b" ");
        assert_eq!(key("space", None), b" ");
        assert_eq!(
            key_bytes("space", None, false, false, true, false, false).unwrap(),
            b" "
        );
        assert_eq!(
            key_bytes("space", None, false, true, false, false, false).unwrap(),
            b"\x1b "
        );
        assert_eq!(
            key_bytes("space", None, true, false, false, false, false).unwrap(),
            [0]
        );
        assert!(key_bytes("v", Some("v"), false, false, false, true, false).is_none());
        assert!(key_bytes("shift", None, false, false, true, false, false).is_none());
    }

    #[test]
    fn pastes_cannot_escape_bracketed_mode() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb");
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf\n", true),
            b"\x1b[200~xrm -rf\r\x1b[201~"
        );
    }

    #[test]
    fn feed_is_bounded_and_resets_before_replay() {
        let feed = Feed::default();
        assert!(feed.take("s").is_none());
        assert!(feed.push("s", b"one"));
        feed.reset("s");
        assert!(feed.push("s", b"two"));
        assert_eq!(
            feed.take("s"),
            Some(Chunk {
                reset: true,
                bytes: b"two".to_vec()
            })
        );
        assert!(feed.take("s").is_none());
        assert!(!feed.push("s", &vec![0; MAX_PENDING_BYTES + 1]));
        assert_eq!(
            feed.take("s"),
            Some(Chunk {
                reset: true,
                bytes: vec![]
            })
        );
    }

    #[test]
    fn emulator_renders_styled_runs_and_scrollback() {
        let mut term = Emulator::new(3, 10);
        term.process(b"\x1b[31mred\x1b[0m ok\r\n");
        let rows = term.rows();
        assert_eq!(rows[0][0].text, "red");
        assert_eq!(rows[0][0].style.fg, vt100::Color::Idx(1));
        assert!(rows[0][1].text.starts_with(" ok"));
        for n in 0..10 {
            term.process(format!("line{n}\r\n").as_bytes());
        }
        assert!(term.text().contains("line9"));
        assert!(term.scroll_by(3) > 0);
        assert!(!term.text().contains("line9"));
        term.scroll_by(-100);
        assert_eq!(term.scroll, 0);
        assert!(term.text().contains("line9"));
    }

    #[test]
    fn the_configured_shell_is_sent_to_terminals_create_only_when_set() {
        use serde_json::json;
        let zsh = json!({"terminal":{"shell":"/bin/zsh","shells":[]}});
        assert_eq!(configured_shell(&zsh), "/bin/zsh");
        for unset in [
            json!({}),
            json!({"terminal":{"shell":""}}),
            json!({"terminal":{"shell":"  "}}),
            json!({"terminal":{"shell":7}}),
        ] {
            assert_eq!(configured_shell(&unset), "", "{unset}");
        }
        assert_eq!(
            create_params("/work", 100, 24, "/bin/zsh"),
            json!({"cwd":"/work","cols":100,"rows":24,"shell":"/bin/zsh"})
        );
        // Windows paths travel verbatim; the hub checks them.
        let git = r"C:\Program Files\Git\bin\bash.exe";
        assert_eq!(create_params("C:\\w", 80, 20, git)["shell"], git);
        let default = create_params("/work", 100, 24, "");
        assert!(default.get("shell").is_none(), "{default}");
    }

    #[test]
    fn shell_choices_list_the_hubs_shells_and_keep_an_unlisted_configured_one() {
        use serde_json::json;
        let listed = json!({"platform":"windows","default":"powershell.exe","shells":[
            {"name":"default","path":"","label":"System default"},
            {"name":"powershell","path":"powershell.exe","label":"PowerShell"},
            {"name":"pwsh","path":"pwsh.exe","label":"PowerShell 7"},
            {"name":"cmd","path":"cmd.exe","label":"Command Prompt"},
            {"name":"wsl","path":"wsl.exe","label":"WSL"},
            {"name":"gitbash","path":"C:\\Program Files\\Git\\bin\\bash.exe","label":"Git Bash"},
        ]});
        let settings = shell_settings(&json!({"terminal":{"shell":"wsl.exe"}}), Ok(listed.clone()));
        assert_eq!(settings["shell"], "wsl.exe");
        assert!(settings["listError"].is_null());
        let choices = shell_choices(&settings);
        let labels: Vec<_> = choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "System default",
                "PowerShell",
                "PowerShell 7",
                "Command Prompt",
                "WSL",
                "Git Bash"
            ]
        );
        assert_eq!(
            choices[0].detail, "powershell.exe",
            "the default names what it runs"
        );
        assert_eq!(choices[5].path, "C:\\Program Files\\Git\\bin\\bash.exe");
        // Set elsewhere and not offered by this hub: shown, not swapped.
        let custom = shell_settings(&json!({"terminal":{"shell":"/opt/nu/bin/nu"}}), Ok(listed));
        let last = shell_choices(&custom).pop().unwrap();
        assert_eq!(
            last,
            ShellChoice {
                path: "/opt/nu/bin/nu".into(),
                label: "Configured".into(),
                detail: "/opt/nu/bin/nu".into()
            }
        );
        // A hub that cannot list its shells still offers its default and
        // the configured one, and says why.
        let old = shell_settings(
            &json!({"terminal":{"shell":"/bin/zsh"}}),
            Err(anyhow::anyhow!("unknown method terminals.shells")),
        );
        assert_eq!(old["listError"], "unknown method terminals.shells");
        let paths: Vec<_> = shell_choices(&old).into_iter().map(|c| c.path).collect();
        assert_eq!(paths, ["", "/bin/zsh"]);
    }

    #[test]
    fn remembered_shells_round_trip_per_hub() {
        let dir = std::env::temp_dir().join(format!("wks-terminals-{}", std::process::id()));
        let settings = dir.join("native-settings.json");
        let a = remembered_path(&settings, "hub-a");
        assert_ne!(a, remembered_path(&settings, "hub-b"));
        assert!(load_remembered(&a).is_empty());
        let map = BTreeMap::from([("agent".to_owned(), "shell".to_owned())]);
        save_remembered(&a, &map).unwrap();
        assert_eq!(load_remembered(&a), map);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn light_backgrounds_are_classified_by_luminance() {
        assert!(is_light(0xffffff));
        assert!(is_light(0xeff1f5));
        assert!(!is_light(0x1e1e2e));
        assert!(!is_light(0x2e3440));
    }

    #[test]
    fn xterm_cube_and_greys() {
        assert_eq!(xterm_color(16), 0);
        assert_eq!(xterm_color(231), 0xffffff);
        assert_eq!(xterm_color(232), 0x080808);
    }
}
