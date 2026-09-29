use super::paths;
use crate::Options;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

type GitCommand = Arc<dyn Fn() -> tokio::process::Command + Send + Sync>;

pub(crate) fn install(options: Options, home: PathBuf) -> Options {
    install_with_git(
        options,
        home,
        Arc::new(|| tokio::process::Command::new("git")),
    )
}
fn install_with_git(mut options: Options, home: PathBuf, git: GitCommand) -> Options {
    for method in [
        "fs.listDir",
        "fs.listEntries",
        "fs.read",
        "fs.readImage",
        "desktop.readFileBytes",
        "desktop.filePickerList",
        "fs.write",
        "app.getCwd",
        "app.supervisorHome",
    ] {
        let home = home.clone();
        let git = git.clone();
        options = options.handler(method, move |_, params| {
            let home = home.clone();
            let git = git.clone();
            async move {
                if method == "fs.listEntries" {
                    let listing = tokio::task::spawn_blocking(move || {
                        let path = paths::canonicalize(Path::new(path_parameter(&params)?))?;
                        DirectoryListing::read(path)
                    })
                    .await??;
                    // The command future stays in the owned handler: cancelling
                    // it drops the process-group/job guard before hub teardown.
                    let ignored = git_ignored_async(&listing.path, &listing.names, git).await;
                    return tokio::task::spawn_blocking(move || listing.finish(&ignored)).await?;
                }
                tokio::task::spawn_blocking(move || call(method, params, &home)).await?
            }
        });
    }
    options
}
pub fn call(method: &str, params: Value, home: &Path) -> Result<Value> {
    if method == "desktop.filePickerList" {
        return picker(&params, home);
    }
    if method == "app.getCwd" {
        return Ok(json!(std::env::current_dir()?));
    }
    if method == "app.supervisorHome" {
        if home.as_os_str().is_empty() {
            return Ok(json!(""));
        }
        let path = home.join(".workspacer");
        create_directory_tree(&path)?;
        return Ok(json!(path));
    }
    let requested = path_parameter(&params)?;
    let requested = if method == "fs.listDir"
        && requested
            .trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
            .is_empty()
    {
        home
    } else {
        Path::new(requested)
    };
    let path = paths::canonicalize(requested)?;
    match method {
        "fs.read" => read(&path),
        "fs.readImage" => super::image_preview::read(&path),
        "desktop.readFileBytes" => {
            use base64::Engine;
            let bytes = bounded_bytes(&path, 16 * 1024 * 1024)?;
            Ok(
                json!({"name":path.file_name().unwrap_or_default().to_string_lossy(),"dataBase64":base64::engine::general_purpose::STANDARD.encode(bytes)}),
            )
        }
        "fs.write" => {
            let text = match params.get("contents") {
                None | Some(Value::Null) => "",
                Some(v) => v.as_str().ok_or_else(|| anyhow!("contents must be text"))?,
            };
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o644);
            }
            create_directory_tree(path.parent().ok_or_else(|| anyhow!("path has no parent"))?)?;
            let mut file = options.open(&path)?;
            file.write_all(text.as_bytes())?;
            Ok(json!({"ok":true}))
        }
        "fs.listDir" => {
            let mut dirs = Vec::new();
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir()
                    && !entry.file_name().to_string_lossy().starts_with('.')
                {
                    dirs.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            dirs.sort();
            Ok(json!({"path":path,"parent":path.parent().unwrap_or(&path),"home":home,"dirs":dirs}))
        }
        "fs.listEntries" => {
            let listing = DirectoryListing::read(path)?;
            let ignored = git_ignored(&listing.path, &listing.names);
            listing.finish(&ignored)
        }
        _ => bail!("unknown filesystem method"),
    }
}
fn path_parameter(params: &Value) -> Result<&str> {
    if !params.is_object() && !params.is_null() {
        bail!("filesystem parameters must be a JSON object");
    }
    match params.get("path") {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(path)) => Ok(path),
        _ => bail!("path must be text"),
    }
}
fn create_directory_tree(path: &Path) -> Result<()> {
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o755);
    }
    directory.create(path)?;
    Ok(())
}
struct DirectoryListing {
    path: PathBuf,
    entries: Vec<std::fs::DirEntry>,
    names: Vec<String>,
}
impl DirectoryListing {
    fn read(path: PathBuf) -> Result<Self> {
        let entries = std::fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        let names = entries
            .iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != ".git")
            .collect();
        Ok(Self {
            path,
            entries,
            names,
        })
    }
    fn finish(self, ignored: &BTreeSet<String>) -> Result<Value> {
        let mut result = Vec::new();
        for entry in self.entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || ignored.contains(&name) {
                continue;
            }
            let is_dir = entry.file_type()?.is_dir()
                || std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
            result.push(json!({"name":name,"path":entry.path(),"isDir":is_dir}));
        }
        result.sort_by(|a, b| {
            b["isDir"]
                .as_bool()
                .cmp(&a["isDir"].as_bool())
                .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
        });
        Ok(json!({"path":self.path,"entries":result}))
    }
}
pub fn read(path: &Path) -> Result<Value> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        bail!("not a regular file: {}", path.display());
    }
    const MAX: usize = 5 * 1024 * 1024;
    if meta.len() > MAX as u64 {
        bail!("file is {} bytes (max {MAX})", meta.len());
    }
    let bytes = bounded_bytes(path, MAX)?;
    if bytes.contains(&0) {
        bail!("file appears to be binary");
    }
    let contents = String::from_utf8(bytes).map_err(|_| anyhow!("file is not valid UTF-8"))?;
    Ok(json!({"path":path,"contents":contents,"size":meta.len()}))
}
pub const GIT_NO_EXEC: &[&str] = &[
    "core.fsmonitor=",
    "core.pager=cat",
    "core.sshCommand=",
    "core.askPass=",
    "core.editor=",
    "core.alternateRefsCommand=",
    "core.gitProxy=",
    "credential.helper=",
    "sequence.editor=",
    "uploadpack.packObjectsHook=",
];
async fn git_ignored_async(path: &Path, names: &[String], git: GitCommand) -> BTreeSet<String> {
    if names.is_empty() {
        return BTreeSet::new();
    }
    let bytes = names.join("\0").into_bytes();
    let mut command = git();
    command.current_dir(path);
    for pair in GIT_NO_EXEC {
        command.args(["-c", pair]);
    }
    command.args([
        "-c",
        "core.quotePath=false",
        "check-ignore",
        "-z",
        "--stdin",
    ]);
    let Ok(output) = super::owned_process::capture_input(
        &mut command,
        &bytes,
        16 * 1024 * 1024,
        1024 * 1024,
        std::time::Duration::from_secs(5),
    )
    .await
    else {
        return BTreeSet::new();
    };
    if !output.status.success() && output.status.code() != Some(1) {
        return BTreeSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn git_ignored(path: &Path, names: &[String]) -> BTreeSet<String> {
    if names.is_empty() {
        return BTreeSet::new();
    }
    let path = path.to_owned();
    let names = names.to_vec();
    // Preserve the public synchronous API, including calls from inside Tokio.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        Some(runtime.block_on(git_ignored_async(
            &path,
            &names,
            Arc::new(|| tokio::process::Command::new("git")),
        )))
    })
    .join()
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Bound the bytes actually read, and prove the opened regular file is still
/// the selected object. Nonblocking opens also refuse FIFO replacement races.
pub(crate) fn bounded_bytes(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !std::fs::metadata(path)?.is_file() {
        bail!("choose a regular file");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        bail!("choose an unchanged regular file up to {limit} bytes");
    }
    let current = options.open(path)?;
    if file_identity(&file)? != file_identity(&current)? || paths::canonicalize(path)? != path {
        bail!("selected file changed before reading");
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("file exceeds {limit} bytes");
    }
    Ok(bytes)
}
#[cfg(unix)]
pub(crate) fn file_identity(file: &std::fs::File) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok((m.dev(), m.ino()))
}
#[cfg(windows)]
pub(crate) fn file_identity(file: &std::fs::File) -> Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn file_identity(_: &std::fs::File) -> Result<(u64, u64)> {
    bail!("file identity unavailable on this platform")
}
fn picker(params: &Value, home: &Path) -> Result<Value> {
    let raw = match params.get("path") {
        None => "",
        Some(v) => v.as_str().ok_or_else(|| anyhow!("Invalid directory"))?,
    };
    let requested = if raw.is_empty() || raw == "~" {
        home.into()
    } else if raw.starts_with("~/") || raw.starts_with("~\\") {
        home.join(&raw[2..])
    } else {
        PathBuf::from(raw)
    };
    let resolved = std::fs::canonicalize(requested)?;
    let directory = if resolved.is_dir() {
        resolved
    } else {
        resolved
            .parent()
            .ok_or_else(|| anyhow!("file has no parent"))?
            .to_path_buf()
    };
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&directory)? {
        let item = item?;
        if entries.len() == 10000 {
            bail!("Directory has too many entries; choose a narrower path");
        }
        entries.push(json!({"name":item.file_name().to_string_lossy(),"path":item.path(),"isDir":item.path().is_dir()}));
    }
    entries.sort_by(|a, b| {
        b["isDir"]
            .as_bool()
            .cmp(&a["isDir"].as_bool())
            .then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap()
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap().to_lowercase())
            })
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(
        json!({"path":directory,"parent":directory.parent().unwrap_or(&directory),"home":home,"entries":entries}),
    )
}

