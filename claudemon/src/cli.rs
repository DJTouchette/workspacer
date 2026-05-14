use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "claudemon", version, about = "Observability daemon for Claude Code sessions")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the daemon: hook ingress on 7890, API on 7891.
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 7890)]
        hook_port: u16,
        #[arg(long, default_value_t = 7891)]
        api_port: u16,
    },
    /// Merge claudemon's hook configuration into ~/.claude/settings.json.
    Init {
        /// Print the merged document instead of writing it.
        #[arg(long)]
        dry_run: bool,
        /// Port the daemon's hook listener is bound to. Used to build the
        /// curl command we install.
        #[arg(long, default_value_t = 7890)]
        hook_port: u16,
    },
    /// Run a command under a PTY wrapper so the daemon can relay input.
    Wrap {
        /// WebSocket base URL of the daemon's wrapper endpoint.
        #[arg(long, default_value = "ws://127.0.0.1:7891/wrapper")]
        daemon: String,
        #[arg(trailing_var_arg = true)]
        argv: Vec<String>,
    },
    /// Attach a TUI to a running daemon (stub).
    Watch {
        #[arg(long, default_value = "http://127.0.0.1:7891")]
        api: String,
    },
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Serve { host, hook_port, api_port } => {
            crate::daemon::run(&host, hook_port, api_port).await
        }
        Command::Init { dry_run, hook_port } => {
            crate::daemon::init::run_with_port(dry_run, hook_port).await
        }
        Command::Wrap { daemon, argv } => {
            crate::wrapper::run_with_daemon(argv, &daemon).await
        }
        Command::Watch { api } => {
            tracing::info!(api, "claudemon watch is not implemented yet");
            println!("TUI not implemented yet. For now, try: curl {api}/sessions");
            Ok(())
        }
    }
}
