//! Lifetime owner for the native backend. GPUI never enters this Tokio runtime.
use crate::{
    backend::Backend,
    bus::Config,
    controller::{Command, Controller, View},
};
use anyhow::{Context, Result, anyhow};
use std::{sync::Arc, thread::JoinHandle};
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(feature = "embedded")]
mod local;
#[cfg(feature = "embedded")]
pub use local::LocalOptions;

pub enum Mode {
    Remote(Config),
    Demo,
    #[cfg(feature = "embedded")]
    Local(LocalOptions),
}

#[derive(Clone, Debug)]
pub struct Ready {
    pub bus_url: String,
    pub engine_api: Option<String>,
    pub engine_hook: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Status {
    Starting,
    Ready(Ready),
    Failed(String),
    Stopped,
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
        #[cfg(feature = "embedded")]
        Mode::Local(options) => {
            return local::run(options, commands, views, status, stopping).await;
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
    use super::*;
    #[cfg(feature = "embedded")]
    #[test]
    fn local_startup_failure_is_visible_and_joinable() {
        let host = NativeHost::start(Mode::Local(LocalOptions {
            services_dir: Some(std::path::PathBuf::from("/nonexistent/native-services")),
            database: std::path::PathBuf::from("unused-native-startup.db"),
            hub_port: 7895,
            mcp_port: 7897,
            hook_port: 0,
            api_port: 0,
            no_plugins: true,
        }))
        .unwrap();
        let status = host.status();
        let controller = host.controller();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Status::Failed(message) = &*status.borrow() {
                assert!(message.contains("Missing"), "{message}");
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
        assert!(view.notice.contains("Missing"));
        assert!(host.shutdown_blocking().is_err());
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
