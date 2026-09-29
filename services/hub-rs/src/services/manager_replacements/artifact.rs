//! Operation-bound handoff artifacts. Only host-known brief locations can be
//! checkpoint evidence; inspected/opened file identities must remain identical.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn candidate(path: &Path) -> String {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        s.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s.into_owned()
    }
}
fn same_spelling(a: &Path, b: &Path) -> bool {
    #[cfg(not(windows))]
    {
        a.as_os_str() == b.as_os_str()
    }
    #[cfg(windows)]
    {
        fn normalized(p: &Path) -> String {
            let mut s = p.to_string_lossy().replace('/', "\\");
            if s.as_bytes().get(1) == Some(&b':') {
                s.replace_range(0..1, &s[0..1].to_uppercase());
            }
            s
        }
        normalized(a) == normalized(b)
    }
}
fn plain(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        let b = value.as_bytes();
        if b.len() < 3
            || !b[0].is_ascii_alphabetic()
            || b[1] != b':'
            || !matches!(b[2], b'/' | b'\\')
        {
            return false;
        }
        let reserved = regex::Regex::new(
            r"(?i)^(?:(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³]) *(?:\.|$)|CONIN\$$|CONOUT\$$)",
        )
        .unwrap();
        return value[3..].split(['\\', '/']).all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with([' ', '.'])
                && !p.contains(':')
                && !reserved.is_match(p)
        }) || value.len() == 3;
    }
    #[cfg(not(windows))]
    {
        value == "/"
            || value[1..]
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
    }
}
fn open_nofollow(path: &Path) -> Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
    }
    let file = options.open(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata()?.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            bail!("linked file refused");
        }
    }
    Ok(file)
}
fn identity(file: &File) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok((m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
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
    {
        let _ = file;
        bail!("File identity unavailable")
    }
}
struct Verified {
    canonical: PathBuf,
    identity: (u64, u64),
    directory: bool,
    file: bool,
    size: u64,
}
fn verify(path: &Path) -> Result<Verified> {
    if !plain(path) {
        bail!("invalid path spelling");
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let meta = std::fs::symlink_metadata(&current)?;
        if meta.file_type().is_symlink() {
            bail!("linked path refused");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                bail!("reparse path refused");
            }
        }
        if current != path && !meta.is_dir() {
            bail!("non-directory parent");
        }
        if matches!(component, Component::ParentDir | Component::CurDir) {
            bail!("dirty path refused");
        }
    }
    let inspected = std::fs::symlink_metadata(path)?;
    let first = open_nofollow(path)?;
    let before = identity(&first)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before != (inspected.dev(), inspected.ino()) {
            bail!("Path changed before inspection");
        }
    }
    let canonical = std::fs::canonicalize(path)?;
    let final_file = open_nofollow(&canonical)?;
    let final_id = identity(&final_file)?;
    if before.1 == 0 || before != final_id {
        bail!("Path changed during inspection");
    }
    Ok(Verified {
        canonical,
        identity: before,
        directory: inspected.is_dir(),
        file: inspected.is_file(),
        size: inspected.len(),
    })
}
fn bytes_at(path: &Path, verified: &Verified, max: usize) -> Result<Vec<u8>> {
    let file = open_nofollow(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || identity(&file)? != verified.identity
        || meta.len() != verified.size
        || meta.len() > max as u64
    {
        bail!("Checkpoint file identity/size changed before read");
    }
    let mut bytes = vec![];
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max || bytes.len() as u64 != meta.len() {
        bail!("Checkpoint file size changed during read");
    }
    Ok(bytes)
}
pub fn create_path(cwd: &str, operation: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(operation).context("Invalid handoff operation identity")?;
    let mut dir = super::super::paths::canonicalize(Path::new(cwd))?;
    for name in [".workspacer", "manager-handoffs", operation] {
        dir.push(name);
        let meta = match std::fs::symlink_metadata(&dir) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&dir)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
                }
                std::fs::symlink_metadata(&dir)?
            }
            Err(error) => return Err(error.into()),
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                bail!("Handoff directory must not be a reparse point");
            }
        }
        if !meta.is_dir() || meta.file_type().is_symlink() {
            bail!("Handoff directory must be a real local directory");
        }
    }
    Ok(dir.join("handoff.json"))
}
fn exact_ids(rows: &Value, key: &str, expected: &Value) -> bool {
    let (Some(rows), Some(expected)) = (rows.as_array(), expected.as_array()) else {
        return false;
    };
    rows.len() == expected.len()
        && rows
            .iter()
            .map(|r| text(&r[key]))
            .collect::<BTreeSet<_>>()
            .len()
            == rows.len()
        && rows.iter().all(|r| expected.iter().any(|id| *id == r[key]))
}
pub fn validate(op: &Value, receipt_text: &str) -> Result<(String, String)> {
    let cwd = text(&op["launch"]["options"]["cwd"]);
    let expected = super::super::paths::canonicalize(Path::new(cwd))?
        .join(".workspacer")
        .join("manager-handoffs")
        .join(text(&op["operationId"]))
        .join("handoff.json");
    if !same_spelling(Path::new(text(&op["artifactPath"])), &expected) {
        bail!("Handoff artifact path changed");
    }
    let proposal = verify(&expected)?;
    if !proposal.file || proposal.size == 0 || proposal.size > 256 * 1024 {
        bail!("Handoff artifact must be nonempty regular file at most256KiB");
    }
    let bytes = bytes_at(&proposal.canonical, &proposal, 256 * 1024)?;
    let raw = String::from_utf8(bytes.clone()).context("Handoff artifact requires valid UTF8")?;
    let a: Value = serde_json::from_str(&raw)?;
    if a["version"] != 1
        || a["operationId"] != op["operationId"]
        || a["sourceSessionId"] != op["sourceSessionId"]
        || a["cwd"] != cwd
        || a["checkpoint"]["completed"] != true
        || a["checkpoint"]["files"]
            .as_array()
            .is_none_or(Vec::is_empty)
        || !exact_ids(&a["workers"], "sessionId", &op["workerIds"])
        || !exact_ids(&a["tasks"], "taskId", &op["taskIds"])
        || a["workers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| text(&w["instructions"]).trim().is_empty())
        || a["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| text(&t["nextAction"]).trim().is_empty())
        || ["pendingDecisions", "facts"].iter().any(|key| {
            a[*key]
                .as_array()
                .is_none_or(|r| r.iter().any(|v| text(v).trim().is_empty()))
        })
        || text(&a["nextAction"]).trim().is_empty()
    {
        bail!("Handoff checkpoint identity, workers, tasks or decisions invalid");
    }
    let mut roots = vec![PathBuf::from(cwd)];
    roots.extend(
        op["metadata"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| PathBuf::from(text(&m["cwd"]))),
    );
    roots.extend(
        op["projectCwds"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| PathBuf::from(text(p))),
    );
    let mut fleet_brief = false;
    for item in a["checkpoint"]["files"].as_array().unwrap() {
        let path = Path::new(text(&item["path"]));
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let parent = path.parent().context("Invalid brief path")?;
        let root = parent.parent().context("Invalid brief root")?;
        if !plain(path)
            || parent.file_name() != Some(std::ffi::OsStr::new(".workspacer"))
            || !["brief.md", "brief.archive.md"].contains(&name)
            || !roots
                .iter()
                .any(|r| plain(r) && candidate(r) == candidate(root))
        {
            bail!("Checkpoint pointer is not a host-known brief");
        }
        let digest = text(&item["sha256"]);
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            bail!("Checkpoint hash must be lowercase SHA256");
        }
        let pointer_root = verify(root)?;
        let pointer = verify(path)?;
        if !pointer.file {
            bail!("Checkpoint pointer must be regular file");
        }
        let mut matched = None;
        for (index, known) in roots.iter().enumerate() {
            if !plain(known) || candidate(known) != candidate(root) {
                continue;
            }
            let Ok(known_root) = verify(known) else {
                continue;
            };
            if !known_root.directory || known_root.identity != pointer_root.identity {
                continue;
            }
            let Ok(known_file) = verify(&known.join(".workspacer").join(name)) else {
                continue;
            };
            if !known_file.file || known_file.identity != pointer.identity {
                continue;
            }
            if index == 0 && name == "brief.md" {
                fleet_brief = true;
            }
            matched = Some(known_file);
            break;
        }
        let target = matched.context("Checkpoint pointer not a verified host brief")?;
        let brief = bytes_at(&target.canonical, &target, 2 * 1024 * 1024)?;
        if hash(&brief) != digest {
            bail!("Checkpoint hash does not match current bytes");
        }
    }
    if !fleet_brief {
        bail!("Checkpoint must include fleet brief");
    }
    let digest = hash(&bytes);
    let regex = regex::Regex::new(r"```wks-manager-handoff\s*\n([\s\S]*?)\n```").unwrap();
    let blocks: Vec<_> = regex.captures_iter(receipt_text).collect();
    if blocks.len() != 1 {
        bail!("Expected one operation-correlated handoff receipt");
    }
    let receipt: Value = serde_json::from_str(&blocks[0][1])?;
    if receipt["operationId"] != op["operationId"]
        || receipt["sourceSessionId"] != op["sourceSessionId"]
        || receipt["sha256"] != digest
    {
        bail!("Handoff receipt identity or hash mismatch");
    }
    Ok((raw, digest))
}
pub fn preparation_prompt(op: &Value) -> String {
    let example = json!({"version":1,"operationId":op["operationId"],"sourceSessionId":op["sourceSessionId"],"cwd":op["launch"]["options"]["cwd"],"checkpoint":{"completed":true,"files":[{"path":Path::new(text(&op["launch"]["options"]["cwd"])).join(".workspacer/brief.md"),"sha256":"<SHA256 of checkpointed bytes>"}]},"workers":op["workerIds"].as_array().into_iter().flatten().map(|id|json!({"sessionId":id,"instructions":"<exact instructions and current state>"})).collect::<Vec<_>>(),"tasks":op["taskIds"].as_array().into_iter().flatten().map(|id|json!({"taskId":id,"nextAction":"<pending decisions and next action>"})).collect::<Vec<_>>(),"pendingDecisions":[],"facts":[],"nextAction":"<immediate next action>"});
    format!(
        "HOST-OWNED MANAGER HANDOFF {}. Run /checkpoint first. Preparation only: do not dispatch, adopt, terminate, ask user to reopen, or continue task actions. Preserve pending user decisions and exact worker instructions. Host owns replacement and adoption; standalone handoff instructions do not apply.\nProject roots: {}. Write exactly {} as JSON matching this example; include every listed worker/task, and never omit a known remote or still-allocating worker:\n{}\nInclude checkpointed fleet brief and relevant project brief paths with SHA256, never copies. After writing, end with exactly one fenced wks-manager-handoff JSON receipt naming operationId, sourceSessionId and sha256 of handoff.json bytes. Do not continue work.",
        text(&op["operationId"]),
        op["projectCwds"],
        text(&op["artifactPath"]),
        serde_json::to_string_pretty(&example).unwrap()
    )
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn only_plain_local_windows_spellings_can_be_checkpoint_candidates() {
        for path in [
            r"C:\Work\project\brief.md",
            r"c:/Work/project/brief.md",
            r"C:\",
        ] {
            assert!(plain(Path::new(path)), "{path}");
        }
        for path in [
            r"\\server\share\brief.md",
            r"\\?\C:\Work\brief.md",
            r"\Work\brief.md",
            r"C:Work\brief.md",
            r"C:\Work\..\brief.md",
            r"C:\Work\\brief.md",
            r"C:\Work.\brief.md",
            r"C:\Work \brief.md",
            r"C:\Work\NUL.txt",
            r"C:\Work\COM¹.log",
            r"C:\Work\CONIN$",
            r"C:\Work\brief.md:stream",
        ] {
            assert!(!plain(Path::new(path)), "{path}");
        }
    }
    #[test]
    fn opened_file_identity_rejects_same_byte_replacement_after_inspection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brief.md");
        std::fs::write(&path, b"same bytes").unwrap();
        let inspected = verify(&path).unwrap();
        std::fs::rename(&path, directory.path().join("old.md")).unwrap();
        std::fs::write(&path, b"same bytes").unwrap();
        assert!(bytes_at(&path, &inspected, 1024).is_err());
    }
}
