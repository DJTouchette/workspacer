//! Small, typed requests for the native workspace's secondary views.
use crate::{backend::Backend, model::Session};
use anyhow::{Result, bail, ensure};
use base64::Engine;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub const RELEASES_URL: &str = "https://github.com/DJTouchette/workspacer/releases";
pub const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum Request {
    /// Read-only view of a chat-linked file on the session's machine.
    FilePreview {
        session: String,
        target: crate::links::FileTarget,
    },
    /// Write an edited file back on the session's machine. Unless `force`,
    /// the hub's current contents must still equal `base` (what the editor
    /// loaded); otherwise nothing is written and the answer is a conflict.
    SaveFile {
        session: String,
        path: String,
        contents: String,
        base: String,
        force: bool,
    },
    /// One directory of a file explorer, read on the session's machine by
    /// `fs.listEntries`. `.git` is always omitted; git-ignored entries are
    /// omitted unless `include_ignored` (older hubs always omit them, and say
    /// so by not echoing `includeIgnored: true`).
    ListDir {
        path: String,
        include_ignored: bool,
    },
    Previews {
        paths: Vec<String>,
    },
    CardDiff {
        session: String,
        path: String,
    },
    TurnChanges {
        session: String,
        turn: String,
        cwd: String,
    },
    Recent,
    Changes {
        cwd: String,
    },
    Diff {
        cwd: String,
        path: String,
        staged: bool,
        untracked: bool,
    },
    Setup {
        provider: String,
        check: bool,
    },
    History {
        session: String,
    },
    SubagentHistory {
        session: String,
        agent: String,
    },
    Upload {
        session: String,
        source: AttachmentSource,
    },
    Updates,
    /// Download and verify the installer from an `Updates` result's `asset`.
    DownloadUpdate {
        asset: Value,
    },
    /// Tailscale sharing and phone pairings on the connected hub.
    Remote,
    /// An owner change to sharing; answers with the refreshed `Remote` state.
    /// Its own key, so a refresh never cancels a half-applied change.
    RemoteAction(crate::remote::Action),
    /// The hub's shared project registry (`config.projects` and its legacy
    /// arrays); see [`crate::projects`].
    Projects,
    /// A user-initiated pin/unpin/forget, verified against the hub's reply.
    SaveProject {
        path: String,
        change: crate::projects::Patch,
    },
    /// Record a launch's project as recently used. Its own key, so it never
    /// supersedes a user's save.
    TouchProject {
        path: String,
        at: i64,
    },
    /// Whether a directory exists on the hub and what git says about it.
    InspectProject {
        path: String,
    },
    /// One level of the hub's filesystem, for choosing a remote folder.
    BrowseFolders {
        path: String,
    },
    /// The hub's shared `agents.childFullAccess`: read (`None`) or set and
    /// verified on readback. Children are launched by the hub, so this is
    /// hub configuration, not a device preference.
    ChildAccess {
        set: Option<bool>,
    },
    /// Downloaded project icons (`iconFile`) from the hub's
    /// `<configDir>/project-icons/`, as small PNGs keyed by file name.
    ProjectIcons {
        files: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub enum AttachmentSource {
    File(PathBuf),
    Image {
        name: String,
        bytes: Arc<Vec<u8>>,
    },
    #[cfg(target_os = "windows")]
    WindowsClipboard,
}

impl Request {
    pub fn key(&self) -> &'static str {
        match self {
            Self::FilePreview { .. } => "file-preview",
            Self::SaveFile { .. } => "file-save",
            Self::ListDir { .. } => "file-tree",
            Self::Previews { .. } => "previews",
            Self::CardDiff { .. } => "card-diff",
            Self::TurnChanges { .. } => "turn-changes",
            Self::Recent => "recent",
            Self::Changes { .. } => "changes",
            Self::Diff { .. } => "diff",
            Self::Setup { .. } => "setup",
            Self::History { .. } => "history",
            Self::SubagentHistory { .. } => "subagent-history",
            Self::Upload { .. } => "upload",
            Self::Updates => "updates",
            Self::DownloadUpdate { .. } => "update-download",
            Self::Remote => "remote",
            Self::RemoteAction(_) => "remote-action",
            Self::Projects => "projects",
            Self::SaveProject { .. } => "project-save",
            Self::TouchProject { .. } => "project-touch",
            Self::InspectProject { .. } => "project-inspect",
            Self::BrowseFolders { .. } => "project-browse",
            Self::ProjectIcons { .. } => "project-icons",
            Self::ChildAccess { .. } => "child-access",
        }
    }
    pub async fn run(&self, backend: &Backend) -> Result<Value> {
        match self {
            Self::FilePreview { target, .. } => {
                let fail = |e: anyhow::Error| {
                    anyhow::anyhow!(crate::links::read_error(target, &e.to_string()))
                };
                match target.kind {
                    crate::links::FileKind::Text => {
                        let value = backend
                            .call("fs.read", json!({"path":target.path}))
                            .await
                            .map_err(fail)?;
                        let contents = value["contents"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("The hub returned no file contents."))?;
                        crate::links::check_text(contents)?;
                        Ok(value)
                    }
                    crate::links::FileKind::Image => {
                        let value = backend
                            .call("fs.readImage", json!({"path":target.path}))
                            .await
                            .map_err(fail)?;
                        tokio::task::spawn_blocking(move || full_image(value))
                            .await?
                            .map_err(fail)
                    }
                }
            }
            Self::SaveFile {
                path,
                contents,
                base,
                force,
                ..
            } => save_file(backend, path, contents, base, *force).await,
            Self::ListDir {
                path,
                include_ignored,
            } => {
                let value = backend
                    .call(
                        "fs.listEntries",
                        json!({"path":path,"includeIgnored":include_ignored}),
                    )
                    .await?;
                ensure!(
                    value["entries"].is_array(),
                    "The hub returned no directory entries."
                );
                Ok(value)
            }
            Self::Previews { paths } => {
                let mut previews = serde_json::Map::new();
                for path in paths.iter().take(8) {
                    let preview = backend.call("fs.readImage", json!({"path":path})).await;
                    let preview = match preview {
                        Ok(value) => tokio::task::spawn_blocking(move || thumbnail(value))
                            .await
                            .unwrap_or_else(|e| Err(e.into())),
                        Err(error) => Err(error),
                    };
                    previews.insert(
                        path.clone(),
                        preview.unwrap_or_else(|e| json!({"error":e.to_string()})),
                    );
                }
                Ok(Value::Object(previews))
            }
            Self::CardDiff { session, path } => {
                backend
                    .call(
                        "desktop.htmlCardReadDiff",
                        json!({"ownerId":session,"target":path}),
                    )
                    .await
            }
            Self::TurnChanges { cwd, .. } => {
                let (status, staged, unstaged) = tokio::join!(
                    backend.call("git.status", json!({"cwd":cwd})),
                    backend.call("git.numstat", json!({"cwd":cwd,"staged":true})),
                    backend.call("git.numstat", json!({"cwd":cwd,"staged":false}))
                );
                Ok(json!({"status":status?,"staged":staged?,"unstaged":unstaged?}))
            }
            Self::Recent => backend.call("sessions.recent", json!({})).await,
            Self::Projects => {
                let mut revision = backend.project_write().await;
                let config = backend.call("config.get", json!({})).await?;
                Ok(project_snapshot(&config, &mut revision))
            }
            Self::SaveProject { path, change } => save_project(backend, path, change).await,
            Self::TouchProject { path, at } => {
                save_project(backend, path, &crate::projects::Patch::Touch(*at)).await
            }
            Self::InspectProject { path } => inspect_project(backend, path).await,
            Self::BrowseFolders { path } => backend.call("fs.listDir", json!({"path":path})).await,
            Self::ChildAccess { set } => {
                let config = match set {
                    // `agents` deep-merges on save, so only this key changes.
                    Some(enabled) => {
                        let saved = backend
                            .call("config.save", json!({"agents":{"childFullAccess":enabled}}))
                            .await?;
                        ensure!(
                            (saved["agents"]["childFullAccess"] == true) == *enabled,
                            "The hub did not save the setting (its config may be busy); try again"
                        );
                        saved
                    }
                    None => backend.call("config.get", json!({})).await?,
                };
                Ok(json!({
                    "childFullAccess": config["agents"]["childFullAccess"] == true,
                    "fleetFullAccess": config["agents"]["fleetFullAccess"] == true,
                }))
            }
            Self::ProjectIcons { files } => {
                let mut icons = serde_json::Map::new();
                for file in files.iter().take(16) {
                    let icon = match backend
                        .call("ui.asset", json!({"file":file,"kind":"icon"}))
                        .await
                    {
                        Ok(value) => tokio::task::spawn_blocking(move || project_icon(&value))
                            .await
                            .unwrap_or_else(|e| Err(e.into())),
                        Err(error) => Err(error),
                    };
                    icons.insert(
                        file.clone(),
                        icon.unwrap_or_else(|e| json!({"error":e.to_string()})),
                    );
                }
                Ok(Value::Object(icons))
            }
            Self::Remote => crate::remote::state(backend).await,
            Self::RemoteAction(action) => crate::remote::apply(backend, action).await,
            Self::Changes { cwd } => backend.call("git.status", json!({"cwd":cwd})).await,
            Self::Diff {
                cwd,
                path,
                staged,
                untracked,
            } => {
                backend
                    .call(
                        "git.diff",
                        json!({"cwd":cwd,"path":path,"staged":staged,"untracked":untracked}),
                    )
                    .await
            }
            Self::History { session } => {
                history_document(backend.conversation(session, None).await?)
            }
            Self::SubagentHistory { session, agent } => {
                let value = backend
                    .call(
                        "sessions.subagentConversation",
                        json!({"sessionId":session,"agentId":agent}),
                    )
                    .await?;
                ensure!(!value.is_null(), "Child transcript is not available yet");
                ensure!(
                    value["session_id"].as_str().is_none_or(|id| id == session)
                        && value["agent_id"].as_str().is_none_or(|id| id == agent),
                    "Child transcript belongs to another session"
                );
                let snapshot: crate::model::ConversationSnapshot = serde_json::from_value(value)?;
                let first_seq = snapshot.first_seq;
                let mut transcript = crate::model::Transcript::default();
                transcript.snapshot(snapshot);
                let rows: Vec<_> = transcript.rows.iter().map(AsRef::as_ref).collect();
                Ok(json!({"rows":rows,"first_seq":first_seq,
                    "omitted":transcript.omitted,"session_id":session,"agent_id":agent}))
            }
            Self::Setup { provider, check } => {
                let installed = backend.call("providers.checkAll", json!({})).await?;
                let readiness = backend
                    .call(
                        "desktop.providerReadiness",
                        json!({"provider":provider,"check":check}),
                    )
                    .await;
                Ok(
                    json!({"installed":installed,"readiness":readiness.as_ref().ok(),"readinessError":readiness.err().map(|e| e.to_string())}),
                )
            }
            Self::Upload { source, .. } => {
                let source = source.clone();
                let (name, bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
                    match source {
                        AttachmentSource::Image { name, bytes } => {
                            normalize_clipboard(name, &bytes)
                        }
                        #[cfg(target_os = "windows")]
                        AttachmentSource::WindowsClipboard => {
                            let mut clipboard = arboard::Clipboard::new()?;
                            let data = clipboard.get_image()?;
                            ensure!(
                                data.width <= 8192
                                    && data.height <= 8192
                                    && data.width.saturating_mul(data.height) <= 16_000_000,
                                "Screenshot dimensions exceed the 16 megapixel limit"
                            );
                            let image = image::RgbaImage::from_raw(
                                data.width as u32,
                                data.height as u32,
                                data.bytes.into_owned(),
                            )
                            .ok_or_else(|| anyhow::anyhow!("Invalid clipboard bitmap"))?;
                            encode_png(image::DynamicImage::ImageRgba8(image))
                        }
                        AttachmentSource::File(path) => {
                            use std::io::Read;
                            let file = std::fs::File::open(&path)?;
                            ensure!(
                                file.metadata()?.len() <= MAX_ATTACHMENT_BYTES as u64,
                                "Attachment exceeds 8 MiB"
                            );
                            let mut bytes = Vec::new();
                            file.take((MAX_ATTACHMENT_BYTES + 1) as u64)
                                .read_to_end(&mut bytes)?;
                            Ok((
                                path.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                bytes,
                            ))
                        }
                    }
                })
                .await??;
                validate_attachment(&name, bytes.len())?;
                let response = backend.call("files.upload", json!({"name":name,"dataBase64":base64::engine::general_purpose::STANDARD.encode(bytes)})).await?;
                let path = response["path"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Upload returned no path"))?;
                ensure!(
                    !path.contains(['\n', '\r', ']']),
                    "Upload returned an invalid path"
                );
                Ok(json!({"name":name,"path":path}))
            }
            Self::Updates => crate::updates::check(&installed_version()).await,
            Self::DownloadUpdate { asset } => {
                let path = crate::updates::download(asset).await?;
                Ok(json!({"installer": path, "version": asset["version"]}))
            }
        }
    }
}

/// Local projection metadata only, never sent to config.save. Allocate while
/// holding the hub transaction guard, before another read/write can begin.
fn project_snapshot(config: &Value, revision: &mut u64) -> Value {
    *revision += 1;
    let mut snapshot = crate::projects::registry(config);
    snapshot["revision"] = json!(*revision);
    snapshot
}

/// Read, patch and write back the shared registry, then confirm the hub kept
/// the change: `config.save` answers a skipped write with the old config.
/// The whole round holds the hub's project-write lock, so a pin and a launch
/// touch (separate request keys, separate receipts) cannot both read the same
/// map and have the later wholesale save erase the earlier change.
async fn save_project(
    backend: &Backend,
    path: &str,
    change: &crate::projects::Patch,
) -> Result<Value> {
    // A new icon URL is fetched by the hub (desktop.downloadProjectIcon:
    // http(s) only, image types only, 2 MiB, content-addressed) BEFORE the
    // registry transaction, so a slow host never holds the write lock and a
    // failed download saves nothing rather than a half-applied identity.
    let resolved;
    let change = match change {
        crate::projects::Patch::Identity(identity) => {
            let mut identity = identity.normalized()?;
            if !identity.favicon.is_empty() && identity.icon_file.is_empty() {
                let stored = backend
                    .call(
                        "desktop.downloadProjectIcon",
                        json!({"url":identity.favicon}),
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Couldn't download the icon: {e}"))?;
                ensure!(
                    stored["ok"] != false,
                    "Couldn't download the icon: {}",
                    stored["error"].as_str().unwrap_or("refused")
                );
                identity.icon_file = stored["file"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("The hub stored no icon file"))?
                    .to_owned();
                identity = identity.normalized()?;
            }
            resolved = crate::projects::Patch::Identity(identity);
            &resolved
        }
        other => other,
    };
    let mut revision = backend.project_write().await;
    let current = backend.call("config.get", json!({})).await?;
    let partial = crate::projects::patch(&current, path, change)?;
    let saved = backend.call("config.save", partial).await?;
    crate::projects::verify(&saved, path, change)?;
    Ok(project_snapshot(&saved, &mut revision))
}

/// Compare and write under the owning host's file lock. The distinct method
/// makes old hubs fail safely instead of ignoring a new conditional parameter.
async fn save_file(
    backend: &Backend,
    path: &str,
    contents: &str,
    base: &str,
    force: bool,
) -> Result<Value> {
    ensure!(
        contents.len() <= crate::links::MAX_EDITABLE_BYTES,
        "This file is {}; the editor saves files up to {}.",
        crate::links::size(contents.len() as u64),
        crate::links::size(crate::links::MAX_EDITABLE_BYTES as u64)
    );
    let saved = backend
        .call(
            "fs.compareWrite",
            json!({
                "path":path,"contents":contents,"expected":base,"force":force
            }),
        )
        .await?;
    if saved["saved"] == true {
        ensure!(
            saved["contents"].as_str() == Some(contents),
            "The hub returned an invalid save acknowledgement."
        );
        let readback = backend.call("fs.read", json!({"path":path})).await?;
        ensure!(
            readback["contents"].as_str() == Some(contents),
            "The file changed again after saving. Reload to see the current contents."
        );
    } else {
        ensure!(
            saved["saved"] == false && saved["conflict"].is_string(),
            "The hub does not support guarded editor saves. Update the hub and try again."
        );
    }
    Ok(saved)
}

/// `exists` comes from listing the folder; `git` from `git.status`, whose
/// failure on an existing folder is reported, not treated as a missing one.
async fn inspect_project(backend: &Backend, path: &str) -> Result<Value> {
    if let Err(error) = backend.call("fs.listDir", json!({"path":path})).await {
        return Ok(json!({"path":path,"exists":false,"error":error.to_string()}));
    }
    Ok(
        match backend.call("git.status", json!({"cwd":path})).await {
            Ok(status) => json!({"path":path,"exists":true,"git":{
                "branch":status["branch"],
                "changes":status["files"].as_array().map_or(0, Vec::len)
            }}),
            Err(error) => json!({"path":path,"exists":true,"gitError":error.to_string()}),
        },
    )
}

/// A viewer-sized image, decoded under the same limits as thumbnails and
/// re-encoded so the UI thread only ever uploads a bounded PNG.
fn full_image(value: Value) -> Result<Value> {
    let size = value["size"].as_u64();
    let image = decode_preview(&value)?;
    let (width, height) = (image.width(), image.height());
    let side = crate::links::MAX_IMAGE_SIDE;
    let image = if width > side || height > side {
        image.resize(side, side, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(json!({
        "width": width,
        "height": height,
        "size": size,
        "png": base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()),
    }))
}

fn decode_preview(value: &Value) -> Result<image::DynamicImage> {
    let url = value["dataUrl"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No image preview available"))?;
    ensure!(url.len() <= 12 * 1024 * 1024, "Image preview exceeds 8 MiB");
    let encoded = url
        .split_once(";base64,")
        .ok_or_else(|| anyhow::anyhow!("Invalid preview encoding"))?
        .1;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// A project icon from `ui.asset` (`dataBase64` + `mime`) as a 64 px PNG.
/// SVG is refused: native draws only bounded raster decodes.
fn project_icon(value: &Value) -> Result<Value> {
    let mime = value["mime"].as_str().unwrap_or("");
    ensure!(
        mime != "image/svg+xml",
        "SVG project icons show on the desktop app only"
    );
    let data = value["dataBase64"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("The hub returned no icon data"))?;
    ensure!(data.len() <= 3 * 1024 * 1024, "Icon exceeds 2 MiB");
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.thumbnail(64, 64);
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(json!({"png":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}))
}

fn thumbnail(value: Value) -> Result<Value> {
    let image = decode_preview(&value)?.thumbnail(640, 320);
    let width = image.width();
    let height = image.height();
    let (_, bytes) = encode_png(image)?;
    Ok(
        json!({"width":width,"height":height,"dataUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}),
    )
}

fn encode_png(image: image::DynamicImage) -> Result<(String, Vec<u8>)> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    let bytes = bytes.into_inner();
    validate_attachment("Screenshot.png", bytes.len())?;
    Ok(("Screenshot.png".into(), bytes))
}

fn normalize_clipboard(name: String, bytes: &[u8]) -> Result<(String, Vec<u8>)> {
    ensure!(
        bytes.len() <= MAX_ATTACHMENT_BYTES,
        "Screenshot exceeds 8 MiB"
    );
    if name.ends_with(".tiff") || name.ends_with(".bmp") {
        let mut reader =
            image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        encode_png(reader.decode()?)
    } else {
        Ok((name, bytes.to_vec()))
    }
}

pub fn validate_attachment(name: &str, len: usize) -> Result<()> {
    ensure!(
        len > 0 && len <= MAX_ATTACHMENT_BYTES,
        "Choose a nonempty attachment up to 8 MiB"
    );
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !["png", "jpg", "jpeg", "gif", "webp", "pdf"].contains(&ext.as_str()) {
        bail!("Choose a PNG, JPG, GIF, WebP, or PDF file");
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct RequestState {
    pub number: u64,
    pub request: Request,
    pub loading: bool,
    pub value: Arc<Value>,
    pub error: Option<String>,
}

/// Notification transitions, never historical state inferred on initial connection.
pub fn attention_transition(old: &Session, new: &Session) -> Option<&'static str> {
    if new.approval.is_some() && new.approval != old.approval {
        Some("Approval needed")
    } else if new.questions.is_some() && new.questions != old.questions {
        Some("Your answer is needed")
    } else if matches!(
        old.state.as_str(),
        "working" | "thinking" | "running" | "responding" | "tool" | "executing"
    ) && matches!(new.state.as_str(), "idle" | "done" | "input")
    {
        Some("Work completed")
    } else {
        None
    }
}

/// Chunk long messages before display, so pagination never loses their beginning.
fn history_document(value: Value) -> Result<Value> {
    let snapshot: crate::model::ConversationSnapshot = serde_json::from_value(value)?;
    let first_seq = snapshot.first_seq;
    let mut transcript = crate::model::Transcript::default();
    for item in snapshot.items {
        transcript.append_history(item);
    }
    let mut rows = Vec::new();
    for row in &transcript.rows {
        if row.tool.is_none() && row.text.len() > crate::model::MAX_ROW_BYTES {
            let mut offset = 0;
            let mut part = 1;
            while offset < row.text.len() {
                let chunk = crate::transcript::head(&row.text[offset..], 32768);
                offset += chunk.len();
                rows.push(json!({"role":row.role,"text":chunk,"part":part,"continued":true,"timestamp":row.timestamp}));
                part += 1;
            }
        } else {
            rows.push(serde_json::to_value(&**row)?);
        }
    }
    for (ix, row) in rows.iter_mut().enumerate() {
        row["key"] = json!(ix);
    }
    Ok(json!({"rows":rows,"first_seq":first_seq}))
}

/// The installer records the release independently of Cargo's crate version.
pub fn installed_version() -> String {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("build-stamp.json")))
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["version"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{} (development build)", env!("CARGO_PKG_VERSION")))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_requests_keep_reads_and_changes_apart() {
        assert_eq!(Request::Remote.key(), "remote");
        assert_eq!(
            Request::RemoteAction(crate::remote::Action::Serve(true)).key(),
            "remote-action"
        );
    }
    #[test]
    fn attachments_are_bounded_and_typed() {
        assert!(validate_attachment("screen.PNG", 100).is_ok());
        assert!(validate_attachment("x.exe", 100).is_err());
        assert!(validate_attachment("x.png", MAX_ATTACHMENT_BYTES + 1).is_err());
        assert!(validate_attachment("x.pdf", 0).is_err());
    }
    #[test]
    fn attention_only_notifies_new_decisions_and_completed_work() {
        let old = Session {
            state: "working".into(),
            ..Default::default()
        };
        let new = Session {
            state: "idle".into(),
            ..Default::default()
        };
        assert_eq!(attention_transition(&old, &new), Some("Work completed"));
        assert_eq!(attention_transition(&new, &new), None);
        let approval = Session {
            approval: Some(json!({"tool":"Read"})),
            ..new
        };
        assert_eq!(
            attention_transition(&old, &approval),
            Some("Approval needed")
        );
        assert_eq!(attention_transition(&approval, &approval), None);
    }
    #[test]
    fn history_preserves_the_start_and_end_of_long_unicode_messages() {
        let text = format!("BEGIN{}END", "🐙".repeat(40000));
        let history = history_document(
            json!({"seq":7,"first_seq":1,"items":[{"kind":"assistant_text","text":text}]}),
        )
        .unwrap();
        let rows = history["rows"].as_array().unwrap();
        assert!(rows.len() > 4);
        let rebuilt = rows
            .iter()
            .map(|row| row["text"].as_str().unwrap())
            .collect::<String>();
        assert_eq!(rebuilt, text);
        assert!(
            rows.iter()
                .all(|row| row["text"].as_str().unwrap().len() <= 32 * 1024)
        );
    }
    #[test]
    fn history_keeps_large_tool_payloads_paired_and_lossless() {
        let input = "input".repeat(20000);
        let output = "output".repeat(20000);
        let history = history_document(json!({"seq":2,"first_seq":1,"items":[
            {"kind":"tool_use","id":"t","name":"Bash","input":{"command":input}},
            {"kind":"tool_result","tool_use_id":"t","content":output}
        ]}))
        .unwrap();
        assert_eq!(history["rows"].as_array().unwrap().len(), 1);
        assert_eq!(history["rows"][0]["tool"]["output"], output);
        let tool: crate::transcript::Tool =
            serde_json::from_value(history["rows"][0]["tool"].clone()).unwrap();
        assert_eq!(tool.value()["command"], input);
        assert!(!tool.clipped);
    }
    #[test]
    fn image_previews_are_decoded_and_resized_before_reaching_the_ui() {
        let (_, bytes) = encode_png(image::DynamicImage::new_rgb8(1280, 640)).unwrap();
        let preview=thumbnail(json!({"dataUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))})).unwrap();
        assert_eq!(preview["width"], 640);
        assert_eq!(preview["height"], 320);
        assert!(thumbnail(json!({"dataUrl":"data:image/png;base64,bm90IGFuIGltYWdl"})).is_err());
    }
    #[test]
    fn viewer_images_keep_source_size_but_upload_a_bounded_png() {
        let encode = |w, h| {
            let (_, bytes) = encode_png(image::DynamicImage::new_rgb8(w, h)).unwrap();
            json!({"size":bytes.len(),"dataUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))})
        };
        let decoded = |value: &Value| {
            let png = base64::engine::general_purpose::STANDARD
                .decode(value["png"].as_str().unwrap())
                .unwrap();
            image::load_from_memory(&png).unwrap()
        };
        let small = full_image(encode(300, 200)).unwrap();
        assert_eq!(
            (small["width"].as_u64(), small["height"].as_u64()),
            (Some(300), Some(200))
        );
        assert_eq!(decoded(&small).width(), 300);
        let large = full_image(encode(5120, 1280)).unwrap();
        assert_eq!(large["width"], 5120);
        let shown = decoded(&large);
        assert_eq!((shown.width(), shown.height()), (2560, 640));
        assert!(full_image(json!({"dataUrl":"data:image/png;base64,bm90IGFuIGltYWdl"})).is_err());
        assert!(full_image(json!({})).is_err());
    }
    #[test]
    fn clipboard_bitmaps_are_encoded_as_uploadable_png() {
        let image = image::DynamicImage::new_rgb8(2, 2);
        for (ext, format) in [
            ("bmp", image::ImageFormat::Bmp),
            ("tiff", image::ImageFormat::Tiff),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            let (name, encoded) =
                normalize_clipboard(format!("Screenshot.{ext}"), bytes.get_ref()).unwrap();
            assert_eq!(name, "Screenshot.png");
            assert!(encoded.starts_with(b"\x89PNG"));
        }
    }
}
