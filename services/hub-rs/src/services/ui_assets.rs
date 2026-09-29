//! Managed display assets are distinct from arbitrary filesystem reads: ui.* is
//! view-scoped, whereas installation remains an authenticated owner operation.
use super::{config::atomic_bytes, files::bounded_bytes, paths};
use crate::Options;
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
const FONT_LIMIT: usize = 12 * 1024 * 1024;
fn font(name: &str) -> bool {
    regex::Regex::new(r"(?i)\.(ttf|otf|woff2?)$")
        .unwrap()
        .is_match(name)
}
fn basename(value: &Value) -> Result<&str> {
    let name = value
        .as_str()
        .ok_or_else(|| anyhow!("Asset file must be one filename"))?;
    if name.is_empty()
        || name.encode_utf16().count() > 200
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0'])
    {
        bail!("Asset file must be one filename");
    }
    Ok(name)
}
pub fn family(name: &str) -> String {
    let ext = regex::Regex::new(r"(?i)\.(ttf|otf|woff2?)$")
        .unwrap()
        .replace(name, "");
    let axes = regex::Regex::new(r"\[[^\]]*\]")
        .unwrap()
        .replace_all(&ext, "");
    let style = regex::Regex::new(r"(?i)[-_. ]?(VariableFont[^.]*|Variable|Regular|VF)$")
        .unwrap()
        .replace(&axes, "");
    let display = regex::Regex::new(r"[-_.]+")
        .unwrap()
        .replace_all(&style, " ");
    let display = super::dispatch_templates::trim_js(&display);
    if display.is_empty() {
        name.into()
    } else {
        display.into()
    }
}
pub struct Assets {
    fonts: PathBuf,
    icons: PathBuf,
}
impl Assets {
    pub fn new(home: PathBuf, config: PathBuf) -> Self {
        Self {
            fonts: home.join(".workspacer/fonts"),
            icons: config.join("project-icons"),
        }
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "ui.fonts" => {
                let entries = match std::fs::read_dir(&self.fonts) {
                    Ok(entries) => entries,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!([])),
                    Err(e) => return Err(e.into()),
                };
                let mut names = Vec::new();
                for e in entries {
                    let name = e?.file_name().to_string_lossy().into_owned();
                    if font(&name) {
                        names.push(name);
                    }
                }
                names.sort();
                Ok(json!(
                    names
                        .into_iter()
                        .map(|file| json!({"family":family(&file),"file":file}))
                        .collect::<Vec<_>>()
                ))
            }
            "ui.asset" => {
                let name = basename(&params["file"])?;
                let kind = params["kind"].as_str().unwrap_or("");
                let (root, limit) = match kind {
                    "font" if font(name) => (&self.fonts, FONT_LIMIT),
                    "icon"
                        if regex::Regex::new(
                            r"^[a-f0-9]{32}\.(png|jpg|gif|webp|svg|ico|avif)$",
                        )
                        .unwrap()
                        .is_match(name) =>
                    {
                        (&self.icons, 2 * 1024 * 1024)
                    }
                    _ => bail!("Unknown UI asset kind or invalid filename"),
                };
                let path = paths::selected_path(root, name)?;
                let bytes = bounded_bytes(&path, limit)?;
                let mime = if kind == "font" {
                    format!(
                        "font/{}",
                        Path::new(name)
                            .extension()
                            .unwrap()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                    )
                } else {
                    super::image_preview::mime(Path::new(name)).unwrap().into()
                };
                let mut out = json!({"dataBase64":base64::engine::general_purpose::STANDARD.encode(bytes),"mime":mime});
                if kind == "font" {
                    out["family"] = json!(family(name));
                }
                Ok(out)
            }
            "desktop.installUiFont" => {
                let file = basename(&params["name"])?;
                let data = params["dataBase64"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Choose a font file up to 12 MiB"))?;
                if !font(file) || data.len() > FONT_LIMIT * 4 / 3 + 4 {
                    bail!("Choose a font file up to 12 MiB");
                }
                let clean: Vec<_> = data
                    .bytes()
                    .filter_map(|b| match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => Some(b),
                        b'-' => Some(b'+'),
                        b'_' => Some(b'/'),
                        _ => None,
                    })
                    .collect();
                let decoder = base64::engine::general_purpose::GeneralPurpose::new(
                    &base64::alphabet::STANDARD,
                    base64::engine::general_purpose::GeneralPurposeConfig::new()
                        .with_decode_allow_trailing_bits(true)
                        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
                );
                let bytes = decoder.decode(clean)?;
                if bytes.len() > FONT_LIMIT
                    || bytes.len() < 4
                    || ![
                        b"\0\x01\0\0".as_slice(),
                        b"OTTO",
                        b"true",
                        b"typ1",
                        b"wOFF",
                        b"wOF2",
                    ]
                    .contains(&&bytes[..4])
                {
                    bail!("The file is not a supported font");
                }
                std::fs::create_dir_all(&self.fonts)?;
                let target = paths::selected_path(&self.fonts, file)?;
                atomic_bytes(&target, &bytes)?;
                Ok(json!({"file":file,"family":family(file)}))
            }
            _ => bail!("unknown UI asset method"),
        }
    }
    pub async fn download_icon(&self, input: &Value) -> Result<Value> {
        let raw = input["url"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid icon URL"))?;
        let url = url::Url::parse(raw.trim())?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("Only http and https URLs can be downloaded");
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::limited(20))
            .build()?;
        let mut response = client
            .get(url)
            .header("Accept", "image/*")
            .send()
            .await?
            .error_for_status()?;
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let extension = match mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
            "image/avif" => "avif",
            _ => bail!("That is not an image type we can use"),
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                bail!("That image is larger than 2 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            bail!("The file was empty");
        }
        let file = format!(
            "{}.{}",
            &format!("{:x}", Sha256::digest(&bytes))[..32],
            extension
        );
        let root = self.icons.clone();
        let stored = file.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&root)?;
            let target = paths::selected_path(&root, &stored)?;
            if target.exists() {
                if bounded_bytes(&target, 2 * 1024 * 1024)? != bytes {
                    bail!("cached icon contents differ from their content-addressed filename");
                }
            } else {
                atomic_bytes(&target, &bytes)?;
            }
            Ok(())
        })
        .await??;
        Ok(json!({"ok":true,"file":file}))
    }
}
pub(crate) fn install(mut options: Options, home: PathBuf, config: PathBuf) -> Options {
    let service = Arc::new(Assets::new(home, config));
    for method in [
        "ui.fonts",
        "ui.asset",
        "desktop.installUiFont",
        "desktop.downloadProjectIcon",
    ] {
        let service = service.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move {
                if method == "desktop.downloadProjectIcon" {
                    service.download_icon(&params).await
                } else {
                    tokio::task::spawn_blocking(move || service.call(method, params)).await?
                }
            }
        });
    }
    options
}
