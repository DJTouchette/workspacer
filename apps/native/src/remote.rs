//! Remote access over Tailscale: the hub is served at `https://<node>.ts.net`
//! by Tailscale Serve, and phones pair with scoped, revocable tokens. Every
//! change runs on the connected hub as its owner; nothing here shells out.
use crate::backend::Backend;
use anyhow::{Result, ensure};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `tailscale serve` the hub over HTTPS, or reset Serve.
    Serve(bool),
    /// Create (or reuse) the pairing token for one scope.
    Pair(String),
    Revoke(String),
}

/// Pairing scopes, least to most access, with the label and the consequence
/// shown beside the QR code.
pub const SCOPES: [(&str, &str, &str); 3] = [
    (
        "view",
        "Read-only",
        "Can watch sessions and transcripts, but cannot control agents.",
    ),
    (
        "triage",
        "Triage",
        "Can approve, answer, chat and interrupt. Cannot start agents, open terminals or change Git.",
    ),
    (
        "operator",
        "Full control",
        "Full agent control, including starting agents and terminals. Anyone with this link controls this machine.",
    ),
];

pub const TAILSCALE_DOWNLOAD: &str = "https://tailscale.com/download";

/// The hub labels pairings this way and only lists, reuses and revokes
/// records labelled like this.
fn pairing_label(scope: &str) -> String {
    format!("Remote Control: {scope}")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    pub token: String,
    pub scope: String,
    pub created: String,
}

/// What the Remote settings show, read from [`state`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    /// Tailscale is installed, running and signed in on the hub's machine.
    pub available: bool,
    /// This node's MagicDNS name, e.g. `host.tailnet.ts.net`.
    pub magic_name: String,
    /// Tailscale Serve currently forwards HTTPS to the hub.
    pub serving: bool,
    pub can_serve: bool,
    /// The one-time fix when Serve cannot be changed yet.
    pub hint: String,
    /// The hub refused or could not read Tailscale status.
    pub error: String,
    pub can_manage_tokens: bool,
    pub pairings: Vec<Pairing>,
}

impl Remote {
    pub fn parse(value: &Value) -> Self {
        let tailscale = &value["tailscale"];
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        let pairings = value["tokens"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|record| {
                let scope = record["scope"].as_str().unwrap_or_default();
                record["label"].as_str() == Some(pairing_label(scope).as_str())
            })
            .map(|record| Pairing {
                token: text(&record["token"]),
                scope: text(&record["scope"]),
                created: text(&record["created"]),
            })
            .filter(|pairing| !pairing.token.is_empty())
            .collect();
        Self {
            available: tailscale["available"] == true,
            magic_name: text(&tailscale["magicName"]),
            serving: tailscale["serveActive"] == true,
            can_serve: tailscale["canServe"] == true,
            hint: text(&tailscale["hint"]),
            error: text(&value["tailscaleError"]),
            can_manage_tokens: value["pairing"]["canManageTokens"] == true,
            pairings,
        }
    }
    /// HTTPS is live, so a phone can open the hub by name.
    pub fn reachable(&self) -> bool {
        self.available && self.serving && !self.magic_name.is_empty()
    }
    pub fn pairing(&self, scope: &str) -> Option<&Pairing> {
        self.pairings.iter().find(|pairing| pairing.scope == scope)
    }
    /// The phone client link for a scope's pairing, once HTTPS is live.
    pub fn phone_url(&self, scope: &str) -> Option<String> {
        let pairing = self.pairing(scope)?;
        self.reachable()
            .then(|| phone_url(&self.magic_name, &pairing.token))
            .filter(|url| !url.is_empty())
    }
}

/// `https://<node>/m?token=…`, or empty when the name is not a valid host.
pub fn phone_url(magic_name: &str, token: &str) -> String {
    let mut url = url::Url::parse("https://localhost/m").expect("static URL");
    if url.set_host(Some(magic_name)).is_err() {
        return String::new();
    }
    url.query_pairs_mut().append_pair("token", token);
    url.into()
}

/// A Tailscale admin link inside a hint, so it can be a button.
pub fn hint_link(hint: &str) -> Option<&str> {
    hint.split_whitespace()
        .find(|word| word.starts_with("https://"))
        .map(|word| word.trim_end_matches(['.', ',', ')']))
}

pub fn scope_label(scope: &str) -> &'static str {
    SCOPES
        .iter()
        .find(|(id, ..)| *id == scope)
        .map(|(_, label, _)| *label)
        .unwrap_or("Unknown access")
}

