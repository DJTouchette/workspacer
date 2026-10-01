//! Offline trusted plugin installation. No shell interpolation; reject links and
//! host-owned marker files from the source. The caller must stop old sidecars
//! before replacing their directory.
use super::manifest::Manifest;
use anyhow::{Result, bail};
use claudemon::background_process::BackgroundCommand;
use claudemon::child_env::SanitizeChildEnvironment;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const MARKERS: &[&str] = &[
    ".bus-token",
    ".settings.json",
    ".disabled",
    ".install-source",
];
fn copy_tree(source: &Path, dest: &Path, budget: &mut u64) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if MARKERS.iter().any(|m| name == *m) {
            continue;
        }
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            bail!("plugin install source must not contain symlinks")
        }
        if ty.is_dir() {
            copy_tree(&entry.path(), &dest.join(name), budget)?;
        } else if ty.is_file() {
            let size = entry.metadata()?.len();
            *budget = budget
                .checked_sub(size)
                .ok_or_else(|| anyhow::anyhow!("plugin exceeds 256 MiB"))?;
            std::fs::copy(entry.path(), dest.join(name))?;
        } else {
            bail!("plugin contains unsupported file type")
        }
    }
    Ok(())
}
pub fn install_local(source: &Path, root: &Path) -> Result<Manifest> {
    let source = source.canonicalize()?;
    let original = Manifest::load(&source.join("plugin.json"))?;
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    if root.starts_with(&source) {
        bail!("install destination cannot be inside source")
    }
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&root)?;
    copy_tree(&source, staging.path(), &mut (256 * 1024 * 1024))?;
    let staged = Manifest::load(&staging.path().join("plugin.json"))?;
    Consent::default().check(&staged)?;
    if let Some((command, args)) = staged.install.split_first() {
        let status = Command::new(command)
            .args(args)
            .current_dir(staging.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .no_console_window()
            .status()?;
        if !status.success() {
            bail!("plugin install command failed: {status}")
        }
    }
    // Installation commands are trusted code; validate their resulting manifest.
    let staged = Manifest::load(&staging.path().join("plugin.json"))?;
    if staged.id != original.id {
        bail!("install command changed plugin identity")
    }
    let destination = root.join(&staged.id);
    if destination.exists() {
        bail!("plugin already installed; stop and uninstall before replacing")
    }
    std::fs::write(
        staging.path().join(".install-source"),
        source.to_string_lossy().as_bytes(),
    )?;
    std::fs::rename(staging.path(), &destination)?;
    Manifest::load(&destination.join("plugin.json"))
}
pub fn uninstall_directory(root: &Path, directory: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let directory = directory.canonicalize()?;
    if directory.parent() != Some(root.as_path()) {
        bail!("refusing to uninstall outside plugin root")
    }
    let trash: PathBuf = root.join(format!(".trash-{}", uuid::Uuid::new_v4()));
    std::fs::rename(directory, &trash)?;
    std::fs::remove_dir_all(trash)?;
    Ok(())
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Consent {
    pub allow_install_command: bool,
    pub consented_argv: Vec<String>,
}
#[derive(Debug)]
pub struct ConsentRequired {
    pub plugin_id: String,
    pub argv: Vec<String>,
}
impl std::fmt::Display for ConsentRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "plugin install command requires matching explicit consent"
        )
    }
}
impl std::error::Error for ConsentRequired {}
impl Consent {
    fn check(&self, m: &Manifest) -> Result<()> {
        if !m.install.is_empty()
            && (!self.allow_install_command || self.consented_argv != m.install)
        {
            return Err(ConsentRequired {
                plugin_id: m.id.clone(),
                argv: m.install.clone(),
            }
            .into());
        }
        Ok(())
    }
}
pub fn platform(raw: &str) -> String {
    raw.replace(
        "${os}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        },
    )
    .replace(
        "${arch}",
        match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            x => x,
        },
    )
    .replace("${exe}", if cfg!(windows) { ".exe" } else { "" })
}
/// Pin only a bare Node runtime; package managers and explicit paths retain
/// their declared executable. The same decision governs builds and sidecars.
pub fn runtime_command(
    command: &str,
    runtime: Option<&str>,
) -> (String, std::collections::BTreeMap<String, String>) {
    let command = platform(command);
    match runtime.filter(|v| !v.is_empty()) {
        Some(runtime) if matches!(command.as_str(), "node" | "node.exe") => (
            runtime.into(),
            [("ELECTRON_RUN_AS_NODE".into(), "1".into())].into(),
        ),
        _ => (command, Default::default()),
    }
}