#[cfg(all(test, unix))]
mod read_tests {
    use super::*;
    #[test]
    fn file_creation_permissions_and_missing_home_are_checked_in_an_isolated_process() {
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "WKS_FS_MODE_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            std::env::set_current_dir(&root).unwrap();
            unsafe {
                libc::umask(0);
            }
            assert_eq!(
                call("app.supervisorHome", Value::Null, Path::new("")).unwrap(),
                json!("")
            );
            assert!(!root.join(".workspacer").exists());
            call("app.supervisorHome", Value::Null, &root).unwrap();
            let file = root.join("new/child/file");
            call("fs.write", json!({"path":file,"contents":"first"}), &root).unwrap();
            for directory in [
                root.join(".workspacer"),
                root.join("new"),
                root.join("new/child"),
            ] {
                assert_eq!(
                    std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                    0o755
                );
            }
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o644
            );
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o751)).unwrap();
            call("fs.write", json!({"path":file,"contents":"second"}), &root).unwrap();
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o751
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "services::files::read_tests::file_creation_permissions_and_missing_home_are_checked_in_an_isolated_process", "--nocapture"])
            .env(CHILD, root.path()).output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn named_pipe_is_refused_before_a_blocking_open() {
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
        let root = tempfile::tempdir().unwrap();
        let pipe = root.path().join("fifo");
        let name = std::ffi::CString::new(pipe.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let selected = pipe.clone();
        let reader = std::thread::spawn(move || {
            let _ = tx.send(read(&selected));
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(1));
        // Release a regressed blocking reader before failing so the test itself
        // cannot leave a stuck thread in the shared library suite.
        let writer = if result.is_err() {
            std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&pipe)
                .ok()
        } else {
            None
        };
        reader.join().unwrap();
        drop(writer);
        assert!(result.expect("fs.read blocked on a FIFO").is_err());
    }
}

