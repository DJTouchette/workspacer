//! Native self-update: follow the channel the build came from (a nightly
//! install tracks the rolling `nightly` prerelease, a stable one the latest
//! release), compare versions, and on Windows download the per-user NSIS
//! installer, verify it, and hand off to a helper that installs after this
//! process exits and relaunches it.
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

/// PowerShell single-quoted literal.
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The helper script: wait for this app (and anything else running from its
/// install directory, such as the local backend) to exit, install silently,
/// then relaunch with the same arguments.
pub fn handoff_script(pid: u32, installer: &str, exe: &str, args: &[String]) -> String {
    // String split, so the script is identical whichever OS builds it.
    let dir = exe.rsplit_once(['\\', '/']).map_or("", |(dir, _)| dir);
    let args = args
        .iter()
        .map(|a| ps_quote(a))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "$ErrorActionPreference='SilentlyContinue'; \
         Wait-Process -Id {pid} -Timeout 60; \
         $dir={dir}; \
         Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($dir + '\\') }} | Wait-Process -Timeout 30; \
         Start-Process -FilePath {installer} -ArgumentList '/S' -Wait; \
         Start-Process -FilePath {exe}{relaunch}",
        dir = ps_quote(dir),
        installer = ps_quote(installer),
        exe = ps_quote(exe),
        relaunch = if args.is_empty() {
            String::new()
        } else {
            format!(" -ArgumentList @({args})")
        },
    )
}

/// Start the detached helper. The caller quits right after.
pub fn hand_off(installer: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let exe = std::env::current_exe()?;
        let args: Vec<String> = std::env::args().skip(1).collect();
        let script = handoff_script(
            std::process::id(),
            &installer.to_string_lossy(),
            &exe.to_string_lossy(),
            &args,
        );
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-Command",
                script.as_str(),
            ])
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .context("Could not start the installer helper")?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = installer;
        bail!(
            "In-app install is available on Windows; download this platform's build from the release page"
        )
    }
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

    #[test]
    fn handoff_waits_installs_silently_and_relaunches_with_quoted_paths() {
        let script = handoff_script(
            42,
            r"C:\Users\o'neil\AppData\Local\Temp\setup.exe",
            r"C:\Users\o'neil\AppData\Local\Programs\Workspacer\wks-native.exe",
            &["--local".into()],
        );
        assert!(script.contains("Wait-Process -Id 42"));
        assert!(
            script.contains(
                r"'C:\Users\o''neil\AppData\Local\Temp\setup.exe' -ArgumentList '/S' -Wait"
            )
        );
        assert!(script.contains(r"$dir='C:\Users\o''neil\AppData\Local\Programs\Workspacer'"));
        assert!(script.ends_with("-ArgumentList @('--local')"));
    }
}