pub fn archive_urls(input: &str) -> Result<Vec<String>> {
    let input = input.trim();
    if input.ends_with(".tar.gz") || input.ends_with(".tgz") {
        let url = url::Url::parse(input)?;
        if !["http", "https"].contains(&url.scheme()) {
            bail!("archive URL must use HTTP(S)")
        }
        return Ok(vec![input.into()]);
    }
    let s = input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("git@")
        .trim_start_matches("github.com/")
        .trim_start_matches("github.com:")
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let parts: Vec<_> = s.split('/').collect();
    if parts.len() < 2
        || parts[..2].iter().any(|p| {
            p.is_empty()
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        bail!("expected GitHub owner/repository")
    }
    let refs: Vec<_> = if parts.len() >= 4 && ["tree", "commit"].contains(&parts[2]) {
        vec![parts[3..].join("/")]
    } else if parts.len() == 2 {
        vec!["main".into(), "master".into()]
    } else {
        bail!("unsupported GitHub reference")
    };
    Ok(refs
        .into_iter()
        .map(|r| {
            format!(
                "https://codeload.github.com/{}/{}/tar.gz/{r}",
                parts[0], parts[1]
            )
        })
        .collect())
}
/// Reject all links, traversal and special entries; limit both count and sizes.
pub fn extract_tar_gz(reader: impl std::io::Read, root: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    let mut total = 0u64;
    for (i, item) in archive.entries()?.enumerate() {
        if i >= 10_000 {
            bail!("too many archive entries")
        };
        let mut item = item?;
        let raw = item.path()?.to_string_lossy().into_owned();
        let normalized = raw.replace('\\', "/");
        if normalized.trim_matches('/').is_empty() || normalized == "." || normalized == "./" {
            continue;
        }
        let relative = super::manifest::relative_path(&normalized)?;
        let destination = root.join(relative);
        let kind = item.header().entry_type();
        if kind.is_dir() {
            std::fs::create_dir_all(&destination)?;
        } else if kind.is_file() {
            let size = item.header().size()?;
            total = total
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("archive size overflow"))?;
            if size > 128 << 20 || total > 512 << 20 {
                bail!("archive exceeds extraction limits")
            };
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)?;
            std::io::copy(&mut item, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(
                    item.header().mode()? & 0o777,
                ))?;
            }
        } else {
            bail!("archive links and special entries are forbidden")
        }
    }
    Ok(())
}
fn locate(root: &Path) -> Result<PathBuf> {
    if root.join("plugin.json").is_file() {
        return Ok(root.into());
    }
    let mut candidates = vec![];
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.join("plugin.json").is_file() {
            candidates.push(path)
        }
    }
    if candidates.len() != 1 {
        bail!("archive must contain one plugin manifest at root or one level down")
    }
    Ok(candidates.remove(0))
}
fn download(
    input: &str,
    root: &Path,
    progress: &dyn Fn(&str),
) -> Result<(tempfile::TempDir, PathBuf)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let mut last = None;
    for url in archive_urls(input)? {
        let tmp = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(root)?;
        let result = (|| {
            progress("downloading");
            let response = client.get(&url).send()?.error_for_status()?;
            progress("extracting");
            extract_tar_gz(std::io::Read::take(response, 128 << 20), tmp.path())?;
            locate(tmp.path())
        })();
        match result {
            Ok(path) => return Ok((tmp, path)),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no archive candidates")))
}
pub fn inspect(input: &str) -> Result<Manifest> {
    let root = tempfile::tempdir()?;
    let (_tmp, path) = download(input, root.path(), &|_| {})?;
    let mut m = Manifest::load(&path.join("plugin.json"))?;
    m.dir = PathBuf::new();
    m.disabled = false;
    m.source = input.into();
    Ok(m)
}
pub struct Prepared {
    _temporary: tempfile::TempDir,
    source: PathBuf,
    pub manifest: Manifest,
    reference: String,
}
pub fn prepare(root: &Path, input: &str, consent: &Consent) -> Result<Prepared> {
    prepare_with_progress(root, input, consent, &|_| {})
}
pub fn prepare_with_progress(
    root: &Path,
    input: &str,
    consent: &Consent,
    progress: &dyn Fn(&str),
) -> Result<Prepared> {
    prepare_cancellable(
        root,
        input,
        consent,
        progress,
        &std::sync::atomic::AtomicBool::new(false),
    )
}
pub fn prepare_cancellable(
    root: &Path,
    input: &str,
    consent: &Consent,
    progress: &dyn Fn(&str),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Prepared> {
    prepare_cancellable_with_runtime(root, input, consent, progress, cancel, None)
}
pub fn prepare_cancellable_with_runtime(
    root: &Path,
    input: &str,
    consent: &Consent,
    progress: &dyn Fn(&str),
    cancel: &std::sync::atomic::AtomicBool,
    runtime: Option<&str>,
) -> Result<Prepared> {
    std::fs::create_dir_all(root)?;
    let (temporary, source) = download(input, root, progress)?;
    prepare_staged(
        temporary,
        source,
        input.into(),
        consent,
        progress,
        cancel,
        runtime,
    )
}
pub fn prepare_directory(root: &Path, source: &Path, consent: &Consent) -> Result<Prepared> {
    prepare_directory_cancellable(
        root,
        source,
        consent,
        &std::sync::atomic::AtomicBool::new(false),
    )
}
pub fn prepare_directory_cancellable(
    root: &Path,
    source: &Path,
    consent: &Consent,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Prepared> {
    prepare_directory_cancellable_with_runtime(root, source, consent, cancel, None)
}
pub fn prepare_directory_cancellable_with_runtime(
    root: &Path,
    source: &Path,
    consent: &Consent,
    cancel: &std::sync::atomic::AtomicBool,
    runtime: Option<&str>,
) -> Result<Prepared> {
    let source = source.canonicalize()?;
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    if root.starts_with(&source) {
        bail!("install destination cannot be inside source")
    }
    let temporary = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&root)?;
    copy_tree(&source, temporary.path(), &mut (256 * 1024 * 1024))?;
    let staged = temporary.path().to_path_buf();
    prepare_staged(
        temporary,
        staged,
        String::new(),
        consent,
        &|_| {},
        cancel,
        runtime,
    )
}
fn prepare_staged(
    temporary: tempfile::TempDir,
    source: PathBuf,
    reference: String,
    consent: &Consent,
    progress: &dyn Fn(&str),
    cancel: &std::sync::atomic::AtomicBool,
    runtime: Option<&str>,
) -> Result<Prepared> {
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        bail!("plugin install cancelled")
    }
    for marker in MARKERS {
        let path = source.join(marker);
        if path.is_dir() {
            std::fs::remove_dir_all(path)?
        } else if path.exists() {
            std::fs::remove_file(path)?
        }
    }
    let m = Manifest::load(&source.join("plugin.json"))?;
    consent.check(&m)?;
    if let Some((command, args)) = m.install.split_first() {
        progress("building");
        // A child is always reaped, including timeout; no shell evaluation.
        let (command, env) = runtime_command(command, runtime);
        let mut command = Command::new(command);
        command
            .scrub_host_authority()
            .envs(env)
            .args(args.iter().map(|a| platform(a)))
            .current_dir(&source)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        let (child, job) = super::supervisor::windows_job::Job::spawn(
            &mut command,
            claudemon::background_process::CREATION_FLAGS,
        )?;
        #[cfg(not(windows))]
        let child = command.spawn()?;
        let mut child = InstallChild {
            process: child,
            #[cfg(windows)]
            _job: Some(job),
        };
        let start = std::time::Instant::now();
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                bail!("plugin install cancelled")
            }
            if let Some(status) = child.process.try_wait()? {
                if !status.success() {
                    bail!("plugin install command failed: {status}")
                }
                break;
            }
            if start.elapsed() > std::time::Duration::from_secs(300) {
                let _ = child.process.kill();
                let _ = child.process.wait();
                bail!("plugin install command timed out")
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    let built = Manifest::load(&source.join("plugin.json"))?;
    if built.id != m.id {
        bail!("install command changed plugin id")
    }
    Ok(Prepared {
        _temporary: temporary,
        source,
        manifest: built,
        reference,
    })
}
impl Prepared {
    /// Caller must have stopped the existing sidecar before commit.
    pub fn commit(self, root: &Path) -> Result<Manifest> {
        let destination = existing_destination(root, &self.manifest.id)?
            .unwrap_or_else(|| root.join(&self.manifest.id));
        let backup = root.join(format!(".trash-{}", uuid::Uuid::new_v4()));
        let had_old = destination.exists();
        if had_old
            && Manifest::load(&destination.join("plugin.json"))
                .map(|m| m.id != self.manifest.id)
                .unwrap_or(true)
        {
            bail!("install destination belongs to another or invalid plugin")
        }
        // Ignore installer-written loader state, then copy only the existing
        // loader's markers; install code cannot change the persisted identity.
        for marker in MARKERS {
            let path = self.source.join(marker);
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?
            } else if path.exists() {
                std::fs::remove_file(&path)?
            }
            let old = destination.join(marker);
            if had_old && old.is_file() {
                std::fs::copy(old, path)?;
            }
        }
        std::fs::write(self.source.join(".install-source"), self.reference)?;
        if had_old {
            rename_directory(&destination, &backup)?;
        }
        if let Err(error) = rename_directory(&self.source, &destination) {
            if had_old {
                rename_directory(&backup, &destination)?;
            }
            return Err(error.into());
        }
        if had_old {
            let _ = std::fs::remove_dir_all(backup);
        }
        Manifest::load(&destination.join("plugin.json"))
    }
}
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(v: &str) -> Vec<&str> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .collect()
    }
    let (a, b) = (split(a), split(b));
    for i in 0..a.len().max(b.len()) {
        let av = a.get(i).filter(|s| !s.is_empty()).copied().unwrap_or("0");
        let bv = b.get(i).filter(|s| !s.is_empty()).copied().unwrap_or("0");
        let order = match (av.parse::<i64>(), bv.parse::<i64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => av.cmp(bv),
        };
        if !order.is_eq() {
            return order;
        }
    }
    std::cmp::Ordering::Equal
}
pub fn check_updates(manifests: Vec<Manifest>) -> serde_json::Value {
    use std::sync::Mutex;
    let count = manifests.len();
    let queue = Mutex::new(
        manifests
            .into_iter()
            .enumerate()
            .collect::<std::collections::VecDeque<_>>(),
    );
    let results = Mutex::new(vec![serde_json::Value::Null; count]);
    std::thread::scope(|scope| {
        for _ in 0..count.min(6) {
            let queue = &queue;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let Some((index, m)) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let current = m
                        .contributions
                        .get("version")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let mut status =
                        serde_json::json!({"id":m.id,"installed":current,"hasUpdate":false});
                    if !m.source.trim().is_empty() {
                        match inspect(&m.source) {
                            Ok(remote) => {
                                let latest = remote
                                    .contributions
                                    .get("version")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("");
                                status["latest"] = latest.into();
                                status["hasUpdate"] = (!current.trim().is_empty()
                                    && !latest.trim().is_empty()
                                    && compare_versions(latest, current).is_gt())
                                .into();
                            }
                            Err(e) => status["error"] = e.to_string().into(),
                        }
                    }
                    results.lock().unwrap()[index] = status;
                }
            });
        }
    });
    serde_json::Value::Array(results.into_inner().unwrap())
}

