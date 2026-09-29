//! Listener addresses, local dial targets and advertised client URLs are
//! different values. Never hand a wildcard listener to a child as its URL.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
/// Go hub busDialAddr semantics: preserve concrete addresses and the port,
/// including IPv6 scope information; wildcard listeners use their own family.
pub fn dial_addr(bind: SocketAddr) -> SocketAddr {
    if !bind.ip().is_unspecified() {
        return bind;
    }
    SocketAddr::new(
        match bind.ip() {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        },
        bind.port(),
    )
}
/// Launcher advertisement policy over host-owned interface candidates. This is
/// deliberately separate from internal dialing: prefer Tailscale CGNAT, then
/// the first non-loopback IPv4, retaining any concrete bind unchanged.
pub fn advertise_addr(bind: SocketAddr, ipv4s: &[Ipv4Addr]) -> SocketAddr {
    if !bind.ip().is_unspecified() {
        return bind;
    }
    let candidates: Vec<_> = ipv4s
        .iter()
        .copied()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect();
    let chosen = candidates
        .iter()
        .copied()
        .find(|ip| {
            let bytes = ip.octets();
            bytes[0] == 100 && (64..128).contains(&bytes[1])
        })
        .or_else(|| candidates.first().copied());
    chosen
        .map(|ip| SocketAddr::new(ip.into(), bind.port()))
        .unwrap_or_else(|| dial_addr(bind))
}
/// Enumeration only; no route probes, remote network requests or process calls.
pub fn local_ipv4s() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
            _ => None,
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn internal_targets_preserve_family_concrete_scope_and_port() {
        for (bind, want) in [
            ("0.0.0.0:7895", "127.0.0.1:7895"),
            ("[::]:7895", "[::1]:7895"),
            ("100.86.79.73:7895", "100.86.79.73:7895"),
            ("[2001:db8::2]:7895", "[2001:db8::2]:7895"),
            ("127.0.0.1:0", "127.0.0.1:0"),
        ] {
            assert_eq!(
                dial_addr(bind.parse().unwrap()),
                want.parse::<SocketAddr>().unwrap()
            );
        }
        let scoped = SocketAddr::V6(std::net::SocketAddrV6::new(
            "fe80::123".parse().unwrap(),
            8080,
            4,
            7,
        ));
        assert_eq!(dial_addr(scoped), scoped);
    }
    #[test]
    fn advertised_hosts_match_launcher_cgnat_boundaries_and_keep_concrete_binds() {
        for (bind, ips, want) in [
            ("192.168.1.5:7895", vec!["100.100.0.1"], "192.168.1.5:7895"),
            ("127.0.0.1:7895", vec!["100.100.0.1"], "127.0.0.1:7895"),
            (
                "0.0.0.0:7895",
                vec!["192.168.1.5", "100.100.0.1"],
                "100.100.0.1:7895",
            ),
            (
                "0.0.0.0:7895",
                vec!["100.0.0.1", "192.168.1.5"],
                "100.0.0.1:7895",
            ),
            (
                "0.0.0.0:7895",
                vec!["192.168.1.5", "10.0.0.2"],
                "192.168.1.5:7895",
            ),
            ("0.0.0.0:7895", vec![], "127.0.0.1:7895"),
            ("[::]:7895", vec!["100.99.1.1"], "100.99.1.1:7895"),
            ("[::]:7895", vec![], "[::1]:7895"),
            (
                "0.0.0.0:7895",
                vec!["10.0.0.1", "100.63.255.255", "100.128.0.0"],
                "10.0.0.1:7895",
            ),
        ] {
            let ips = ips
                .into_iter()
                .map(|ip| ip.parse().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                advertise_addr(bind.parse().unwrap(), &ips),
                want.parse::<SocketAddr>().unwrap()
            );
        }
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use crate::{Hub, Options, client::Client, protocol::Frame};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use std::time::Duration;
    #[test]
    fn wildcard_plugin_child() {
        let settings = std::env::var("WKS_SETTINGS")
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or(Value::Null);
        if settings["probe"] != "wildcard-address-fixture" {
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let target = std::env::var("HUB_URL").unwrap();
                let mut url = url::Url::parse(&target).unwrap();
                let ip = match url.host().unwrap() {
                    url::Host::Ipv4(ip) => IpAddr::V4(ip),
                    url::Host::Ipv6(ip) => IpAddr::V6(ip),
                    _ => panic!("fixture requires concrete IP"),
                };
                assert!(ip.is_loopback());
                url.query_pairs_mut()
                    .append_pair("token", &std::env::var("HUB_TOKEN").unwrap());
                let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
                    .await
                    .unwrap();
                let hello = socket.next().await.unwrap().unwrap();
                assert!(hello.is_text());
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&Frame {
                            methods: vec!["wildcard.echo".into()],
                            ..Frame::op("register")
                        })
                        .unwrap(),
                    ))
                    .await
                    .unwrap();
                while let Some(message) = socket.next().await {
                    let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                        continue;
                    };
                    let frame: Frame = serde_json::from_str(&text).unwrap();
                    if frame.method == "wildcard.echo" {
                        socket
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::to_string(&Frame {
                                    id: frame.id,
                                    result: Some(json!({"url":target})),
                                    ..Frame::op("result")
                                })
                                .unwrap(),
                            ))
                            .await
                            .unwrap();
                    }
                }
            });
    }
    #[tokio::test]
    async fn actual_plugin_registers_and_answers_through_wildcard_listener() {
        for bind in ["0.0.0.0:0", "[::]:0"] {
            if bind.starts_with('[') && std::net::TcpListener::bind(bind).is_err() {
                eprintln!("IPv6 unavailable; wildcard IPv6 socket fixture skipped");
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            let plugin = directory.path().join("wildcard");
            std::fs::create_dir(&plugin).unwrap();
            std::fs::write(plugin.join("plugin.json"),serde_json::to_vec(&json!({"id":"wildcard","apiVersion":"1","provides":["wildcard.echo"],"settings":[{"key":"probe","type":"string","default":"wildcard-address-fixture"}],"server":{"command":std::env::current_exe().unwrap(),"args":["--exact","net_address::socket_tests::wildcard_plugin_child","--nocapture"]}})).unwrap()).unwrap();
            let mut options = Options::default();
            options.listen = Some(bind.parse().unwrap());
            options.token = "fixture-host".into();
            options.plugins_dir = Some(directory.path().into());
            let hub = Hub::start(options).unwrap();
            let bound = hub.ready().await.unwrap().unwrap();
            let handle = hub.handle();
            let result = tokio::time::timeout(Duration::from_secs(8), async {
                while !handle.health().await.unwrap()["methodNames"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("wildcard.echo"))
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Client::connect(&handle)
                    .await
                    .unwrap()
                    .call("wildcard.echo", json!({}))
                    .await
                    .unwrap()
            })
            .await;
            tokio::task::spawn_blocking(move || hub.shutdown())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                result.expect("plugin never registered through its owned bus URL"),
                json!({"url":format!("ws://{}/bus",dial_addr(bound))})
            );
        }
    }
}