pub async fn state(backend: &Backend) -> Result<Value> {
    let (tailscale, pairing) = tokio::join!(
        backend.call("remote.tailscaleInfo", json!({})),
        backend.call("remote.pairingInfo", json!({}))
    );
    let pairing = pairing?;
    let tokens = if pairing["canManageTokens"] == true {
        backend.call("remote.tokensList", json!({})).await?
    } else {
        json!([])
    };
    Ok(match tailscale {
        Ok(tailscale) => json!({"tailscale":tailscale,"pairing":pairing,"tokens":tokens}),
        Err(error) => json!({
            "tailscale":null,"tailscaleError":error.to_string(),
            "pairing":pairing,"tokens":tokens
        }),
    })
}

/// Apply one owner change, then answer with the refreshed [`state`].
pub async fn apply(backend: &Backend, action: &Action) -> Result<Value> {
    match action {
        Action::Serve(enabled) => {
            backend
                .call("remote.tailscaleServe", json!({"enabled":enabled}))
                .await?
        }
        Action::Pair(scope) => {
            ensure!(
                SCOPES.iter().any(|(id, ..)| id == scope),
                "Unknown pairing scope"
            );
            backend
                .call(
                    "remote.tokenGetOrCreate",
                    json!({"scope":scope,"label":pairing_label(scope)}),
                )
                .await?
        }
        Action::Revoke(token) => {
            backend
                .call("remote.tokenRevoke", json!({"token":token}))
                .await?
        }
    };
    state(backend).await
}

/// A QR code for `text` as rows of dark modules, without a quiet zone.
pub fn qr_modules(text: &str) -> Option<Vec<Vec<bool>>> {
    let code = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).ok()?;
    let size = code.size();
    Some(
        (0..size)
            .map(|y| (0..size).map(|x| code.get_module(x, y)).collect())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(serving: bool) -> Value {
        json!({
            "tailscale":{"available":true,"magicName":"node.tailnet.ts.net","serveActive":serving,"canServe":true},
            "pairing":{"scope":"operator","canManageTokens":true},
            "tokens":[
                {"token":"t-view","scope":"view","label":"Remote Control: view","created":"2026-10-02T10:00:00Z"},
                {"token":"t-mislabelled","scope":"triage","label":"Remote Control: operator"},
                {"token":"t-node","scope":"provider","label":"Node: fly"}
            ]
        })
    }

    #[test]
    fn only_hub_labelled_pairings_are_listed() {
        let remote = Remote::parse(&fixture(true));
        assert_eq!(remote.pairings.len(), 1);
        assert_eq!(remote.pairing("view").unwrap().token, "t-view");
        assert!(remote.pairing("triage").is_none());
        assert!(remote.can_manage_tokens);
    }

    #[test]
    fn phone_links_need_live_https_and_encode_the_token() {
        let serving = Remote::parse(&fixture(true));
        assert_eq!(
            serving.phone_url("view").as_deref(),
            Some("https://node.tailnet.ts.net/m?token=t-view")
        );
        assert_eq!(serving.phone_url("operator"), None);
        assert_eq!(Remote::parse(&fixture(false)).phone_url("view"), None);
        assert_eq!(
            phone_url("node.ts.net", "a+b/c"),
            "https://node.ts.net/m?token=a%2Bb%2Fc"
        );
        assert_eq!(phone_url("bad host/", "x"), "");
    }

    #[test]
    fn refused_status_is_reported_not_treated_as_missing_tailscale() {
        let remote = Remote::parse(&json!({
            "tailscale":null,"tailscaleError":"remote.tailscaleInfo requires the server owner",
            "pairing":{"canManageTokens":false},"tokens":[]
        }));
        assert!(!remote.available);
        assert!(remote.error.contains("server owner"));
    }

    #[test]
    fn hint_links_drop_trailing_punctuation() {
        assert_eq!(
            hint_link(
                "Enable Tailscale Serve for your tailnet once: https://login.tailscale.com/f/serve?node=n1."
            ),
            Some("https://login.tailscale.com/f/serve?node=n1")
        );
        assert_eq!(hint_link("Run sudo tailscale set --operator=$USER"), None);
    }

    #[test]
    fn qr_codes_are_square() {
        let modules = qr_modules("https://node.tailnet.ts.net/m?token=abc").unwrap();
        assert!(modules.len() >= 21);
        assert!(modules.iter().all(|row| row.len() == modules.len()));
        assert!(modules.iter().flatten().any(|dark| *dark));
    }
}
