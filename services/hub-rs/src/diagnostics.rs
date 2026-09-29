//! Credential-safe diagnostic text; pairing URLs intentionally use a separate path.
use std::sync::OnceLock;

pub(crate) fn credential_queries(text: &str) -> String {
    if !text.contains(['?', '&']) {
        return text.to_owned();
    }
    static QUERY: OnceLock<regex::Regex> = OnceLock::new();
    QUERY
        .get_or_init(|| {
            regex::Regex::new(
                r#"(?i)([?&](?:token|bus_?token|auth_?token|access_?token|api_?key|apikey|secret|password|passwd|pwd|key|sig|signature|internal|t)=)[^&\s"'\\<>)\]]*"#,
            )
            .expect("valid credential query pattern")
        })
        .replace_all(text, "${1}REDACTED")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_query_spellings_preserve_diagnostics_and_are_idempotent() {
        for (input, expected) in [
            (
                "ws://h:7895/bus?token=abc123",
                "ws://h:7895/bus?token=REDACTED",
            ),
            (
                "ws://h/bus?internal=k1&token=abc123",
                "ws://h/bus?internal=REDACTED&token=REDACTED",
            ),
            ("http://h:7897/mcp?t=abc123", "http://h:7897/mcp?t=REDACTED"),
            (
                "http://h/plugins/x/ui?busToken=abc123&pane=1",
                "http://h/plugins/x/ui?busToken=REDACTED&pane=1",
            ),
            ("ws://h/bus?TOKEN=abc123", "ws://h/bus?TOKEN=REDACTED"),
            (
                r#"Get "ws://h/bus?token=abc123": timeout"#,
                r#"Get "ws://h/bus?token=REDACTED": timeout"#,
            ),
            (
                "dial ws://h/bus?token=abc123 failed",
                "dial ws://h/bus?token=REDACTED failed",
            ),
            ("ws://h/bus?token=", "ws://h/bus?token=REDACTED"),
            ("ws://h:7895/bus", "ws://h:7895/bus"),
            ("http://h/x?page=2", "http://h/x?page=2"),
            ("", ""),
        ] {
            let once = credential_queries(input);
            assert_eq!(once, expected);
            assert_eq!(credential_queries(&once), once);
        }
        for key in [
            "bus_token",
            "authToken",
            "access_token",
            "apiKey",
            "secret",
            "password",
            "passwd",
            "pwd",
            "key",
            "sig",
            "signature",
        ] {
            assert_eq!(
                credential_queries(&format!("?{key}=secret&keep=1")),
                format!("?{key}=REDACTED&keep=1")
            );
        }
        let multi = credential_queries("peers: ws://a/bus?token=first, ws://b/bus?token=second");
        assert_eq!(multi.matches("REDACTED").count(), 2);
        assert!(!multi.contains("first") && !multi.contains("second"));
    }
}
