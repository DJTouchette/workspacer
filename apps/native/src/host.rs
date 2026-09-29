//! Lifetime owner for the native backend. GPUI never enters this Tokio runtime.
use crate::{
    backend::Backend,
    bus::Config,
    controller::{Command, Controller, View},
};
use anyhow::{Context, Result, anyhow};
use std::{sync::Arc, thread::JoinHandle};
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(feature = "rust-hub")]
mod rust_local;
#[cfg(feature = "rust-hub")]
pub use rust_local::RustOptions;

pub enum Mode {
    Remote(Config),
    Demo,
    #[cfg(feature = "rust-hub")]
    Rust(RustOptions),
}

#[derive(Clone, Debug)]
pub struct Ready {
    pub bus_url: String,
    pub engine_api: Option<String>,
    pub engine_hook: Option<String>,
    /// Actual owned bind receipts, used to verify joined listener shutdown.
    pub owned_listeners: Vec<std::net::SocketAddr>,
}

#[derive(Clone, Debug)]
pub enum Status {
    Starting,
    Ready(Ready),
    Failed(String),
    Stopped,
}

/// Probe the addresses reported by this owner after its joined shutdown.
/// Reuse-address handles TCP TIME_WAIT, but only after a connection attempt
/// confirms there is no listening service to accidentally share on Windows.
pub async fn verify_owned_listeners_released(addresses: &[std::net::SocketAddr]) -> Result<()> {
    use tokio::net::{TcpSocket, TcpStream};
    for address in addresses {
        // Winsock can take over one second to report refusal on a closed
        // loopback port. Keep a bounded budget without equating a timeout with
        // refusal, and do not attempt rebind until absence is actually proved.
        verify_connection_refused(*address, std::time::Duration::from_secs(5), async {
            TcpStream::connect(address).await.map(|_| ())
        })
        .await?;
        let socket = if address.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        socket.set_reuseaddr(true)?;
        socket.bind(*address).with_context(|| {
            format!("Owned listener could not be rebound after shutdown: {address}")
        })?;
        drop(socket.listen(1)?);
    }
    Ok(())
}

async fn verify_connection_refused(
    address: std::net::SocketAddr,
    budget: std::time::Duration,
    connect: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    match tokio::time::timeout(budget, connect).await {
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(Ok(())) => {
            anyhow::bail!("An owned listener remains reachable after shutdown: {address}")
        }
        Ok(Err(error)) => anyhow::bail!(
            "Cannot verify owned listener shutdown at {address}: connect failed ({:?}): {error}",
            error.kind()
        ),
        Err(_) => anyhow::bail!(
            "Timed out after {budget:?} connecting to owned listener after shutdown: {address}"
        ),
    }
}

