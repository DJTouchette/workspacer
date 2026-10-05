//! Native self-update: follow the channel the build came from (a nightly
//! install tracks the rolling `nightly` prerelease, a stable one the latest
//! release), compare versions, and on Windows download the per-user NSIS
//! installer, verify it, and hand off to a helper that installs after this
//! process exits and relaunches it. The helper confirms it is running before
//! the app quits, checks the installer's exit code and installed version, and
//! records each step in `state_dir()` for the next launch to report.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::Digest;
use std::{cmp::Ordering, path::PathBuf};

const REPO_API: &str = "https://api.github.com/repos/DJTouchette/workspacer/releases";
const INSTALLER_PREFIX: &str = "Workspacer-Native-Rust-Preview-Setup-";
const INSTALLER_SUFFIX: &str = "-x64.exe";
/// Installers are ~30 MB; refuse anything absurd before writing it to disk.
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    pub fn of(version: &str) -> Self {
        if version.contains("-nightly") {
            Self::Nightly
        } else {
            Self::Stable
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
    fn release_url(self) -> String {
        match self {
            Self::Stable => format!("{REPO_API}/latest"),
            Self::Nightly => format!("{REPO_API}/tags/nightly"),
        }
    }
}

/// Semver precedence: numeric core, then a release outranks its prereleases,
/// then prerelease identifiers left to right (numeric ones numerically).
/// `None` when either side is not a version (a development build).
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    fn parse(v: &str) -> Option<(Vec<u64>, Vec<&str>)> {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next()?;
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let core: Vec<u64> = core
            .split('.')
            .map(|n| n.parse().ok())
            .collect::<Option<_>>()?;
        (core.len() == 3).then_some(())?;
        let pre = if pre.is_empty() {
            Vec::new()
        } else {
            pre.split('.').collect()
        };
        Some((core, pre))
    }
    let (ac, ap) = parse(a)?;
    let (bc, bp) = parse(b)?;
    Some(
        ac.cmp(&bc)
            .then_with(|| match (ap.is_empty(), bp.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (x, y) in ap.iter().zip(&bp) {
                        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => x.cmp(y),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    ap.len().cmp(&bp.len())
                }
            }),
    )
}

/// The Windows native installer in a GitHub release, with its version parsed
/// from the file name (the rolling nightly tag carries no version).
pub fn installer_asset(release: &Value) -> Option<Value> {
    release["assets"].as_array()?.iter().find_map(|asset| {
        let name = asset["name"].as_str()?;
        let version = name
            .strip_prefix(INSTALLER_PREFIX)?
            .strip_suffix(INSTALLER_SUFFIX)?;
        Some(json!({
            "name": name,
            "version": version,
            "url": asset["browser_download_url"],
            "size": asset["size"],
            // GitHub publishes "sha256:<hex>" per asset.
            "sha256": asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")),
        }))
    })
}

/// What the Settings → About card shows.
pub fn check_result(installed: &str, release: &Value) -> Value {
    let channel = Channel::of(installed);
    let asset = installer_asset(release);
    let latest = asset
        .as_ref()
        .and_then(|a| a["version"].as_str().map(str::to_owned))
        .or_else(|| {
            release["tag_name"]
                .as_str()
                .filter(|t| *t != "nightly")
                .map(|t| t.trim_start_matches('v').to_owned())
        });
    let newer = latest
        .as_deref()
        .and_then(|latest| compare(latest, installed))
        .map(|o| o == Ordering::Greater);
    json!({
        "channel": channel.label(),
        "installed": installed,
        "latest": latest,
        // null: the installed build is not a release version (development).
        "update_available": newer,
        "installable": cfg!(target_os = "windows") && asset.is_some(),
        "asset": asset,
        "release_url": release["html_url"],
    })
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("Workspacer-Native")
        .timeout(std::time::Duration::from_secs(600))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()?)
}

pub async fn check(installed: &str) -> Result<Value> {
    let response = client()?
        .get(Channel::of(installed).release_url())
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        response.content_length().unwrap_or(0) <= 4 * 1024 * 1024,
        "Release response too large"
    );
    Ok(check_result(installed, &response.json().await?))
}

