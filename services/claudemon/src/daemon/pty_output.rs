//! ConPTY can retain its output pipe after the owned child has exited. Bound
//! the final drain from a positive child-exit observation, never a signal ACK.

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub(super) struct ExitDrain {
    deadline: Option<Instant>,
}

impl ExitDrain {
    pub(super) fn new() -> Self {
        Self { deadline: None }
    }

    pub(super) async fn recv(
        &mut self,
        output: &mut mpsc::UnboundedReceiver<Vec<u8>>,
        mut child_exited: impl FnMut() -> bool,
    ) -> Option<Vec<u8>> {
        loop {
            if self.deadline.is_none() && child_exited() {
                self.deadline = Some(Instant::now() + Duration::from_millis(250));
            }
            if self
                .deadline
                .is_some_and(|deadline| deadline <= Instant::now())
            {
                return None;
            }
            let wake = self
                .deadline
                .unwrap_or_else(|| Instant::now() + Duration::from_millis(20));
            tokio::select! {
                // Even a continuously readable pipe cannot extend the drain.
                biased;
                _ = tokio::time::sleep_until(wake) => {
                    if self.deadline.is_some() {
                        return None;
                    }
                }
                chunk = output.recv() => return chunk,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_pipe_drains_final_bytes_then_finishes_only_after_child_exit() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut drain = ExitDrain::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(40), drain.recv(&mut rx, || false))
                .await
                .is_err()
        );
        tx.send(b"before exit".to_vec()).unwrap();
        assert_eq!(drain.recv(&mut rx, || false).await.unwrap(), b"before exit");
        tx.send(b"final output".to_vec()).unwrap();
        assert_eq!(drain.recv(&mut rx, || true).await.unwrap(), b"final output");
        assert!(
            tokio::time::timeout(Duration::from_secs(1), drain.recv(&mut rx, || true))
                .await
                .unwrap()
                .is_none()
        );
        // This sender deliberately remains alive: EOF is not the exit proof.
        assert!(!tx.is_closed());
    }

    #[tokio::test]
    async fn final_output_cannot_reset_the_exit_deadline_and_eof_still_finishes() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut drain = ExitDrain::new();
        tx.send(vec![1]).unwrap();
        assert_eq!(drain.recv(&mut rx, || true).await, Some(vec![1]));
        let deadline = drain.deadline.unwrap();
        tx.send(vec![2]).unwrap();
        assert_eq!(drain.recv(&mut rx, || true).await, Some(vec![2]));
        assert_eq!(drain.deadline, Some(deadline));
        drain.deadline = Some(Instant::now());
        tx.send(vec![3]).unwrap();
        assert!(drain.recv(&mut rx, || true).await.is_none());
        drop(tx);
        let mut live = ExitDrain::new();
        assert_eq!(live.recv(&mut rx, || false).await, Some(vec![3]));
        assert!(live.recv(&mut rx, || false).await.is_none());
        assert!(live.deadline.is_none());
    }
}
