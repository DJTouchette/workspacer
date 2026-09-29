//! Attachment spill runs under the same OS identity as the execution backend.
use crate::Options;
use anyhow::{Result, bail};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
const MAX_BYTES: usize = 24 << 20;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    #[serde(default)]
    name: String,
    #[serde(default)]
    data_base64: String,
}
fn directory() -> PathBuf {
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    let uid = "-1";
    std::env::temp_dir().join(format!("workspacer-uploads-{uid}"))
}
fn store(params: Value, dir: &Path) -> Result<Value> {
    let params: Params = serde_json::from_value(params)
        .map_err(|e| anyhow::anyhow!("files.upload: bad params: {e}"))?;
    if params.data_base64.is_empty() {
        bail!("files.upload: dataBase64 is required")
    }
    if params.data_base64.len() > (MAX_BYTES / 3 + 1) * 4 {
        bail!("files.upload: payload exceeds 24 MiB")
    }
    // Go's standard decoder accepts CR/LF in a base64 payload, not spaces.
    let encoded = params.data_base64.replace(['\r', '\n'], "");
    let decoder = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    );
    let bytes = decoder
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("files.upload: dataBase64 is not valid base64"))?;
    if bytes.is_empty() {
        bail!("files.upload: empty payload")
    }
    if bytes.len() > MAX_BYTES {
        bail!("files.upload: payload exceeds 24 MiB")
    }
    let ext = Path::new(&params.name)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["png", "jpg", "jpeg", "gif", "webp", "pdf"].contains(&ext.as_str()) {
        bail!("files.upload: extension not allowed (png, jpg, jpeg, gif, webp, pdf)")
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let metadata = std::fs::symlink_metadata(dir)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "upload directory is not a regular directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            metadata.uid() == unsafe { libc::getuid() } && metadata.mode() & 0o077 == 0,
            "upload directory must be private to this user"
        );
    }
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    let name = format!(
        "m-{}-{}.{}",
        chrono::Utc::now().timestamp_millis(),
        &uuid::Uuid::new_v4().simple().to_string()[..8],
        ext
    );
    let path = dir.join(name);
    file.persist_noclobber(&path).map_err(|e| e.error)?;
    Ok(json!({"path":path,"size":bytes.len()}))
}
fn within_envelope(params: &Value) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("upload envelope exceeds limit"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget((MAX_BYTES / 3 + 1) * 4 + 16 * 1024), params).is_ok()
}
pub(crate) fn install_front(options: Options, hub: crate::Handle) -> Options {
    let forward = options.uploads_to_worker;
    options.handler("files.upload", move |_, params| {
        let hub = hub.clone();
        async move {
            if forward {
                anyhow::ensure!(within_envelope(&params), "files.upload: payload too large");
                let caller = crate::client::Client::connect_service(&hub).await?;
                let result = caller
                    .call_with_timeout(
                        "files.receiveUpload",
                        params,
                        std::time::Duration::from_secs(60),
                    )
                    .await;
                caller.close();
                result
            } else {
                tokio::task::spawn_blocking(move || store(params, &directory())).await?
            }
        }
    })
}
pub(crate) fn install(options: Options) -> Options {
    options.handler("files.receiveUpload", |_, params| async move {
        tokio::task::spawn_blocking(move || store(params, &directory())).await?
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn public_upload_forwards_once_with_receiver_authority_and_never_falls_back() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root = tempfile::tempdir().unwrap();
        let tokens = root.path().join("tokens.json");
        let triage = crate::auth::mint(&tokens, crate::auth::Scope::Triage, "viewer").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let mut options =
            Options::default().handler("files.receiveUpload", move |caller, params| {
                let called = called.clone();
                async move {
                    assert!(caller.authenticated_host);
                    assert_eq!(params["name"], "photo.png");
                    called.fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("worker acknowledgement unavailable")
                }
            });
        options.token = "owner".into();
        options.scoped_tokens = Some(tokens);
        options.uploads_to_worker = true;
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let caller = crate::client::Client::from_connection(
            hub.handle()
                .connect_authenticated(triage.token, false)
                .await
                .unwrap(),
        );
        assert!(
            caller
                .call(
                    "files.receiveUpload",
                    json!({"name":"photo.png","dataBase64":"YQ=="})
                )
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let error = caller
            .call(
                "files.upload",
                json!({"name":"photo.png","dataBase64":"YQ=="}),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("worker acknowledgement unavailable")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        hub.shutdown().unwrap();
    }
    #[test]
    fn upload_uses_only_allowed_extension_and_preserves_bytes() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("private");
        let result = store(
            json!({"name":"../../escape.JPG","dataBase64":"AAE\r\nC/w=="}),
            &dir,
        )
        .unwrap();
        let path = Path::new(result["path"].as_str().unwrap());
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(path.extension().unwrap(), "jpg");
        assert_eq!(std::fs::read(path).unwrap(), [0, 1, 2, 255]);
        assert_eq!(result["size"], 4);
        assert!(store(json!({"name":"bad.exe","dataBase64":"YQ=="}), &dir).is_err());
        assert!(store(json!({"name":"a.png","dataBase64":"bad value"}), &dir).is_err());
        let second = store(json!({"name":"same.jpg","dataBase64":"YQ=="}), &dir).unwrap();
        assert_ne!(second["path"], result["path"]);
    }
    #[cfg(unix)]
    #[test]
    fn preexisting_shared_or_redirected_spill_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let params = json!({"name":"a.pdf","dataBase64":"YQ=="});
        assert!(store(params.clone(), &target).is_err());
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(store(params, &link).is_err());
    }
}
