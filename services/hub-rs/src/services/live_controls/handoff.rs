//! Host-selected agent-authored brief with a read-only mechanical fallback.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{future::Future, path::Path, time::Duration};

fn instruction(target: &Path) -> String {
    format!(
        "Stop what you're doing and write a handoff brief to {} — another AI coding agent is about to take over this session and will read that file first. Create the file (markdown) with:\n1. The goal of this session, in one paragraph.\n2. State of the work: what's done and verified, what's in progress, what hasn't been started.\n3. Key files touched and why.\n4. Decisions and constraints your successor must respect (including approaches tried and rejected, and why).\n5. Gotchas or surprises you hit.\n6. The exact next step you would take.\nWrite only that file, then reply \"Handoff brief written.\" — do not continue any other work.",
        target.display()
    )
}

pub(super) async fn authored<S, F, SF, FF>(
    home: &Path,
    wait: Duration,
    poll: Duration,
    submit: S,
    fallback: F,
) -> Result<Value>
where
    S: FnOnce(String) -> SF,
    SF: Future<Output = Result<bool>>,
    F: FnOnce() -> FF,
    FF: Future<Output = Result<Value>>,
{
    ensure!(home.is_absolute(), "home directory unavailable");
    let directory = home.join(".workspacer/handoffs");
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(&directory).await?;
    let target = directory.join(format!(
        "{}-{}-agent.md",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().to_string()[..8]
    ));
    let accepted = submit(instruction(&target)).await.unwrap_or(false);
    let reason = if accepted {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            tokio::time::sleep(poll).await;
            if tokio::fs::symlink_metadata(&target)
                .await
                .is_ok_and(|m| m.is_file() && m.len() > 0)
            {
                return Ok(json!({"ok":true,"path":target}));
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
        }
        "Source agent did not write the brief before the deadline"
    } else {
        "Source agent could not accept the brief request"
    };
    Ok(match fallback().await {
        Ok(reply) => {
            json!({"ok":!super::text(&reply,"path").is_empty(),"path":reply["path"],"fallback":true,"error":reason})
        }
        Err(error) => super::failed(format!(
            "{reason}; mechanical fallback also failed: {error}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    fn target(message: &str) -> PathBuf {
        PathBuf::from(
            message
                .strip_prefix("Stop what you're doing and write a handoff brief to ")
                .unwrap()
                .split(" — another AI")
                .next()
                .unwrap(),
        )
    }
    #[tokio::test]
    async fn authored_regular_file_uses_host_path_and_preserves_instruction_sections() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let root = dir.path().to_owned();
        let result = authored(
            dir.path(),
            Duration::from_millis(25),
            Duration::from_millis(1),
            move |message| async move {
                for section in [
                    "1. The goal",
                    "2. State of the work",
                    "3. Key files",
                    "4. Decisions and constraints",
                    "5. Gotchas",
                    "6. The exact next step",
                ] {
                    assert!(message.contains(section));
                }
                let path = target(&message);
                assert_eq!(path.parent().unwrap(), root.join(".workspacer/handoffs"));
                std::fs::write(path, "# Actual authored handoff\n")?;
                Ok(true)
            },
            || async { anyhow::bail!("authored success must not invoke fallback") },
        )
        .await?;
        assert_eq!(result["ok"], true);
        assert!(result.get("fallback").is_none());
        assert_eq!(
            std::fs::read_to_string(result["path"].as_str().unwrap())?,
            "# Actual authored handoff\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root_path(dir.path()))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        Ok(())
    }
    #[cfg(unix)]
    fn root_path(home: &Path) -> PathBuf {
        home.join(".workspacer/handoffs")
    }
    #[tokio::test]
    async fn refusal_timeout_and_empty_artifact_use_one_mechanical_fallback() -> Result<()> {
        for mode in ["refused", "timeout", "empty"] {
            let dir = tempfile::tempdir()?;
            let calls = Arc::new(AtomicUsize::new(0));
            let seen = calls.clone();
            let result = authored(
                dir.path(),
                Duration::from_millis(10),
                Duration::from_millis(1),
                move |message| async move {
                    if mode == "empty" {
                        std::fs::write(target(&message), "")?;
                    }
                    Ok(mode != "refused")
                },
                move || async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({"path":"mechanical.md","markdown":"mechanical"}))
                },
            )
            .await?;
            assert_eq!(result["fallback"], true);
            assert_eq!(result["ok"], true);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                result["error"]
                    .as_str()
                    .unwrap()
                    .contains(if mode == "refused" {
                        "could not accept"
                    } else {
                        "deadline"
                    })
            );
        }
        Ok(())
    }
    #[tokio::test]
    async fn fallback_failure_and_preparation_failure_are_not_false_successes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let result = authored(
            dir.path(),
            Duration::ZERO,
            Duration::ZERO,
            |_| async { anyhow::bail!("send failed") },
            || async { anyhow::bail!("mechanical unavailable") },
        )
        .await?;
        assert_eq!(result["ok"], false);
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("mechanical fallback also failed")
        );
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, "not a directory")?;
        assert!(
            authored(
                &blocked,
                Duration::ZERO,
                Duration::ZERO,
                |_| async { panic!("must not submit") },
                || async { panic!("must not fall back") }
            )
            .await
            .is_err()
        );
        Ok(())
    }
    #[tokio::test]
    async fn cancellation_does_not_start_a_mechanical_fallback() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let home = dir.path().to_owned();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let task = tokio::spawn(async move {
            authored(
                &home,
                Duration::from_secs(150),
                Duration::from_secs(1),
                move |_| async move {
                    signal.notify_one();
                    Ok(true)
                },
                move || async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({}))
                },
            )
            .await
        });
        started.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        Ok(())
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn linked_artifact_never_satisfies_agent_completion() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let other = dir.path().join("other.md");
        std::fs::write(&other, "not authored")?;
        let result = authored(
            dir.path(),
            Duration::from_millis(10),
            Duration::from_millis(1),
            move |message| async move {
                std::os::unix::fs::symlink(other, target(&message))?;
                Ok(true)
            },
            || async { Ok(json!({"path":"fallback.md"})) },
        )
        .await?;
        assert_eq!(result["fallback"], true);
        Ok(())
    }
}