#[cfg(all(test, unix))]
mod listing_process_tests {
    use super::*;
    use crate::{Hub, client::Client};
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    #[tokio::test]
    async fn actual_ignore_process_receives_all_guard_args_and_nul_delimited_names() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().to_owned();
        let git: GitCommand = Arc::new(|| {
            let mut command = tokio::process::Command::new("/bin/sh");
            command.args([
                "-c",
                "printf '%s\\0' \"$@\" > argv; cat > stdin; printf 'ignored\\0'",
                "git-spy",
            ]);
            command
        });
        let names = ["ignored".to_owned(), "é\nfile".into()];
        let ignored = git_ignored_async(&dir, &names, git).await;
        assert_eq!(ignored, ["ignored".to_owned()].into());
        let argv = std::fs::read(dir.join("argv")).unwrap();
        let args: Vec<_> = argv
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
            .map(|b| std::str::from_utf8(b).unwrap())
            .collect();
        let expected: Vec<_> = GIT_NO_EXEC
            .iter()
            .flat_map(|key| ["-c", *key])
            .chain([
                "-c",
                "core.quotePath=false",
                "check-ignore",
                "-z",
                "--stdin",
            ])
            .collect();
        assert_eq!(args, expected);
        assert_eq!(
            std::fs::read(dir.join("stdin")).unwrap(),
            names.join("\0").as_bytes()
        );
    }
    #[tokio::test]
    async fn async_listing_preserves_ignore_status_directory_order_and_fallback() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("folder")).unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        std::fs::write(root.path().join("ignored"), "").unwrap();
        std::fs::write(root.path().join("visible"), "").unwrap();
        for status in [0, 1, 2] {
            let git: GitCommand = Arc::new(move || {
                let mut command = tokio::process::Command::new("/bin/sh");
                command.args([
                    "-c",
                    &format!("cat >/dev/null; printf 'ignored\\0'; exit {status}"),
                ]);
                command
            });
            let hub = Hub::start(install_with_git(
                Options::default(),
                root.path().into(),
                git,
            ))
            .unwrap();
            hub.ready().await.unwrap();
            let client = Client::connect(&hub.handle()).await.unwrap();
            let result = client
                .call("fs.listEntries", json!({"path":root.path()}))
                .await
                .unwrap();
            let names: Vec<_> = result["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["name"].as_str().unwrap())
                .collect();
            assert_eq!(
                names,
                if status == 2 {
                    vec!["folder", "ignored", "visible"]
                } else {
                    vec!["folder", "visible"]
                }
            );
            assert_eq!(result["entries"][0]["isDir"], true);
            tokio::task::spawn_blocking(move || hub.shutdown())
                .await
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn hub_shutdown_cancels_git_ignore_tree_without_inheriting_host_tokens() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("repository");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::write(cwd.join("entry"), "visible").unwrap();
        let executable = root.path().join("fake-git");
        std::fs::write(
            &executable,
            r##"#!/bin/sh
[ -z "${HUB_TOKEN+x}" ] && [ -z "${WKS_MCP_TOKEN+x}" ] && [ -z "${WKS_MCP_HUB_TOKEN+x}" ] || exit 72
[ "$PROVIDER_FIXTURE_KEY" = "retained" ] || exit 73
printf ok > scrubbed
printf '%s' "$$" > leader.pid
trap '' HUP TERM
/bin/sh -c 'trap "" HUP TERM; printf "%s" "$$" > descendant.pid; : > ready; sleep 3; : > escaped' &
wait
"##,
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let before: Vec<_> = claudemon::child_env::HOST_AUTHORITY_KEYS
            .iter()
            .map(|key| std::env::var_os(key))
            .collect();
        let git: GitCommand = Arc::new(move || {
            let mut command = tokio::process::Command::new(&executable);
            for key in claudemon::child_env::HOST_AUTHORITY_KEYS {
                command.env(key, "fixture-host-secret");
            }
            command.env("PROVIDER_FIXTURE_KEY", "retained");
            command
        });
        let hub = Hub::start(install_with_git(
            Options::default(),
            root.path().into(),
            git,
        ))
        .unwrap();
        hub.ready().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        let selected = cwd.clone();
        let request = tokio::spawn(async move {
            client
                .call("fs.listEntries", json!({"path":selected}))
                .await
        });
        let ready = tokio::time::timeout(Duration::from_secs(2), async {
            while !cwd.join("ready").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // Always stop the hub before asserting readiness, including regression failures.
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || hub.shutdown()),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        ready.expect("fake Git never reached the owned command phase");
        assert_eq!(std::fs::read_to_string(cwd.join("scrubbed")).unwrap(), "ok");
        assert!(request.await.unwrap().is_err());
        let descendant: i32 = std::fs::read_to_string(cwd.join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let stopped = || {
            #[cfg(target_os = "linux")]
            {
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{descendant}/stat")) else {
                    return true;
                };
                return stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, tail)| tail.starts_with('Z'));
            }
            #[cfg(not(target_os = "linux"))]
            {
                (unsafe { libc::kill(descendant, 0) }) != 0
            }
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while !stopped() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Git descendant outlived hub cancellation");
        tokio::time::sleep(Duration::from_millis(3200)).await;
        assert!(
            !cwd.join("escaped").exists(),
            "cancelled Git child performed its delayed side effect"
        );
        assert!(
            before
                == claudemon::child_env::HOST_AUTHORITY_KEYS
                    .iter()
                    .map(|key| std::env::var_os(key))
                    .collect::<Vec<_>>(),
            "parent authority environment changed"
        );
    }
}
