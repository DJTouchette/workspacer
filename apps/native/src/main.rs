#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod ui;

use anyhow::{Context as _, Result};
use clap::Parser;
use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use wks_native::appearance::{Appearance, preference_path};
use wks_native::navigation::{Settings, settings_path};
use wks_native::{
    bus::Config,
    host::{Mode, NativeHost},
};

#[derive(Parser)]
#[command(about = "Experimental native Workspacer client (connects to an existing hub)")]
struct Args {
    #[arg(long, env = "WKS_HUB_BUS", conflicts_with = "local")]
    bus: Option<String>,
    /// Own an embedded local engine and its hub services.
    #[arg(long, conflicts_with_all = ["bus", "demo", "token_file"])]
    local: bool,
    /// Own the Rust backend with isolated state in this directory.
    #[cfg(feature = "rust-hub")]
    #[arg(long, conflicts_with_all = ["bus", "demo", "token_file", "services_dir", "database"])]
    rust_local_dir: Option<PathBuf>,
    /// Retired service-bundle override; local ownership uses Rust in process.
    #[arg(long, requires = "local")]
    services_dir: Option<PathBuf>,
    #[arg(long, requires = "local")]
    database: Option<PathBuf>,
    #[arg(long, default_value_t = 7895)]
    hub_port: u16,
    #[arg(long, default_value_t = 7897)]
    mcp_port: u16,
    /// Minimize on window close; explicit Quit still stops owned services.
    #[arg(long)]
    keep_running: bool,
    /// Read credentials from a file. HUB_TOKEN takes precedence over discovery.
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Run against an isolated local fixture, without agents or credentials.
    #[arg(long)]
    demo: bool,
    /// Pin this window to one session; disable controls if it becomes unavailable.
    #[arg(long)]
    session: Option<String>,
}

fn token_path() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
    };
    Some(base.join("workspacer/remote-token"))
}

struct BackendLifetime {
    host: Option<NativeHost>,
    stopped: Option<Result<()>>,
}

impl BackendLifetime {
    fn stop(&mut self) {
        if let Some(host) = self.host.take() {
            let result = host.shutdown_blocking();
            if let Err(error) = &result {
                eprintln!("Native backend shutdown failed: {error:#}");
            }
            self.stopped = Some(result);
        }
    }
}

