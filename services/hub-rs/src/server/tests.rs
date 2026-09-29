use super::*;
use crate::{Hub, Options};
use std::net::IpAddr;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
#[tokio::test]
async fn browser_assets_preserve_public_shell_and_operator_entry_split() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), "<main>full renderer</main>").unwrap();
    std::fs::write(web.join("app.js"), "window.fixture=true").unwrap();
    std::fs::write(root.path().join("private.txt"), "not public").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path().join("private.txt"), web.join("escape.txt")).unwrap();
    let tokens = root.path().join("tokens.json");
    let view = crate::auth::mint(&tokens, crate::auth::Scope::View, "viewer").unwrap();
    let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "operator").unwrap();
    let mut options = Options::default();
    options.token = "owner-key".into();
    options.scoped_tokens = Some(tokens.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = hub.handle();
    let server = tokio::spawn(async move {
        serve(
            listener,
            handle,
            "owner-key".into(),
            Some(tokens),
            None,
            Some(web),
            vec![],
        )
        .await
    });
    let client = reqwest::Client::new();
    let base = format!("http://{address}");
    let mobile = client.get(format!("{base}/m")).send().await.unwrap();
    assert_eq!(mobile.status(), 200);
    assert!(mobile.text().await.unwrap().contains("push.subscribe"));
    let worker = client.get(format!("{base}/sw.js")).send().await.unwrap();
    assert_eq!(worker.headers()["service-worker-allowed"], "/");
    assert_eq!(worker.headers()["cache-control"], "no-cache");
    for path in ["/remote", "/app/"] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .bearer_auth(&view.token)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .bearer_auth(&operator.token)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .get(format!("{base}{path}?token=owner-key"))
                .header("authorization", "not-bearer")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/app/app.js"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{base}/app/%2e%2e%2fprivate.txt"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    #[cfg(unix)]
    assert_eq!(
        client
            .get(format!("{base}/app/escape.txt"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{base}/m"))
            .header("host", "rebound.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    server.abort();
    let _ = server.await;
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn wildcard_listener_uses_actual_socket_for_rebinding_and_declared_proxy_exemption() {
    let mut options = Options::default();
    options.token = "owner-key".into();
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = hub.handle();
    let server = tokio::spawn(async move {
        serve(
            listener,
            handle,
            "owner-key".into(),
            None,
            None,
            None,
            vec!["node.ts.net".into()],
        )
        .await
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    assert_eq!(
        client
            .get(format!("{base}/m"))
            .header("host", "rebound.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{base}/m"))
            .header("host", "node.ts.net")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for (origin, allowed) in [
        ("https://node.ts.net", true),
        ("https://foreign.example", false),
        ("null", false),
    ] {
        let mut request = format!("ws://127.0.0.1:{port}/bus?token=owner-key")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("host", "node.ts.net".parse().unwrap());
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        let result = tokio_tungstenite::connect_async(request).await;
        assert_eq!(result.is_ok(), allowed, "{origin}");
        if let Ok((mut socket, _)) = result {
            let _ = socket.close(None).await;
        }
    }
    // Selecting a route does not send a UDP datagram. It gives us this machine's
    // actual non-loopback address, allowing the real accepted socket to be tested.
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    probe.connect("192.0.2.1:9").unwrap();
    let ip = probe.local_addr().unwrap().ip();
    assert!(matches!(ip, IpAddr::V4(_)) && !ip.is_loopback());
    assert_eq!(
        client
            .get(format!("http://{ip}:{port}/m"))
            .header("host", "lan.example")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut request = format!("ws://{ip}:{port}/bus?token=owner-key")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("host", "lan.example".parse().unwrap());
    request
        .headers_mut()
        .insert("origin", "http://lan.example".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let _ = socket.close(None).await;
    server.abort();
    let _ = server.await;
    hub.shutdown().unwrap();
}