/// Download the installer into a fresh temp directory, enforcing the
/// published size and, when GitHub provides it, the SHA-256 digest.
pub async fn download(asset: &Value) -> Result<PathBuf> {
    let name = asset["name"].as_str().context("Installer has no name")?;
    ensure!(
        name.starts_with(INSTALLER_PREFIX)
            && name.ends_with(INSTALLER_SUFFIX)
            && !name.contains(['/', '\\']),
        "Unexpected installer name"
    );
    let url = asset["url"].as_str().context("Installer has no URL")?;
    ensure!(
        url.starts_with("https://github.com/DJTouchette/workspacer/releases/download/"),
        "Installer is not hosted on the Workspacer release page"
    );
    let size = asset["size"].as_u64().context("Installer has no size")?;
    ensure!(
        size > 0 && size <= MAX_INSTALLER_BYTES,
        "Installer size is out of range"
    );
    let dir = std::env::temp_dir().join(format!(
        "workspacer-native-update-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    let mut response = client()?.get(url).send().await?.error_for_status()?;
    let mut file = std::fs::File::create(&path)?;
    let mut hasher = sha2::Sha256::new();
    let mut written = 0u64;
    while let Some(chunk) = response.chunk().await? {
        written += chunk.len() as u64;
        if written > size {
            bail!("Installer is larger than published");
        }
        hasher.update(&chunk);
        std::io::Write::write_all(&mut file, &chunk)?;
    }
    drop(file);
    ensure!(written == size, "Installer download was incomplete");
    if let Some(expected) = asset["sha256"].as_str() {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        ensure!(
            actual.eq_ignore_ascii_case(expected),
            "Installer checksum mismatch"
        );
    }
    Ok(path)
}

/// Where the helper records its progress and log: outside the install folder
/// (which it replaces) and the temp download (which it deletes), so the next
/// launch can report what happened.
pub fn state_dir() -> Result<PathBuf> {
    let base = directories::BaseDirs::new().context("Cannot locate the local data directory")?;
    Ok(base
        .data_local_dir()
        .join("Workspacer Native Rust Preview")
        .join("updates"))
}

const STATE_FILE: &str = "last-update.json";
const SEEN_FILE: &str = "last-update.seen.json";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const PLAN_FILE: &str = "last-update-plan.json";
const LOG_FILE: &str = "last-update.log";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const OUTPUT_FILE: &str = "last-update-output.log";
/// The helper script. Constant text: paths and arguments travel in the plan.
pub const HELPER_SCRIPT: &str = include_str!("update_helper.ps1");

/// How long each helper step may take.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// The helper must report that it is waiting before the app quits.
    pub ready: std::time::Duration,
    /// The app closing, including an unsaved-edits question.
    pub app_exit: std::time::Duration,
    /// Other processes running from the install folder.
    pub siblings: std::time::Duration,
    pub install: std::time::Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        use std::time::Duration;
        Self {
            ready: Duration::from_secs(30),
            app_exit: Duration::from_secs(15 * 60),
            siblings: Duration::from_secs(60),
            install: Duration::from_secs(10 * 60),
        }
    }
}

/// Everything the helper needs to replace the running app and start it again.
#[derive(Clone, Debug)]
pub struct Handoff {
    /// The process the helper waits for.
    pub pid: u32,
    pub installer: PathBuf,
    /// The version the installer must leave in `build-stamp.json`.
    pub expected_version: String,
    /// The executable to relaunch; its folder is the install folder.
    pub exe: PathBuf,
    /// Relaunch arguments, exactly as this process received them.
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub state_dir: PathBuf,
    pub powershell: PathBuf,
    pub timeouts: Timeouts,
}

impl Handoff {
    /// Replace this process: its executable, arguments and working directory.
    pub fn current(installer: PathBuf, expected_version: &str) -> Result<Self> {
        let args = std::env::args_os()
            .skip(1)
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| anyhow::anyhow!("A launch argument is not valid Unicode"))
            })
            .collect::<Result<Vec<_>>>()?;
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Ok(Self {
            pid: std::process::id(),
            installer,
            expected_version: expected_version.to_owned(),
            exe: std::env::current_exe()?,
            args,
            cwd: std::env::current_dir()?,
            state_dir: state_dir()?,
            // Never resolved through PATH or the app's own folder.
            powershell: PathBuf::from(system_root)
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            timeouts: Timeouts::default(),
        })
    }

    pub fn install_dir(&self) -> Result<PathBuf> {
        Ok(self
            .exe
            .parent()
            .context("The app has no install folder")?
            .to_path_buf())
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join(STATE_FILE)
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join(LOG_FILE)
    }

    /// The JSON plan the helper script reads.
    pub fn plan(&self, nonce: &str) -> Result<Value> {
        ensure!(
            compare(&self.expected_version, &self.expected_version).is_some(),
            "The update has no valid version"
        );
        let text = |path: &std::path::Path| {
            path.to_str()
                .map(str::to_owned)
                .context("A path is not valid Unicode")
        };
        let millis = |d: std::time::Duration| d.as_millis().min(i32::MAX as u128) as u64;
        Ok(json!({
            "nonce": nonce,
            "parentId": self.pid,
            "installer": text(&self.installer)?,
            "exe": text(&self.exe)?,
            "installDir": text(&self.install_dir()?)?,
            "arguments": windows_command_line(&self.args),
            "cwd": text(&self.cwd)?,
            "expected": self.expected_version,
            "state": text(&self.state_file())?,
            "log": text(&self.log_file())?,
            "appExitMs": millis(self.timeouts.app_exit),
            "siblingMs": millis(self.timeouts.siblings),
            "installMs": millis(self.timeouts.install),
        }))
    }
}

