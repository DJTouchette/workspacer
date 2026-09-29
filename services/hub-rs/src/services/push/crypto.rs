//! RFC8291 payload encryption uses RustCrypto through web-push-native. VAPID
//! claims are composed here so a non-default HTTPS port stays in the audience.
use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use web_push_native::{
    Auth, WebPushBuilder,
    jwt_simple::{
        algorithms::{ECDSAP256KeyPairLike, ES256KeyPair},
        claims::Claims,
    },
    p256::{PublicKey, elliptic_curve::sec1::ToEncodedPoint},
};
const SUBJECT: &str = "https://github.com/DJTouchette/workspacer";
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Keys {
    pub public_key: String,
    pub private_key: String,
}
impl Keys {
    pub fn generate() -> Self {
        let key = ES256KeyPair::generate();
        let bytes = key.to_bytes();
        let secret =
            web_push_native::p256::SecretKey::from_slice(&bytes).expect("generated P256 secret");
        Self {
            public_key: URL_SAFE_NO_PAD
                .encode(secret.public_key().to_encoded_point(false).as_bytes()),
            private_key: URL_SAFE_NO_PAD.encode(bytes),
        }
    }
    pub fn validate(&self) -> Result<()> {
        let secret = web_push_native::p256::SecretKey::from_slice(
            &URL_SAFE_NO_PAD.decode(&self.private_key)?,
        )
        .map_err(|_| anyhow!("invalid VAPID private key"))?;
        anyhow::ensure!(
            URL_SAFE_NO_PAD.decode(&self.public_key)?
                == secret.public_key().to_encoded_point(false).as_bytes(),
            "VAPID public/private key mismatch"
        );
        Ok(())
    }
}
pub(super) fn endpoint(endpoint: &str) -> Result<url::Url> {
    let url = url::Url::parse(endpoint)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "push endpoint must be HTTPS without URL credentials or fragment"
    );
    if let Some(host) = url.host_str() {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
            anyhow::ensure!(
                public_ip(ip),
                "push endpoint must not be a private, loopback, link-local or non-public IP"
            );
        }
    }
    Ok(url)
}
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_unspecified()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_broadcast()
        }
        std::net::IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(v4.into());
            }
            let octets = ip.octets();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && (octets[0] & 0xfe) != 0xfc
                && !(octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80)
                && octets[..12].iter().any(|byte| *byte != 0)
        }
    }
}
pub(super) fn subscription_keys(keys: &super::SubscriptionKeys) -> Result<(PublicKey, Auth)> {
    anyhow::ensure!(
        keys.p256dh.len() <= 128 && keys.auth.len() <= 64,
        "push subscription keys exceed allowed length"
    );
    let public =
        PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(keys.p256dh.trim_end_matches('='))?)
            .map_err(|_| anyhow!("invalid push subscription public key"))?;
    let auth = URL_SAFE_NO_PAD.decode(keys.auth.trim_end_matches('='))?;
    anyhow::ensure!(
        auth.len() == 16,
        "push subscription auth must contain16 bytes"
    );
    Ok((public, Auth::clone_from_slice(&auth)))
}
pub(super) fn request(
    keys: &Keys,
    subscription: &super::Subscription,
    body: Vec<u8>,
) -> Result<axum::http::Request<Vec<u8>>> {
    let url = endpoint(&subscription.endpoint)?;
    let (public, auth) = subscription_keys(&subscription.keys)?;
    let mut request = WebPushBuilder::new(subscription.endpoint.parse()?, public, auth)
        .with_valid_duration(Duration::from_secs(60))
        .build(body)
        .map_err(|error| anyhow!("push encryption failed: {error}"))?;
    let key = ES256KeyPair::from_bytes(&URL_SAFE_NO_PAD.decode(&keys.private_key)?)
        .map_err(|_| anyhow!("invalid VAPID key"))?;
    let claims = Claims::create(Duration::from_secs(12 * 60 * 60).into())
        .with_audience(url.origin().ascii_serialization())
        .with_subject(SUBJECT);
    let token = key
        .sign(claims)
        .map_err(|_| anyhow!("VAPID signing failed"))?;
    request.headers_mut().insert(
        "authorization",
        format!("vapid t={token}, k={}", keys.public_key).parse()?,
    );
    request.headers_mut().insert("urgency", "high".parse()?);
    Ok(request)
}
