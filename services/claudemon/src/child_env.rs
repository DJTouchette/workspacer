//! Workspacer host authority belongs to the embedding host, not provider CLIs.
//! Keep provider account variables and explicitly scoped facade configuration;
//! never mutate the process-wide environment while admitting concurrent sessions.
use std::{ffi::OsStr, process::Command};
pub const HOST_AUTHORITY_KEYS: &[&str] = &["HUB_TOKEN", "WKS_MCP_TOKEN", "WKS_MCP_HUB_TOKEN"];
pub fn is_host_authority(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    HOST_AUTHORITY_KEYS
        .iter()
        .any(|expected| key.eq_ignore_ascii_case(expected))
}
pub trait SanitizeChildEnvironment {
    fn scrub_host_authority(&mut self) -> &mut Self;
}
impl SanitizeChildEnvironment for Command {
    fn scrub_host_authority(&mut self) -> &mut Self {
        let names = std::env::vars_os()
            .map(|(key, _)| key)
            .chain(self.get_envs().map(|(key, _)| key.to_owned()))
            .filter(|key| is_host_authority(key))
            .collect::<Vec<_>>();
        for key in names {
            self.env_remove(key);
        }
        for key in HOST_AUTHORITY_KEYS {
            self.env_remove(key);
        }
        self
    }
}
impl SanitizeChildEnvironment for tokio::process::Command {
    fn scrub_host_authority(&mut self) -> &mut Self {
        self.as_std_mut().scrub_host_authority();
        self
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{collections::HashMap, io::Read};
    fn verify(bytes: &[u8]) {
        let output = String::from_utf8_lossy(bytes);
        let env = output
            .lines()
            .filter_map(|line| line.trim_end_matches('\r').split_once('='))
            .collect::<HashMap<_, _>>();
        assert!(!env.contains_key("HUB_TOKEN"));
        assert!(!env.contains_key("WKS_MCP_TOKEN"));
        assert!(!env.contains_key("WKS_MCP_HUB_TOKEN"));
        assert!(!env.contains_key("hub_token"));
        assert_eq!(env.get("COPILOT_GITHUB_TOKEN"), Some(&"provider-account"));
        assert_eq!(
            env.get("SESSION_MCP_URL"),
            Some(&"http://127.0.0.1/mcp?t=session-scoped")
        );
    }
    #[test]
    fn isolated_environment_fixture() {
        if std::env::var("WKS_CHILD_ENV_FIXTURE").as_deref() != Ok("1") {
            return;
        }
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "env"])
            .env("HUB_TOKEN", "overlay-owner")
            .env("hub_token", "lowercase-overlay")
            .env("WKS_MCP_HUB_TOKEN", "overlay-upstream-facade")
            .env("COPILOT_GITHUB_TOKEN", "provider-account")
            .env("SESSION_MCP_URL", "http://127.0.0.1/mcp?t=session-scoped");
        let output = command.scrub_host_authority().output().unwrap();
        assert!(output.status.success());
        verify(&output.stdout);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let output = tokio::process::Command::new("/bin/sh")
                .args(["-c", "env"])
                .env("COPILOT_GITHUB_TOKEN", "provider-account")
                .env("SESSION_MCP_URL", "http://127.0.0.1/mcp?t=session-scoped")
                .env("HUB_TOKEN", "overlay-owner")
                .scrub_host_authority()
                .output()
                .await
                .unwrap();
            verify(&output.stdout);
        });
        let extra = HashMap::from([
            ("HUB_TOKEN".into(), "overlay-owner".into()),
            ("WKS_MCP_HUB_TOKEN".into(), "overlay-upstream-facade".into()),
            ("COPILOT_GITHUB_TOKEN".into(), "provider-account".into()),
            (
                "SESSION_MCP_URL".into(),
                "http://127.0.0.1/mcp?t=session-scoped".into(),
            ),
        ]);
        let handle = crate::wrapper::pty::spawn(
            &["/bin/sh".into(), "-c".into(), "env".into()],
            std::env::temp_dir().to_str().unwrap(),
            portable_pty::PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            },
            &extra,
        )
        .unwrap();
        let mut reader = handle.master.lock().unwrap().try_clone_reader().unwrap();
        let mut bytes = Vec::new();
        let _ = reader.by_ref().take(1024 * 1024).read_to_end(&mut bytes);
        handle.child.lock().unwrap().wait().unwrap();
        verify(&bytes);
        assert!(String::from_utf8_lossy(&bytes)
            .lines()
            .any(|line| line.starts_with("SHELL=")));
        assert_eq!(std::env::var("HUB_TOKEN").unwrap(), "parent-owner");
        assert_eq!(std::env::var("WKS_MCP_TOKEN").unwrap(), "parent-mcp-owner");
        assert_eq!(
            std::env::var("WKS_MCP_HUB_TOKEN").unwrap(),
            "parent-upstream-facade"
        );
    }
    #[test]
    fn provider_children_cannot_inherit_host_bearers() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child_env::tests::isolated_environment_fixture",
                "--nocapture",
            ])
            .env("WKS_CHILD_ENV_FIXTURE", "1")
            .env_remove("SHELL")
            .env("HUB_TOKEN", "parent-owner")
            .env("WKS_MCP_TOKEN", "parent-mcp-owner")
            .env("WKS_MCP_HUB_TOKEN", "parent-upstream-facade")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated child environment fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