/// Join arguments into a Windows command line that `CommandLineToArgvW` and
/// the Microsoft C runtime split back into exactly the same arguments.
pub fn windows_command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
                return arg.clone();
            }
            let mut quoted = String::from('"');
            let mut backslashes = 0;
            for c in arg.chars() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                        quoted.push('"');
                        backslashes = 0;
                    }
                    c => {
                        quoted.extend(std::iter::repeat_n('\\', backslashes));
                        quoted.push(c);
                        backslashes = 0;
                    }
                }
            }
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The helper's last recorded state, if it belongs to this hand-off.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn read_state(path: &std::path::Path, nonce: &str) -> Option<Value> {
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (value["nonce"] == nonce).then_some(value)
}

/// The cached, verified download was removed (for example by temp cleanup).
/// The UI may discard this cache entry and offer a fresh download.
#[derive(Debug)]
pub struct MissingInstaller;
impl std::fmt::Display for MissingInstaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The downloaded installer is no longer available. Choose Install and restart to download it again.")
    }
}
impl std::error::Error for MissingInstaller {}

/// A successful handoff retains a live process/readiness probe. Dropping it
/// must not terminate the detached helper when the app exits.
pub struct ReadyHelper {
    probe: Box<dyn FnMut() -> Result<bool> + Send>,
}
impl ReadyHelper {
    pub fn is_waiting(&mut self) -> Result<bool> {
        (self.probe)()
    }

    #[cfg(feature = "ui-tests")]
    pub fn for_test(probe: impl FnMut() -> Result<bool> + Send + 'static) -> Self {
        Self {
            probe: Box::new(probe),
        }
    }
}

