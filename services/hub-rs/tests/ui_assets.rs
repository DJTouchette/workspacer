use base64::Engine;
use serde_json::json;
use workspacer_hub::services::{
    files, image_preview,
    ui_assets::{Assets, family},
};
#[test]
fn managed_fonts_keep_family_identity_and_confine_view_assets() {
    let root = tempfile::tempdir().unwrap();
    let assets = Assets::new(root.path().into(), root.path().join("config"));
    assert_eq!(assets.call("ui.fonts", json!({})).unwrap(), json!([]));
    let bytes = b"wOFFfixture";
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let result = assets
        .call(
            "desktop.installUiFont",
            json!({"name":"JetBrainsMono-VariableFont[wght].woff","dataBase64":encoded}),
        )
        .unwrap();
    assert_eq!(result["family"], "JetBrainsMono");
    assert_eq!(family("Fira_Code-Regular.ttf"), "Fira Code");
    let read = assets
        .call("ui.asset", json!({"kind":"font","file":result["file"]}))
        .unwrap();
    assert_eq!(read["dataBase64"], encoded);
    assert_eq!(read["mime"], "font/woff");
    assert!(
        assets
            .call(
                "ui.asset",
                json!({"kind":"font","file":"../../tokens.json"})
            )
            .is_err()
    );
    assert!(
        assets
            .call(
                "desktop.installUiFont",
                json!({"name":"fake.ttf","dataBase64":"c2VjcmV0"})
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        let secret = root.path().join("secret.ttf");
        std::fs::write(&secret, "secret").unwrap();
        std::os::unix::fs::symlink(secret, root.path().join(".workspacer/fonts/escape.ttf"))
            .unwrap();
        assert!(
            assets
                .call("ui.asset", json!({"kind":"font","file":"escape.ttf"}))
                .is_err()
        );
    }
}
#[test]
fn bounded_download_picker_and_header_only_preview() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image.png");
    let mut png = vec![0u8; 24];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    png[12..16].copy_from_slice(b"IHDR");
    png[16..20].copy_from_slice(&320u32.to_be_bytes());
    png[20..24].copy_from_slice(&200u32.to_be_bytes());
    std::fs::write(&path, &png).unwrap();
    let preview = files::call("fs.readImage", json!({"path":path}), root.path()).unwrap();
    assert_eq!(preview["width"], 320);
    assert_eq!(preview["height"], 200);
    assert!(
        preview["dataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    png[16..20].copy_from_slice(&20000u32.to_be_bytes());
    png[20..24].copy_from_slice(&20000u32.to_be_bytes());
    std::fs::write(&path, &png).unwrap();
    assert!(files::call("fs.readImage", json!({"path":path}), root.path()).is_err());
    std::fs::write(root.path().join("note.env"), "secret").unwrap();
    assert!(
        files::call(
            "fs.readImage",
            json!({"path":root.path().join("note.env")}),
            root.path()
        )
        .is_err()
    );
    std::fs::create_dir(root.path().join("subdir")).unwrap();
    let picker = files::call("desktop.filePickerList", json!({"path":"~"}), root.path()).unwrap();
    assert_eq!(picker["entries"][0]["name"], "subdir");
    let bytes = files::call(
        "desktop.readFileBytes",
        json!({"path":root.path().join("note.env")}),
        root.path(),
    )
    .unwrap();
    assert_eq!(bytes["dataBase64"], "c2VjcmV0");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert!(files::call("desktop.readFileBytes", json!({"path":path}), root.path()).is_err());
    assert_eq!(
        image_preview::dimensions(b"GIF89a\x02\0\x03\0"),
        Some((2, 3))
    );
}
#[cfg(unix)]
#[test]
fn fifo_is_refused_before_opening() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("never.png");
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(files::call("fs.readImage", json!({"path":path}), root.path()).is_err());
    assert!(files::call("desktop.readFileBytes", json!({"path":path}), root.path()).is_err());
}
#[tokio::test]
async fn http_icon_cache_uses_response_mime_hash_and_bounded_bytes() {
    use axum::{Router, http::header, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/image.html",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png; charset=binary")],
                    b"icon-content".to_vec(),
                )
            }),
        )
        .route(
            "/bad",
            get(|| async { ([(header::CONTENT_TYPE, "text/html")], b"not-image".to_vec()) }),
        )
        .route(
            "/huge",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    vec![0u8; 2 * 1024 * 1024 + 1],
                )
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let assets = Assets::new(root.path().into(), root.path().join("config"));
    let first = assets
        .download_icon(&json!({"url":format!("http://{address}/image.html")}))
        .await
        .unwrap();
    let second = assets
        .download_icon(&json!({"url":format!("http://{address}/image.html")}))
        .await
        .unwrap();
    assert_eq!(first, second);
    assert!(first["file"].as_str().unwrap().ends_with(".png"));
    assert_eq!(
        assets
            .call("ui.asset", json!({"kind":"icon","file":first["file"]}))
            .unwrap()["dataBase64"],
        base64::engine::general_purpose::STANDARD.encode(b"icon-content")
    );
    for endpoint in ["bad", "huge"] {
        assert!(
            assets
                .download_icon(&json!({"url":format!("http://{address}/{endpoint}")}))
                .await
                .is_err()
        );
    }
    assert!(
        assets
            .download_icon(&json!({"url":"file:///etc/passwd"}))
            .await
            .is_err()
    );
    server.abort();
}

