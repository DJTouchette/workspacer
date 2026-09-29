//! Host and browser-Origin pins use the socket's actual local address, including
//! when the listener binds all interfaces. Proxy exemptions are explicit names.
use axum::{
    extract::{ConnectInfo, Request, State},
    http::HeaderMap,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{collections::BTreeSet, net::IpAddr};
#[derive(Clone, Debug)]
pub struct Policy {
    trusted: BTreeSet<String>,
    bound: IpAddr,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Socket {
    pub local: Option<IpAddr>,
}
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_>> for Socket {
    fn connect_info(stream: axum::serve::IncomingStream<'_>) -> Self {
        Self {
            local: stream.local_addr().ok().map(|addr| addr.ip()),
        }
    }
}
fn authority(raw: &str) -> Option<(String, String)> {
    let uri: axum::http::uri::Authority = raw.parse().ok()?;
    let host = uri.host().trim_matches(['[', ']']).to_ascii_lowercase();
    (!host.is_empty() && !host.contains('@')).then(|| (host, raw.to_ascii_lowercase()))
}
fn ip_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|ip| ip.is_loopback())
            .unwrap_or_else(|| ip.is_loopback()),
        IpAddr::V4(ip) => ip.is_loopback(),
    }
}
fn loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(ip_loopback)
}
impl Policy {
    pub fn new(bound: IpAddr, trusted: &[String]) -> anyhow::Result<Self> {
        let mut names = BTreeSet::new();
        for raw in trusted {
            for name in raw
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                anyhow::ensure!(
                    name != "*" && !name.contains("://"),
                    "trusted host must be an explicit hostname"
                );
                let (name, _) =
                    authority(name).ok_or_else(|| anyhow::anyhow!("invalid trusted host"))?;
                names.insert(name);
            }
        }
        Ok(Self {
            trusted: names,
            bound,
        })
    }
    pub fn local() -> Self {
        Self {
            trusted: BTreeSet::new(),
            bound: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        }
    }
    pub fn host(&self, headers: &HeaderMap, local: Option<IpAddr>) -> bool {
        let Some(value) = headers.get("host") else {
            return true;
        };
        if value.as_bytes().is_empty() {
            return true;
        }
        let Some((host, _)) = value.to_str().ok().and_then(authority) else {
            return false;
        };
        loopback(&host) || self.trusted.contains(&host) || {
            let local = local.unwrap_or(self.bound);
            host.parse::<IpAddr>().is_ok_and(|ip| ip == local) || !ip_loopback(local)
        }
    }
    pub fn origin(&self, headers: &HeaderMap, local: Option<IpAddr>) -> bool {
        let Some(value) = headers.get("origin") else {
            return true;
        };
        if value.as_bytes().is_empty() {
            return true;
        }
        let Some(_url) = value
            .to_str()
            .ok()
            .and_then(|raw| url::Url::parse(raw).ok())
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && matches!(url.path(), "" | "/")
            })
        else {
            return false;
        };
        // WHATWG URL parsing canonicalizes default ports and legacy IPv4
        // spellings. The reference gate compares the presented authorities:
        // retain those bytes instead of allowing normalization to change the
        // Host/Origin comparison or turn a non-IP name into loopback.
        let Some((host, origin_authority)) = value
            .to_str()
            .ok()
            .and_then(|raw| raw.parse::<axum::http::Uri>().ok())
            .and_then(|uri| uri.authority().and_then(|value| authority(value.as_str())))
        else {
            return false;
        };
        if loopback(&host) {
            return true;
        }
        let Some((_, request_authority)) = headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .and_then(authority)
        else {
            return false;
        };
        origin_authority == request_authority
            && (self.trusted.contains(&host) || !ip_loopback(local.unwrap_or(self.bound)))
    }
    #[cfg(test)]
    pub fn browser(&self, headers: &HeaderMap) -> bool {
        self.host(headers, None) && self.origin(headers, None)
    }
}
pub(crate) async fn guard(State(policy): State<Policy>, request: Request, next: Next) -> Response {
    let local = request
        .extensions()
        .get::<ConnectInfo<Socket>>()
        .and_then(|socket| socket.0.local);
    if !policy.host(request.headers(), local) {
        return (axum::http::StatusCode::FORBIDDEN, "host not allowed").into_response();
    }
    next.run(request).await
}
#[cfg(test)]
mod tests {
    use super::*;
    fn headers(host: &str, origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", host.parse().unwrap());
        if !origin.is_empty() {
            h.insert("origin", origin.parse().unwrap());
        }
        h
    }
    #[test]
    fn actual_loopback_socket_refuses_rebound_names_even_under_wildcard_listener() {
        let p = Policy::new("0.0.0.0".parse().unwrap(), &[]).unwrap();
        let h = headers("evil.test:7895", "http://evil.test:7895");
        assert!(!p.host(&h, Some("127.0.0.1".parse().unwrap())));
        assert!(!p.origin(&h, Some("127.0.0.1".parse().unwrap())));
        assert!(!p.host(&h, Some("::ffff:127.0.0.1".parse().unwrap())));
        assert!(!p.origin(&h, Some("::ffff:127.0.0.1".parse().unwrap())));
        assert!(p.host(&h, Some("192.168.1.10".parse().unwrap())));
        assert!(p.origin(&h, Some("192.168.1.10".parse().unwrap())));
    }
    #[test]
    fn trusted_proxy_is_exact_and_never_accepts_opaque_or_foreign_origins() {
        let p = Policy::new("127.0.0.1".parse().unwrap(), &["node.ts.net".into()]).unwrap();
        assert!(p.browser(&headers("NODE.ts.net:8443", "https://node.ts.net:8443")));
        assert!(!p.browser(&headers("evil.node.ts.net", "https://evil.node.ts.net")));
        assert!(!p.browser(&headers("node.ts.net", "null")));
        assert!(!p.browser(&headers("node.ts.net", "https://foreign.test")));
        assert!(Policy::new("127.0.0.1".parse().unwrap(), &["*".into()]).is_err());
    }
    #[test]
    fn reference_host_origin_matrix_preserves_authority_spelling_and_socket_identity() {
        let policy = Policy::new("0.0.0.0".parse().unwrap(), &[]).unwrap();
        let local = Some("127.0.0.1".parse().unwrap());
        for host in [
            "evil.example.com",
            "evil.example.com:7895",
            "attacker.tld:80",
            "127.0.0.1.evil.tld:7895",
            "10.0.0.9:7895",
        ] {
            assert!(!policy.host(&headers(host, ""), local), "{host}");
        }
        for host in [
            "127.0.0.1:7895",
            "127.0.0.1",
            "127.0.0.2:7895",
            "localhost:7895",
            "LOCALHOST:7895",
            "[::1]:7895",
            "",
        ] {
            assert!(policy.host(&headers(host, ""), local), "{host}");
        }
        for (origin, allowed) in [
            ("", true),
            ("http://127.0.0.1:7895", true),
            ("https://127.0.0.1:7895", true),
            ("http://localhost:5173", true),
            ("http://127.0.0.1:65000", true),
            ("http://evil.example.com", false),
            ("https://attacker.test", false),
            ("null", false),
            ("http://127.1", false),
            ("http://0x7f000001", false),
            ("http://2130706433", false),
        ] {
            assert_eq!(
                policy.origin(&headers("127.0.0.1:7895", origin), local),
                allowed,
                "{origin}"
            );
        }
        let public = Some("100.64.0.5".parse().unwrap());
        let same = headers(
            "myhost.tailnet.ts.net:7895",
            "http://myhost.tailnet.ts.net:7895",
        );
        assert!(policy.host(&same, public) && policy.origin(&same, public));
        assert!(!policy.origin(&same, local));
        assert!(!policy.origin(
            &headers(
                "myhost.tailnet.ts.net:7895",
                "http://evil.tailnet.ts.net:7895"
            ),
            public
        ));
        let trusted = Policy::new("127.0.0.1".parse().unwrap(), &["node.ts.net".into()]).unwrap();
        for (host, origin, allowed) in [
            ("node.ts.net:80", "http://node.ts.net:80", true),
            ("node.ts.net:443", "https://node.ts.net:443", true),
            ("NODE.ts.net:443", "https://node.ts.net:443", true),
            ("node.ts.net", "https://node.ts.net:443", false),
            ("node.ts.net:443", "https://node.ts.net", false),
            ("node.ts.net:80", "http://other.ts.net:80", false),
        ] {
            assert_eq!(
                trusted.browser(&headers(host, origin)),
                allowed,
                "{host} {origin}"
            );
        }
    }
}