/// Until readiness is committed, every error must stop and reap the helper.
/// std::process::Child alone detaches on drop and would leave an unacknowledged
/// update armed for a later ordinary app exit.
#[cfg(windows)]
struct StartingHelper(Option<std::process::Child>);
#[cfg(windows)]
impl std::ops::Deref for StartingHelper {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}
#[cfg(windows)]
impl std::ops::DerefMut for StartingHelper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}
#[cfg(windows)]
impl Drop for StartingHelper {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Start the hidden helper and wait until it holds a handle to this process.
/// Only then may the caller quit; any error means the helper is not running
/// and the app must stay open.
pub fn hand_off(handoff: &Handoff) -> Result<ReadyHelper> {
    start_helper(handoff).inspect_err(|error| {
        #[cfg(windows)]
        {
            use std::io::Write;
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(handoff.log_file())
            {
                let _ = writeln!(
                    log,
                    "{} app {} handoff refused: {error:#}",
                    chrono::Utc::now().to_rfc3339(),
                    handoff.pid
                );
            }
        }
        #[cfg(not(windows))]
        let _ = error;
    })
}

fn start_helper(handoff: &Handoff) -> Result<ReadyHelper> {
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        // A hidden console of its own. DETACHED_PROCESS would make Windows
        // ignore CREATE_NO_WINDOW and leave PowerShell with no console.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        // Leave any job the app runs in, which may end its members with it.
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const ERROR_ACCESS_DENIED: i32 = 5;

        if !handoff.installer.is_file() {
            return Err(MissingInstaller.into());
        }
        ensure!(
            handoff.exe.is_file(),
            "The installed application is unavailable; update manually."
        );
        ensure!(
            handoff.cwd.is_dir(),
            "The original working directory is unavailable; restart from an existing directory before updating."
        );
        std::fs::create_dir_all(&handoff.state_dir)
            .context("Could not create the update state folder")?;
        let nonce = format!(
            "{}-{}",
            handoff.pid,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let plan = handoff.plan(&nonce)?;
        let plan_file = handoff.state_dir.join(format!("{nonce}-{PLAN_FILE}"));
        std::fs::write(&plan_file, serde_json::to_vec_pretty(&plan)?)
            .context("Could not write the update plan")?;
        let state_file = handoff.state_file();
        // Another accepted helper may own the shared outcome. Its mutex and
        // nonce protect the receipt; a new attempt must not erase its state.
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(handoff.log_file())
            .context("Could not create the update log")?;
        writeln!(
            log,
            "{} app {} handing off {} (version {}) for {}",
            chrono::Utc::now().to_rfc3339(),
            handoff.pid,
            handoff.installer.display(),
            handoff.expected_version,
            handoff.exe.display()
        )?;
        drop(log);
        let output_path = handoff.state_dir.join(format!("{nonce}-{OUTPUT_FILE}"));
        let output = std::fs::File::create(&output_path)?;
        let spawn = |flags: u32| -> std::io::Result<std::process::Child> {
            std::process::Command::new(&handoff.powershell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    HELPER_SCRIPT,
                ])
                .env("WKS_UPDATE_PLAN", &plan_file)
                .current_dir(&handoff.state_dir)
                .stdin(std::process::Stdio::null())
                .stdout(output.try_clone()?)
                .stderr(output.try_clone()?)
                .creation_flags(flags)
                .spawn()
        };
        let base = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
        let child = match spawn(base | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(child) => child,
            // A helper still owned by the launcher's job may be killed with
            // this app. Never report ready and reproduce the close-only bug.
            Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                bail!(
                    "Windows prevented the update helper from running independently (launcher or policy restriction). Keep Workspacer open, or close it and run {} manually.",
                    handoff.installer.display()
                );
            }
            Err(error) => return Err(error).context("Could not start the installer helper"),
        };
        let mut child = StartingHelper(Some(child));
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(handoff.log_file())?;
        writeln!(log, "helper {} started; job breakaway: true", child.id())?;
        drop(log);
        let detail = |state: Option<Value>| {
            state
                .and_then(|s| s["detail"].as_str().map(str::to_owned))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| format!("see {}", output_path.display()))
        };
        let deadline = std::time::Instant::now() + handoff.timeouts.ready;
        loop {
            let state = read_state(&state_file, &nonce);
            match state.as_ref().and_then(|s| s["state"].as_str()) {
                Some("waiting") => {
                    // Leave ample time for joined backend shutdown. Once this
                    // expires, the UI must not start a competing helper.
                    let reserve = std::time::Duration::from_secs(120);
                    let safe_wait = handoff
                        .timeouts
                        .app_exit
                        .checked_sub(reserve)
                        .filter(|duration| !duration.is_zero())
                        .unwrap_or(handoff.timeouts.app_exit / 2);
                    let expires = std::time::Instant::now() + safe_wait;
                    let mut process = child.0.take().unwrap();
                    return Ok(ReadyHelper {
                        probe: Box::new(move || {
                            Ok(std::time::Instant::now() < expires
                                && process.try_wait()?.is_none()
                                && read_state(&state_file, &nonce)
                                    .is_some_and(|s| s["state"] == "waiting"))
                        }),
                    });
                }
                Some("failed") => bail!("The installer helper failed: {}", detail(state)),
                _ => {}
            }
            if let Some(status) = child.try_wait()? {
                bail!(
                    "The installer helper exited ({status}) before it was ready: {}",
                    detail(read_state(&state_file, &nonce))
                );
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "The installer helper did not start within {} seconds",
                    handoff.timeouts.ready.as_secs()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = handoff;
        bail!(
            "In-app install is available on Windows; download this platform's build from the release page"
        )
    }
}