fn existing_destination(root: &Path, id: &str) -> Result<Option<PathBuf>> {
    let mut matches = vec![];
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if Manifest::load(&path.join("plugin.json")).is_ok_and(|m| m.id == id) {
            matches.push(path)
        }
    }
    if matches.len() > 1 {
        bail!("multiple installed directories claim plugin {id:?}")
    }
    Ok(matches.pop())
}
/// Seed only the two trusted webview defaults on a genuinely empty directory.
/// Never executes an install command or replaces an existing installation.
pub fn seed_bundled(destination: &Path, examples: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    if std::fs::read_dir(destination)?.next().is_some() {
        return Ok(());
    }
    for name in ["editor", "transcript-timeline"] {
        let source = examples.join(name);
        if !source.exists() {
            continue;
        }
        let temporary = tempfile::Builder::new()
            .prefix(".seed-")
            .tempdir_in(destination)?;
        let staged = temporary.path().join(name);
        copy_tree(&source, &staged, &mut (256 * 1024 * 1024))?;
        let m = Manifest::load(&staged.join("plugin.json"))?;
        if m.id != format!("workspacer.{name}") || m.server.is_some() || !m.install.is_empty() {
            bail!("bundled {name} must be expected webview-only plugin")
        }
        std::fs::rename(staged, destination.join(name))?;
    }
    Ok(())
}