fn stop_backend_on_quit(owner: Rc<RefCell<BackendLifetime>>, cx: &gpui::App) {
    cx.on_app_quit(move |_| {
        // macOS termination need not return from Application::run. GPUI gives
        // the returned future only 100ms, so join during this synchronous hook
        // after termination is committed, never during ordinary UI rendering.
        owner.borrow_mut().stop();
        std::future::ready(())
    })
    .detach();
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    let bus_url = args
        .bus
        .clone()
        .unwrap_or_else(|| "ws://127.0.0.1:7895/bus".into());
    let local_requested = args.local;
    #[cfg(feature = "rust-hub")]
    let local_requested = local_requested || args.rust_local_dir.is_some();
    #[cfg(feature = "rust-hub")]
    let rust_mode = if local_requested {
        anyhow::ensure!(
            args.services_dir.is_none() && args.database.is_none(),
            "Rust local mode uses --rust-local-dir, not legacy service/database paths"
        );
        anyhow::ensure!(
            args.hub_port == 7895 && args.mcp_port == 7897,
            "Rust native local mode owns an in-process bus; port overrides are unavailable"
        );
        let directory = match &args.rust_local_dir {
            Some(directory) => directory.clone(),
            None => wks_native::host::RustOptions::default_directory()?,
        };
        Some(Mode::Rust(wks_native::host::RustOptions::isolated(
            directory,
        )?))
    } else {
        None
    };
    #[cfg(not(feature = "rust-hub"))]
    let rust_mode: Option<Mode> = None;
    let mode = if let Some(mode) = rust_mode {
        mode
    } else if args.local {
        anyhow::bail!("This build does not include the Rust backend; build with rust-hub");
    } else if args.demo {
        Mode::Demo
    } else {
        let token = if let Some(path) = &args.token_file {
            Some(std::fs::read_to_string(path).context("Read token file")?)
        } else if let Ok(token) = std::env::var("HUB_TOKEN") {
            Some(token)
        } else {
            // Local identity is never implicitly forwarded to a remote endpoint.
            let url = url::Url::parse(&bus_url).context("Invalid hub URL")?;
            if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) {
                token_path().and_then(|p| std::fs::read_to_string(p).ok())
            } else {
                None
            }
        }
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
        Mode::Remote(Config::new(bus_url.clone(), token)?)
    };
    let appearance = preference_path()
        .and_then(|path| match Appearance::load(&path) {
            Ok(appearance) => Some(appearance),
            Err(error) => {
                eprintln!("Could not load native theme; using Dark: {error}");
                None
            }
        })
        .unwrap_or_default();
    let settings_path = settings_path();
    let settings = settings_path
        .as_ref()
        .map(|p| Settings::load(p))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("Could not load native settings: {error}");
            None
        })
        .unwrap_or_default();
    let project_scope = if args.demo {
        "demo".to_owned()
    } else if local_requested {
        #[cfg(feature = "rust-hub")]
        {
            let Mode::Rust(options) = &mode else {
                unreachable!("local mode has a Rust owner")
            };
            format!("rust-local:{}", options.config_dir.display())
        }
        #[cfg(not(feature = "rust-hub"))]
        {
            unreachable!("local mode requires the Rust feature")
        }
    } else {
        let url = url::Url::parse(&bus_url)?;
        format!(
            "{}://{}:{}{}",
            url.scheme(),
            url.host_str().unwrap_or(""),
            url.port_or_known_default().unwrap_or(0),
            url.path()
        )
    };
    let keep_running = Rc::new(std::cell::Cell::new(
        args.keep_running || settings.keep_running,
    ));
    let host = NativeHost::start(mode)?;
    let controller = host.controller();
    let backend = Rc::new(RefCell::new(BackendLifetime {
        host: Some(host),
        stopped: None,
    }));
    let quitting_backend = backend.clone();
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            stop_backend_on_quit(quitting_backend, cx);
            gpui_component::init(cx);
            ui::configure_theme(appearance, None, cx);
            ui::bind_keys(cx);
            let bounds = Bounds::centered(None, size(px(1120.), px(780.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.), px(480.))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Workspacer Native".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let close_preference = keep_running.clone();
                    window.on_window_should_close(cx, move |window, _| {
                        if close_preference.get() {
                            window.minimize_window();
                            false
                        } else {
                            true
                        }
                    });
                    window.set_window_title("Workspacer Native");
                    window.set_app_id("workspacer-native");
                    let view = cx.new(|cx| {
                        let mut view = ui::Workspace::new(controller, args.demo, window, cx);
                        view.configure_settings(settings, settings_path, project_scope);
                        view.configure_local(local_requested, keep_running.clone());
                        view.set_appearance(appearance, window, cx);
                        view.open_session(args.session);
                        view
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("open native window");
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    let mut backend = backend.borrow_mut();
    backend.stop();
    backend.stopped.take().unwrap_or(Ok(()))
}

#[cfg(all(test, feature = "ui-tests"))]
mod lifetime_tests {
    use super::*;
    use wks_native::host::Status;

    #[gpui::test]
    fn app_quit_joins_backend_without_waiting_for_run_to_return(cx: &mut gpui::TestAppContext) {
        let host = NativeHost::start(Mode::Demo).unwrap();
        let status = host.status();
        let backend = Rc::new(RefCell::new(BackendLifetime {
            host: Some(host),
            stopped: None,
        }));
        cx.update(|cx| {
            stop_backend_on_quit(backend.clone(), cx);
            cx.shutdown();
        });
        assert!(backend.borrow().host.is_none());
        assert!(matches!(*status.borrow(), Status::Stopped));
        assert!(backend.borrow().stopped.as_ref().unwrap().is_ok());
        // The post-run fallback must preserve the first shutdown outcome.
        backend.borrow_mut().stop();
        assert!(backend.borrow().stopped.as_ref().unwrap().is_ok());
    }
}