/// What the last update did, once: a finished update (or one whose helper
/// stopped without finishing) is reported and then marked seen.
pub fn take_outcome(state_dir: &std::path::Path) -> Option<Value> {
    let path = state_dir.join(STATE_FILE);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let age = chrono::DateTime::parse_from_rfc3339(value["time"].as_str().unwrap_or_default())
        .map(|time| chrono::Utc::now().signed_duration_since(time))
        .unwrap_or(chrono::TimeDelta::MAX);
    let stale = |limit: Timeouts| match value["state"].as_str() {
        Some("waiting") => {
            age > chrono::TimeDelta::from_std(limit.app_exit + limit.ready)
                .unwrap_or(chrono::TimeDelta::MAX)
        }
        Some("installing") => {
            age > chrono::TimeDelta::from_std(limit.install + limit.siblings)
                .unwrap_or(chrono::TimeDelta::MAX)
        }
        _ => false,
    };
    match value["state"].as_str() {
        Some("succeeded" | "failed") => {}
        _ if stale(Timeouts::default()) => {
            let step = value["state"].as_str().unwrap_or("starting").to_owned();
            value["state"] = json!("failed");
            value["detail"] = json!(format!(
                "The update helper stopped while {step} and did not finish."
            ));
        }
        // Still in progress (or unreadable): leave it for the helper.
        _ => return None,
    }
    let _ = std::fs::rename(&path, state_dir.join(SEEN_FILE));
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_like_semver_including_nightly_stamps() {
        let gt = |a, b| compare(a, b) == Some(Ordering::Greater);
        assert!(gt(
            "0.169.0-nightly.202610021714",
            "0.169.0-nightly.202610021432"
        ));
        assert!(gt(
            "0.170.0-nightly.202610030800",
            "0.169.0-nightly.202610021714"
        ));
        assert!(
            gt("0.169.0", "0.169.0-nightly.202610021714"),
            "a release outranks its nightlies"
        );
        assert!(gt("v0.169.1", "0.169.0"));
        assert!(gt("0.169.10", "0.169.9"), "numeric, not lexical");
        assert_eq!(compare("0.169.0", "v0.169.0"), Some(Ordering::Equal));
        assert_eq!(compare("0.169.0 (development build)", "0.169.0"), None);
        assert_eq!(
            Channel::of("0.169.0-nightly.202610021714"),
            Channel::Nightly
        );
        assert_eq!(Channel::of("0.169.0"), Channel::Stable);
    }

    #[test]
    fn nightly_check_reads_the_installer_name_and_digest() {
        let release = json!({
            "tag_name": "nightly",
            "html_url": "https://github.com/DJTouchette/workspacer/releases/tag/nightly",
            "assets": [
                {"name":"Workspacer-Setup-0.169.0-nightly.202610031714.exe","size":1},
                {"name":"Workspacer-Native-Rust-Preview-Setup-0.169.0-nightly.202610031714-x64.exe",
                 "size": 28427608, "digest": "sha256:abc123",
                 "browser_download_url":"https://github.com/DJTouchette/workspacer/releases/download/nightly/x.exe"}
            ]
        });
        let result = check_result("0.169.0-nightly.202610021714", &release);
        assert_eq!(result["channel"], "nightly");
        assert_eq!(result["latest"], "0.169.0-nightly.202610031714");
        assert_eq!(result["update_available"], true);
        assert_eq!(result["asset"]["sha256"], "abc123");
        assert_eq!(result["asset"]["size"], 28427608);
        let same = check_result("0.169.0-nightly.202610031714", &release);
        assert_eq!(same["update_available"], false);
        // A stable release without a native installer still reports its tag.
        let stable = check_result("0.168.0", &json!({"tag_name":"v0.169.0","assets":[]}));
        assert_eq!(stable["latest"], "0.169.0");
        assert_eq!(stable["update_available"], true);
        assert_eq!(stable["installable"], false);
        let dev = check_result("0.169.0 (development build)", &release);
        assert!(dev["update_available"].is_null());
    }

    /// The argument splitting the C runtime and `CommandLineToArgvW` apply.
    fn split_windows(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut chars = line.chars().peekable();
        loop {
            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                chars.next();
            }
            if chars.peek().is_none() {
                return args;
            }
            let (mut arg, mut quoted) = (String::new(), false);
            while let Some(&c) = chars.peek() {
                if !quoted && (c == ' ' || c == '\t') {
                    break;
                }
                chars.next();
                match c {
                    '\\' => {
                        let mut count = 1;
                        while chars.peek() == Some(&'\\') {
                            chars.next();
                            count += 1;
                        }
                        if chars.peek() == Some(&'"') {
                            arg.extend(std::iter::repeat_n('\\', count / 2));
                            if count % 2 == 1 {
                                chars.next();
                                arg.push('"');
                            }
                        } else {
                            arg.extend(std::iter::repeat_n('\\', count));
                        }
                    }
                    '"' if quoted && chars.peek() == Some(&'"') => {
                        chars.next();
                        arg.push('"');
                    }
                    '"' => quoted = !quoted,
                    c => arg.push(c),
                }
            }
            args.push(arg);
        }
    }

    #[test]
    fn relaunch_command_line_round_trips_every_argument() {
        let cases: Vec<Vec<String>> = vec![
            vec![],
            vec!["--local".into()],
            vec![
                "--rust-local-dir".into(),
                r"C:\Users\o'neil\Work Dir\".into(),
                r#"say "hi""#.into(),
                String::new(),
                r#"\\server\share\"#.into(),
                r#"a\\"b"#.into(),
                "tab\there".into(),
                "--bus=ws://127.0.0.1:7895/bus".into(),
            ],
        ];
        for args in cases {
            let line = windows_command_line(&args);
            assert_eq!(split_windows(&line), args, "{line}");
        }
        assert_eq!(windows_command_line(&["--local".into()]), "--local");
    }

    #[test]
    fn helper_plan_carries_every_value_and_the_script_carries_none() {
        let handoff = Handoff {
            pid: 42,
            installer: r"C:\Users\o’neil\Temp\Workspacer-Native-Rust-Preview-Setup-1.2.3-x64.exe".into(),
            expected_version: "1.2.3".into(),
            exe: r"C:\Users\o’neil\AppData\Local\Programs\Workspacer Native Rust Preview\wks-native.exe"
                .into(),
            args: vec!["--local".into(), r#"it's "x""#.into()],
            cwd: r"C:\Users\o’neil".into(),
            state_dir: r"C:\Users\o’neil\AppData\Local\Workspacer Native Rust Preview\updates".into(),
            powershell: r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into(),
            timeouts: Timeouts::default(),
        };
        let plan = handoff.plan("n1").unwrap();
        // Path splitting follows the build host; values are what was given.
        assert_eq!(plan["parentId"], 42);
        assert_eq!(plan["expected"], "1.2.3");
        assert_eq!(plan["arguments"], r#"--local "it's \"x\"""#);
        assert_eq!(plan["installer"], handoff.installer.to_str().unwrap());
        assert_eq!(plan["appExitMs"], 15 * 60 * 1000);
        assert!(
            plan["state"]
                .as_str()
                .unwrap()
                .ends_with("last-update.json")
        );
        // Nothing user-controlled is parsed as PowerShell, and the script
        // survives Windows command-line quoting unchanged.
        assert!(!HELPER_SCRIPT.contains('"'));
        assert!(HELPER_SCRIPT.contains("$env:WKS_UPDATE_PLAN"));
        assert!(HELPER_SCRIPT.contains("'/S /D=' + $plan.installDir"));
        let bad = Handoff {
            expected_version: "latest".into(),
            ..handoff
        };
        assert!(bad.plan("n1").is_err(), "a non-version is never expected");
    }

    #[test]
    fn last_update_is_reported_once_and_a_stopped_helper_is_explained() {
        let dir = std::env::temp_dir().join(format!(
            "wks-update-outcome-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |state: &str, age: i64| {
            let time = chrono::Utc::now() - chrono::TimeDelta::seconds(age);
            std::fs::write(
                dir.join(STATE_FILE),
                json!({"state": state, "detail": "", "expected": "1.2.3", "time": time.to_rfc3339()})
                    .to_string(),
            )
            .unwrap();
        };
        assert!(take_outcome(&dir).is_none(), "nothing recorded");
        write("succeeded", 1);
        assert_eq!(take_outcome(&dir).unwrap()["state"], "succeeded");
        assert!(take_outcome(&dir).is_none(), "reported once");
        write("waiting", 5);
        assert!(
            take_outcome(&dir).is_none(),
            "a helper may still be waiting"
        );
        write("waiting", 600);
        assert!(
            take_outcome(&dir).is_none(),
            "a second app must not consume a live 15-minute wait"
        );
        write("waiting", 1200);
        let stopped = take_outcome(&dir).unwrap();
        assert_eq!(stopped["state"], "failed");
        assert!(
            stopped["detail"]
                .as_str()
                .unwrap()
                .contains("stopped while waiting")
        );
        write("installing", 120);
        assert!(take_outcome(&dir).is_none(), "the installer may still run");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn other_platforms_never_start_a_helper() {
        let handoff = Handoff {
            pid: 1,
            installer: "/nonexistent/setup.exe".into(),
            expected_version: "1.2.3".into(),
            exe: "/nonexistent/wks-native".into(),
            args: vec![],
            cwd: "/".into(),
            state_dir: "/nonexistent/updates".into(),
            powershell: "/nonexistent/powershell".into(),
            timeouts: Timeouts::default(),
        };
        let error = hand_off(&handoff).err().unwrap().to_string();
        assert!(error.contains("available on Windows"));
        assert!(!std::path::Path::new("/nonexistent/updates").exists());
    }
}
