#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod ui;

use anyhow::{Context as _, Result};
use clap::Parser;
use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme, ThemeMode};
use std::path::PathBuf;
use wks_native::{bus::Config, controller::Controller};

#[derive(Parser)]
#[command(about = "Experimental native Workspacer client (connects to an existing hub)")]
struct Args {
    #[arg(long, env = "WKS_HUB_BUS", default_value = "ws://127.0.0.1:7895/bus")]
    bus: String,
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

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let _runtime_guard = runtime.enter();
    let config = if args.demo {
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
        let url = format!("ws://{}/bus", listener.local_addr()?);
        runtime.spawn(wks_native::harness::serve(listener, 100, 1_000));
        Config::new(url, None)?
    } else {
        let token = if let Some(path) = args.token_file {
            Some(std::fs::read_to_string(path).context("Read token file")?)
        } else if let Ok(token) = std::env::var("HUB_TOKEN") {
            Some(token)
        } else {
            // Never send the local host's discovered credential to a remote hub.
            let url = url::Url::parse(&args.bus).context("Invalid hub URL")?;
            let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
            if local {
                token_path().and_then(|p| std::fs::read_to_string(p).ok())
            } else {
                None
            }
        }
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
        Config::new(args.bus, token)?
    };
    let controller = Controller::start(config);
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
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
                window.set_window_title("Workspacer Native");
                window.set_app_id("workspacer-native");
                let view = cx.new(|cx| {
                    let mut view = ui::Workspace::new(controller, args.demo, window, cx);
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
    Ok(())
}
