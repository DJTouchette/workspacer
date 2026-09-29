//! Additive brief maintenance and the desktop board's line-preserving moves.
pub mod cards;
pub mod document;
pub mod report;
use super::{
    config::{Config, atomic_bytes},
    paths,
};
use crate::{Handle, Options};
use anyhow::{Result, anyhow, bail};
use document::{parse, stats, trim};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn confined(project: &Path, name: &str) -> Result<PathBuf> {
    let project = paths::canonicalize(project)?;
    if !project.is_dir() {
        bail!("brief project must be an existing directory");
    }
    let path = paths::canonicalize(&project.join(".workspacer").join(name))?;
    if !paths::contained(&path, &project) {
        bail!("brief path escapes its selected project");
    }
    Ok(path)
}
struct Lock {
    path: PathBuf,
    owner: String,
}
impl Lock {
    fn take(path: &Path) -> Result<Self> {
        use std::io::Write;
        let path = path.with_file_name("brief.md.lock");
        let owner = format!(
            "{} {} {}\n",
            std::process::id(),
            chrono::Utc::now().to_rfc3339(),
            uuid::Uuid::new_v4()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(owner.as_bytes()) {
                        let _ = std::fs::remove_file(&path);
                        return Err(e.into());
                    }
                    return Ok(Self { path, owner });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        || (cfg!(windows)
                            && (e.kind() == std::io::ErrorKind::PermissionDenied
                                || e.raw_os_error() == Some(32))) =>
                {
                    ()
                }
                Err(e) => return Err(e.into()),
            }
            if std::fs::symlink_metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(15))
            {
                if std::fs::remove_file(&path).is_ok() {
                    continue;
                }
            }
            if std::time::Instant::now() >= deadline {
                bail!("brief.md is locked by another writer (waited 3000ms)");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn assert_owned(&self) -> Result<()> {
        if std::fs::read_to_string(&self.path).ok().as_deref() != Some(self.owner.as_str()) {
            bail!("brief writer lost its lock lease; no further writes permitted");
        }
        Ok(())
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        if self.assert_owned().is_ok() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
#[derive(Clone)]
struct Lane {
    dir: PathBuf,
    label: String,
    kind: &'static str,
}
pub struct Briefs {
    config: Arc<Config>,
    home: PathBuf,
}
impl Briefs {
    pub fn new(config: Arc<Config>, home: PathBuf) -> Self {
        Self { config, home }
    }
    fn lanes(&self, rows: &[Value]) -> Vec<Lane> {
        let mut lanes = vec![Lane {
            dir: self.home.clone(),
            label: "Fleet".into(),
            kind: "fleet",
        }];
        let mut seen = vec![self.home.clone()];
        let mut add = |dir: &str, label: Option<&str>| {
            if dir.is_empty() {
                return;
            }
            let Ok(dir) = std::path::absolute(dir) else {
                return;
            };
            if seen
                .iter()
                .any(|p| paths::contained(&dir, p) && paths::contained(p, &dir))
            {
                return;
            }
            seen.push(dir.clone());
            lanes.push(Lane {
                label: label
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        dir.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }),
                dir,
                kind: "project",
            });
        };
        let config = self.config.get();
        if let Some(projects) = config["projects"].as_object() {
            for (dir, identity) in projects {
                add(dir, identity["label"].as_str());
            }
        }
        for key in ["favourites", "recent"] {
            for dir in config["directories"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                add(dir, None);
            }
        }
        for key in ["scripts", "widgets"] {
            if let Some(map) = config[key].as_object() {
                for dir in map.keys() {
                    add(dir, None);
                }
            }
        }
        for row in rows.iter().take(300) {
            let dir = text(row, "cwd");
            if !dir.is_empty() && Path::new(dir).join(".workspacer/brief.md").is_file() {
                add(dir, None);
            }
        }
        lanes
    }
    fn load_lane(&self, lane: &Lane) -> Value {
        let dir = &lane.dir;
        let mut base = json!({"key":dir,"dir":dir,"label":lane.label,"kind":lane.kind,"briefPath":dir.join(".workspacer/brief.md"),"archivePath":dir.join(".workspacer/brief.archive.md"),"exists":false,"cards":[],"extras":[],"indexed":false});
        let result = (|| -> Result<()> {
            let path = confined(dir, "brief.md")?;
            let Some(content) = read(&path)? else {
                return Ok(());
            };
            let index = read(&confined(dir, "brief.index.json")?)?
                .and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let (mut cards, extras) =
                cards::cards(&content, index.as_ref().unwrap_or(&Value::Null), false);
            if let Ok(path) = confined(dir, "brief.archive.md") {
                if let Ok(Some(archive)) = read(&path) {
                    cards.extend(
                        cards::cards(&archive, index.as_ref().unwrap_or(&Value::Null), true).0,
                    );
                }
            }
            base["exists"] = json!(true);
            base["indexed"] = json!(index.is_some());
            base["cards"] = json!(cards);
            base["extras"] = json!(extras);
            Ok(())
        })();
        if let Err(e) = result {
            base["error"] = json!(e.to_string());
        }
        base
    }
    pub fn call(&self, method: &str, params: Value, rows: &[Value]) -> Result<Value> {
        let params = if method == "desktop.moveBriefCard" {
            params.get("request").cloned().unwrap_or(params)
        } else {
            params
        };
        if method == "desktop.loadBriefBoard" {
            return Ok(
                json!({"lanes":self.lanes(rows).iter().map(|l|self.load_lane(l)).collect::<Vec<_>>(),"columns":["Now","Direction","Recently","archive"]}),
            );
        }
        let lane = if method == "desktop.moveBriefCard" {
            let key = std::path::absolute(text(&params, "key"))?;
            Some(
                self.lanes(rows)
                    .into_iter()
                    .find(|l| paths::contained(&l.dir, &key) && paths::contained(&key, &l.dir))
                    .ok_or_else(|| {
                        anyhow!("brief board: requested lane is not one of this fleet's projects")
                    })?,
            )
        } else {
            None
        };
        let project = lane
            .as_ref()
            .map(|l| l.dir.clone())
            .unwrap_or_else(|| PathBuf::from(text(&params, "project")));
        let path = confined(&project, "brief.md")?;
        if method == "brief.check" {
            return Ok(report::check(
                &read(&path)?.unwrap_or_default(),
                rows,
                &path.to_string_lossy(),
            ));
        }
        let section = if method == "desktop.moveBriefCard" {
            String::new()
        } else {
            ["Now", "Direction", "Recently", "User"]
                .into_iter()
                .find(|s| s.eq_ignore_ascii_case(trim(text(&params, "section"))))
                .ok_or_else(|| {
                    anyhow!("brief: expected section Now, Direction, Recently, or User")
                })?
                .to_owned()
        };
        let date = today();
        let line = if method == "brief.append" {
            let raw = params["line"]
                .as_str()
                .ok_or_else(|| anyhow!("brief.append requires line text"))?;
            let candidate = if params.get("sessionId").is_some() || params.get("result").is_some() {
                report::compose(raw, &params, &date)?
            } else {
                raw.into()
            };
            Some(document::normalized(&candidate)?)
        } else {
            None
        };
        let archive_path = confined(&project, "brief.archive.md")?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let _lock = Lock::take(&path)?;
        // Repeat containment after lock acquisition, before any filesystem mutation.
        if confined(&project, "brief.md")? != path
            || confined(&project, "brief.archive.md")? != archive_path
        {
            bail!("brief path changed while waiting for lock");
        }
        for _ in 0..5 {
            let before = read(&path)?;
            let content = before.as_deref().unwrap_or("");
            let mut archive_write = None;
            let mut archived = 0;
            let next = match method {
                "brief.append" => document::append(content, &section, line.as_ref().unwrap())?,
                "brief.archive" => {
                    if before.is_none() {
                        bail!("brief board: no brief at {}", path.display());
                    }
                    let bound =
                        |key: &str| -> Result<Option<usize>> {
                            params
                                .get(key)
                                .map(|v| {
                                    v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(
                                        || anyhow!("{key} must be a whole number, zero or more"),
                                    )
                                })
                                .transpose()
                        };
                    let old = read(&archive_path)?;
                    let (next, archive, n) = document::archive(
                        content,
                        old.as_deref().unwrap_or(""),
                        &section,
                        bound("count")?,
                        bound("keep")?,
                        &date,
                    )?;
                    archived = n;
                    if n > 0 {
                        archive_write = Some((old, archive));
                    }
                    next
                }
                "desktop.moveBriefCard" => {
                    if before.is_none() {
                        bail!("brief board: no brief at {}", path.display());
                    }
                    let to = text(&params, "to");
                    if !["Now", "Direction", "Recently", "archive"].contains(&to) {
                        bail!("brief board: unknown destination column");
                    }
                    if to == "archive" {
                        let doc = parse(content);
                        let entry = doc
                            .entries
                            .iter()
                            .find(|e| e.id == text(&params, "entryId"))
                            .ok_or_else(|| {
                                anyhow!("brief board: entry moved or was edited; reload the board")
                            })?;
                        let old = read(&archive_path)?;
                        let archive = document::append_archive(
                            old.as_deref().unwrap_or(""),
                            &entry.lines,
                            &date,
                        );
                        archive_write = Some((old, archive));
                        let mut lines = doc.lines.clone();
                        lines.drain(entry.start..entry.end);
                        lines.join("\n")
                    } else {
                        document::move_entry(content, text(&params, "entryId"), to)?
                    }
                }
                _ => bail!("unknown brief method"),
            };
            _lock.assert_owned()?;
            if read(&path)? != before {
                continue;
            }
            if let Some((old, archive)) = &archive_write {
                if read(&archive_path)? != *old {
                    continue;
                }
                if next != content {
                    _lock.assert_owned()?;
                    atomic_bytes(&archive_path, archive.as_bytes())?;
                }
            }
            if next != content {
                _lock.assert_owned()?;
                atomic_bytes(&path, next.as_bytes())?;
            }
            if let Some(lane) = &lane {
                return Ok(self.load_lane(lane));
            }
            let mut result = stats(&next, &section);
            result["path"] = json!(path);
            result["section"] = json!(section);
            if method == "brief.append" {
                result["line"] = json!(line);
                result["created"] = json!(before.is_none());
            } else {
                result["archivePath"] = json!(archive_path);
                result["archived"] = json!(archived);
                result["date"] = json!(date);
            }
            return Ok(result);
        }
        bail!("brief changed during five write attempts; no brief update committed")
    }
}
pub(crate) fn install(
    mut options: Options,
    config: Arc<Config>,
    home: PathBuf,
    hub: Handle,
) -> Options {
    let service = Arc::new(Briefs::new(config, home));
    for method in [
        "brief.append",
        "brief.archive",
        "brief.check",
        "desktop.loadBriefBoard",
        "desktop.moveBriefCard",
    ] {
        let service = service.clone();
        let hub = hub.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            let hub = hub.clone();
            async move {
                let rows = if matches!(
                    method,
                    "brief.check" | "desktop.loadBriefBoard" | "desktop.moveBriefCard"
                ) {
                    let client = crate::client::Client::connect_service(&hub).await?;
                    let mut result = if method == "brief.check" {
                        client
                            .call_with_timeout(
                                "sessions.snapshots",
                                json!({}),
                                std::time::Duration::from_secs(3),
                            )
                            .await
                    } else {
                        client
                            .call_with_timeout(
                                "analytics.recent",
                                json!({"limit":300}),
                                std::time::Duration::from_secs(3),
                            )
                            .await
                    };
                    if result.is_err() && method != "brief.check" {
                        result = client
                            .call_with_timeout(
                                "sessions.snapshots",
                                json!({}),
                                std::time::Duration::from_secs(3),
                            )
                            .await;
                    }
                    client.close();
                    match result {
                        Ok(value) => value.as_array().cloned().unwrap_or_default(),
                        Err(error) if method == "brief.check" => return Err(error),
                        Err(_) => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                tokio::task::spawn_blocking(move || service.call(method, params, &rows)).await?
            }
        });
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_expired_writer_never_unlinks_a_successors_lock() {
        let root = tempfile::tempdir().unwrap();
        let lease = Lock::take(&root.path().join("brief.md")).unwrap();
        let path = root.path().join("brief.md.lock");
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "new-owner\n").unwrap();
        assert!(lease.assert_owned().is_err());
        drop(lease);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new-owner\n");
    }
}