struct InstallChild {
    process: std::process::Child,
    #[cfg(windows)]
    _job: Option<super::supervisor::windows_job::Job>,
}
impl Drop for InstallChild {
    fn drop(&mut self) {
        if !matches!(self.process.try_wait(), Ok(Some(_))) {
            let _ = self.process.kill();
        }
        let _ = self.process.wait();
    }
}

/// Explicit development mode rebuilds in the author's real directory, so
/// relative imports and source-control-aware build steps retain their meaning.
pub fn build_in_place(source: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    build_in_place_with_runtime(source, cancel, None)
}
pub fn build_in_place_with_runtime(
    source: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    runtime: Option<&str>,
) -> Result<()> {
    use super::supervisor::Factory;
    let manifest = Manifest::load(&source.join("plugin.json"))?;
    let Some((command, args)) = manifest.install.split_first() else {
        return Ok(());
    };
    let (command, env) = runtime_command(command, runtime);
    let mut process = super::supervisor::NativeFactory.spawn(&super::supervisor::Spec {
        command,
        args: args.iter().map(|a| platform(a)).collect(),
        directory: source.to_path_buf(),
        env,
        health_url: None,
        log: Some(std::sync::Arc::new(|stream, line| {
            eprintln!("[install {stream}] {line}")
        })),
    })?;
    let started = std::time::Instant::now();
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            process.stop()?;
            bail!("plugin build cancelled")
        }
        if process.exited()? {
            let error = process.exit_error();
            process.stop()?;
            if !error.is_empty() {
                bail!("plugin build failed: {error}")
            }
            return Ok(());
        }
        if started.elapsed() > std::time::Duration::from_secs(300) {
            process.stop()?;
            bail!("plugin build timed out")
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn rename_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        for attempt in 0..6 {
            match std::fs::rename(source, destination) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if attempt == 5 || !matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
                        return Err(error);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150 * (attempt + 1)));
                }
            }
        }
    }
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod runtime_tests {
    #[test]
    fn runtime_override_only_pins_bare_node_for_build_and_sidecar() {
        for command in ["node", "node.exe", "node${exe}"] {
            let (binary, env) = super::runtime_command(command, Some("/desktop/electron"));
            assert_eq!(binary, "/desktop/electron");
            assert_eq!(env["ELECTRON_RUN_AS_NODE"], "1");
        }
        for command in ["npm", "npx", "./node", "/usr/bin/node", "NODE"] {
            let (binary, env) = super::runtime_command(command, Some("/desktop/electron"));
            assert_eq!(binary, command);
            assert!(env.is_empty());
        }
        assert_eq!(super::runtime_command("node", None).0, "node");
    }
}