#[test]
fn header_probes_and_font_families_match_typescript_reference() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ui-assets.json")).unwrap();
    for row in fixture["fonts"].as_array().unwrap() {
        assert_eq!(
            family(row["file"].as_str().unwrap()),
            row["family"],
            "{row}"
        );
    }
    for row in fixture["headers"].as_array().unwrap() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(row["base64"].as_str().unwrap())
            .unwrap();
        let actual = image_preview::dimensions(&bytes)
            .map(|(width, height)| json!({"width":width,"height":height}))
            .unwrap_or(serde_json::Value::Null);
        assert_eq!(actual, row["dimensions"]);
    }
}

#[tokio::test]
async fn display_assets_are_viewable_but_installation_remains_owner_only() {
    use workspacer_hub::{
        Hub, Options,
        auth::{self, Scope},
        client::Client,
    };
    let root = tempfile::tempdir().unwrap();
    let tokens = root.path().join("tokens.json");
    let view = auth::mint(&tokens, Scope::View, "display-viewer").unwrap();
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "fixture-owner".into();
    options.scoped_tokens = Some(tokens);
    options.config_dir = Some(root.path().join("config"));
    options.home_dir = Some(root.path().into());
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let host = Client::connect_remote(&format!("ws://{address}/bus"), "fixture-owner")
        .await
        .unwrap();
    let params = json!({"name":"Fixture.ttf","dataBase64":base64::engine::general_purpose::STANDARD.encode(b"\0\x01\0\0font")});
    host.call("desktop.installUiFont", params.clone())
        .await
        .unwrap();
    let reader = Client::connect_remote(&format!("ws://{address}/bus"), &view.token)
        .await
        .unwrap();
    assert_eq!(
        reader.call("ui.fonts", json!({})).await.unwrap()[0]["file"],
        "Fixture.ttf"
    );
    assert!(
        reader
            .call("ui.asset", json!({"kind":"font","file":"Fixture.ttf"}))
            .await
            .is_ok()
    );
    assert!(reader.call("desktop.installUiFont", params).await.is_err());
    assert!(
        reader
            .call(
                "desktop.readFileBytes",
                json!({"path":root.path().join("tokens.json")})
            )
            .await
            .is_err()
    );
    reader.close();
    host.close();
    hub.shutdown().unwrap();
}

#[test]
fn jpeg_metadata_cannot_hide_dimensions_and_inline_limits_remain_exact() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large-metadata.JPG");
    // One legal-size APP1 segment puts SOF beyond the former64KiB probe window.
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff];
    jpeg.resize(6 + 65_533, 0);
    let frame = jpeg.len();
    jpeg.extend_from_slice(&[
        0xff, 0xc0, 0, 17, 8, 0, 0, 0, 0, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0,
    ]);
    // Complete the header through SOS, which the retained Go DecodeConfig
    // uses as its return boundary for a non-JFIF JPEG.
    jpeg.extend_from_slice(&[0xff, 0xda, 0, 12, 3, 1, 0, 2, 0, 3, 0, 0, 63, 0]);
    jpeg[frame + 5..frame + 7].copy_from_slice(&200u16.to_be_bytes());
    jpeg[frame + 7..frame + 9].copy_from_slice(&300u16.to_be_bytes());
    assert_eq!(image_preview::dimensions(&jpeg), Some((300, 200)));
    let mut filled = jpeg.clone();
    filled.splice(frame..frame, [0xff, 0xff, 0xff]);
    assert_eq!(image_preview::dimensions(&filled), Some((300, 200)));
    std::fs::write(&path, &jpeg).unwrap();
    let preview = files::call("fs.readImage", json!({"path":path}), root.path()).unwrap();
    assert_eq!(preview["width"], 300);
    assert_eq!(preview["height"], 200);
    assert_eq!(preview["size"], jpeg.len());
    assert!(
        preview["dataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    jpeg[frame + 5..frame + 7].copy_from_slice(&20_000u16.to_be_bytes());
    jpeg[frame + 7..frame + 9].copy_from_slice(&20_000u16.to_be_bytes());
    std::fs::write(&path, &jpeg).unwrap();
    assert!(
        files::call("fs.readImage", json!({"path":path}), root.path())
            .unwrap_err()
            .to_string()
            .contains("40,000,000")
    );
    let svg = root.path().join("limit.svg");
    std::fs::write(&svg, vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let preview = files::call("fs.readImage", json!({"path":svg}), root.path()).unwrap();
    assert_eq!(preview["size"], 2 * 1024 * 1024);
    assert_eq!(preview["width"], 0);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&svg)
        .unwrap()
        .set_len(2 * 1024 * 1024 + 1)
        .unwrap();
    assert!(files::call("fs.readImage", json!({"path":svg}), root.path()).is_err());
    for (extension, mime) in [
        ("PNG", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("svg", "image/svg+xml"),
        ("bmp", "image/bmp"),
        ("ico", "image/x-icon"),
        ("avif", "image/avif"),
    ] {
        assert_eq!(
            image_preview::mime(&root.path().join(format!("image.{extension}"))),
            Some(mime)
        );
    }
    for extension in ["tif", "tiff", "txt", "env", ""] {
        assert_eq!(
            image_preview::mime(&root.path().join(format!("image.{extension}"))),
            None
        );
    }
}