pub struct NativeHost {
    controller: Controller,
    status: watch::Receiver<Status>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl NativeHost {
    /// Starts without a Tokio runtime on the caller (usually the UI thread).
    pub fn start(mode: Mode) -> Result<Self> {
        let (controller, commands, views) = Controller::channels();
        let (status_tx, status) = watch::channel(Status::Starting);
        let (shutdown, stopping) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("workspacer-backend".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .thread_name("workspacer-worker")
                        .enable_all()
                        .build()?;
                    runtime.block_on(run(
                        mode,
                        commands,
                        views.clone(),
                        status_tx.clone(),
                        stopping,
                    ))
                })();
                match &result {
                    Ok(()) => {
                        status_tx.send_replace(Status::Stopped);
                    }
                    Err(error) => {
                        let message = format!("Backend failed: {error:#}");
                        let mut view = (**views.borrow()).clone();
                        view.connected = false;
                        view.busy = false;
                        view.creating = false;
                        view.notice = message.clone();
                        views.send_replace(Arc::new(view));
                        status_tx.send_replace(Status::Failed(message));
                    }
                }
                result
            })
            .context("starting native backend thread")?;
        Ok(Self {
            controller,
            status,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// Keeping this owner alive keeps the backend alive even without a window.
    pub fn controller(&self) -> Controller {
        self.controller.clone()
    }
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
    pub async fn ready(&self) -> Result<Ready> {
        let mut status = self.status.clone();
        loop {
            let current = status.borrow_and_update().clone();
            match current {
                Status::Ready(info) => return Ok(info),
                Status::Failed(error) => return Err(anyhow!(error)),
                Status::Stopped => return Err(anyhow!("Native backend stopped before readiness")),
                Status::Starting => {}
            }
            status
                .changed()
                .await
                .context("native backend readiness channel closed")?;
        }
    }
    pub fn request_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
    /// Use after GPUI exits or synchronously in its committed app-termination
    /// hook. Joining must never block normal rendering or window interaction.
    pub fn shutdown_blocking(mut self) -> Result<()> {
        self.request_shutdown();
        self.thread
            .take()
            .expect("owned backend thread")
            .join()
            .map_err(|_| anyhow!("Native backend thread panicked"))?
    }
    pub async fn shutdown(self) -> Result<()> {
        tokio::task::spawn_blocking(move || self.shutdown_blocking())
            .await
            .context("joining native backend")?
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

async fn run(
    mode: Mode,
    commands: mpsc::Receiver<Command>,
    views: watch::Sender<Arc<View>>,
    status: watch::Sender<Status>,
    mut stopping: oneshot::Receiver<()>,
) -> Result<()> {
    let config = match mode {
        #[cfg(feature = "rust-hub")]
        Mode::Rust(options) => {
            return rust_local::run(options, commands, views, status, stopping).await;
        }
        Mode::Remote(config) => config,
        Mode::Demo => {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("ws://{}/bus", listener.local_addr()?);
            tokio::spawn(crate::harness::serve(listener, 100, 1000));
            Config::new(url, None)?
        }
    };
    status.send_replace(Status::Ready(Ready {
        bus_url: config.url.clone(),
        engine_api: None,
        engine_hook: None,
        owned_listeners: vec![],
    }));
    let (backend, events) = Backend::connect(config);
    tokio::select! {
        _ = &mut stopping => {},
        _ = Controller::run(backend, events, commands, views) => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn listener_shutdown_proof_requires_refusal_and_reports_indeterminate_causes() {
        let address = "127.0.0.1:1".parse().unwrap();
        verify_connection_refused(address, std::time::Duration::from_secs(1), async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            Err(std::io::ErrorKind::ConnectionRefused.into())
        })
        .await
        .unwrap();
        let timeout = verify_connection_refused(
            address,
            std::time::Duration::from_millis(1),
            std::future::pending::<std::io::Result<()>>(),
        )
        .await
        .unwrap_err();
        assert!(timeout.to_string().contains("Timed out"));
        let unexpected =
            verify_connection_refused(address, std::time::Duration::from_secs(1), async {
                Err(std::io::ErrorKind::PermissionDenied.into())
            })
            .await
            .unwrap_err();
        assert!(unexpected.to_string().contains("PermissionDenied"));
        let reachable =
            verify_connection_refused(address, std::time::Duration::from_secs(1), async { Ok(()) })
                .await
                .unwrap_err();
        assert!(reachable.to_string().contains("remains reachable"));
    }

    #[tokio::test]
    async fn listener_shutdown_probe_refuses_a_live_listener_then_verifies_its_release() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(
            verify_owned_listeners_released(&[address])
                .await
                .unwrap_err()
                .to_string()
                .contains("remains reachable")
        );
        drop(listener);
        verify_owned_listeners_released(&[address]).await.unwrap();
    }

    use super::*;
    #[cfg(feature = "rust-hub")]
    #[test]
    fn rust_startup_failure_is_visible_and_joinable() {
        let path = std::env::temp_dir().join(format!(
            "wks-native-config-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "fixture: a file cannot be a config directory").unwrap();
        let database = path.with_extension("db");
        let host = NativeHost::start(Mode::Rust(RustOptions {
            config_dir: path.clone(),
            database: database.clone(),
            data_dir: path.with_extension("hub"),
            home_dir: path.with_extension("home"),
            usage_poll_on_boot: Some(false),
        }))
        .unwrap();
        let status = host.status();
        let controller = host.controller();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Status::Failed(message) = &*status.borrow() {
                assert!(message.contains("Backend failed"), "{message}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "failure was not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let view = controller.views.borrow().clone();
        assert!(!view.connected);
        assert!(!view.busy && !view.creating);
        assert!(view.notice.contains("Backend failed"));
        assert!(host.shutdown_blocking().is_err());
        assert!(
            !database.exists(),
            "invalid host configuration must fail before engine startup"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn backend_starts_without_ui_tokio_context_and_stops_without_a_window() {
        let host = NativeHost::start(Mode::Demo).unwrap();
        let controller = host.controller();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !controller.views.borrow().connected {
            assert!(
                std::time::Instant::now() < deadline,
                "backend did not connect"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(controller);
        let status = host.status();
        host.shutdown_blocking().unwrap();
        assert!(matches!(*status.borrow(), Status::Stopped));
    }
}
