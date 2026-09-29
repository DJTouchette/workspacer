//! Standalone administration and the in-process Rust launcher.
mod admin;
mod compat;
mod dev_watch;
mod identity;
mod install;
mod launcher_paths;
mod parent;
mod presentation;
mod readiness;
mod serve;
mod usage;
use anyhow::Result;
use clap::builder::TypedValueParser;
use clap::{Args, Parser, Subcommand};
pub use identity::{config_directory, load_or_create_host_token};
pub use serve::{ServePlan, plan_serve};
use std::{io::Write, path::PathBuf};
#[derive(Parser, Debug)]
#[command(
    name = "workspacer",
    about = "Workspacer Rust launcher and host administration"
)]
pub struct CommandLine {
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,
    #[arg(long, global = true, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, global = true, default_value_t = 7895)]
    pub hub_port: u16,
    #[arg(long, global = true, allow_hyphen_values = true)]
    pub token: Option<String>,
    #[arg(long, global = true)]
    pub tokens_file: Option<PathBuf>,
    #[arg(long,global=true,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,default_value_t=false,value_parser=compat::boolean)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    Serve(ServeArgs),
    Status {
        #[arg(long, default_value_t = 7891)]
        claudemon_api_port: u16,
    },
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    Jobs {
        #[command(subcommand)]
        command: JobsCommand,
    },
    Fleet {
        #[command(subcommand)]
        command: FleetCommand,
    },
    InstallCli {
        #[arg(long,value_parser=clap::builder::OsStringValueParser::new().map(PathBuf::from))]
        dir: Option<PathBuf>,
    },
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
}
#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, conflicts_with = "upstream")]
    pub hub_only: bool,
    /// Borrow a daemon only in explicit --hub-only mode; a bare flag uses its API port.
    #[arg(long, num_args=0..=1, default_missing_value="")]
    pub external_claudemon: Option<String>,
    #[arg(long)]
    pub quiet: bool,
    #[arg(long)]
    pub upstream: Option<String>,
    #[arg(long, requires = "upstream")]
    pub upstream_token_file: Option<PathBuf>,
    #[arg(long, requires = "upstream")]
    pub upstream_caller_token_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "full")]
    pub provider_scope: crate::provider_relay::Scope,
    #[arg(long, default_value = "")]
    pub node_id: String,
    #[arg(long)]
    pub nodes_file: Option<PathBuf>,
    #[arg(long)]
    pub nodes_keep_failed_wakes_running: bool,
    #[arg(long, default_value_t = 7891)]
    pub claudemon_api_port: u16,
    #[arg(long, default_value_t = 7890)]
    pub claudemon_hook_port: u16,
    #[arg(long,value_parser=clap::builder::OsStringValueParser::new().map(PathBuf::from))]
    pub claudemon_db_path: Option<PathBuf>,
    #[arg(long, default_value_t = 7897)]
    pub mcp_port: u16,
    #[arg(long)]
    pub no_mcp: bool,
    #[arg(long, value_enum)]
    pub untokened: Option<crate::mcp::UntokenedAccess>,
    #[arg(long)]
    pub mcp_token: Option<String>,
    #[arg(long,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,default_value_t=false,value_parser=compat::boolean)]
    pub no_claudemon_init: bool,
    #[arg(long,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,value_parser=compat::boolean)]
    pub allow_new_token: Option<bool>,
    #[arg(long,value_parser=clap::builder::OsStringValueParser::new().map(PathBuf::from))]
    pub plugins_dir: Option<PathBuf>,
    #[arg(long)]
    pub examples_dir: Option<PathBuf>,
    #[arg(long)]
    pub sidecar_node: Option<String>,
    #[arg(long)]
    pub plugin_origin: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub trusted_host: Vec<String>,
    #[arg(long,value_parser=clap::builder::OsStringValueParser::new().map(PathBuf::from))]
    pub webapp_dir: Option<PathBuf>,
    #[arg(long)]
    pub push_dir: Option<PathBuf>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    #[arg(long)]
    pub home_dir: Option<PathBuf>,
    #[arg(long)]
    pub jobs_file: Option<PathBuf>,
    #[arg(long)]
    pub no_jobs: bool,
    #[arg(long)]
    pub uploads_to_worker: bool,
    #[arg(long)]
    pub peers_file: Option<PathBuf>,
}
impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            hub_only: false,
            external_claudemon: None,
            quiet: false,
            upstream: None,
            upstream_token_file: None,
            upstream_caller_token_file: None,
            provider_scope: crate::provider_relay::Scope::Full,
            node_id: String::new(),
            nodes_file: None,
            nodes_keep_failed_wakes_running: false,
            claudemon_api_port: 7891,
            claudemon_hook_port: 7890,
            claudemon_db_path: None,
            mcp_port: 7897,
            no_mcp: false,
            untokened: None,
            mcp_token: None,
            no_claudemon_init: false,
            allow_new_token: None,
            plugins_dir: None,
            examples_dir: None,
            sidecar_node: None,
            plugin_origin: None,
            trusted_host: Vec::new(),
            webapp_dir: None,
            push_dir: None,
            data_dir: None,
            home_dir: None,
            jobs_file: None,
            no_jobs: false,
            uploads_to_worker: false,
            peers_file: None,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum TokenCommand {
    /// Initialize a local pairing identity; existing identities are preserved.
    InitHost {
        #[arg(long)]
        allow_new_token: bool,
    },
    Create {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value = "")]
        label: String,
        #[arg(long,hide=true,num_args=0..=1,default_missing_value="true",require_equals=true,value_parser=compat::boolean)]
        full_access: Option<bool>,
    },
    List,
    Revoke {
        #[arg(allow_hyphen_values = true)]
        reference: String,
    },
    FacadeAuthority {
        #[arg(long)]
        label: String,
        #[arg(long,required=true,action=clap::ArgAction::Set,value_parser=compat::boolean)]
        enabled: bool,
    },
}
#[derive(Subcommand, Debug)]
pub enum JobsCommand {
    List,
    Add {
        #[arg(short = 'f', long)]
        file: PathBuf,
    },
    Show {
        id: String,
    },
    History {
        id: String,
    },
    Run {
        id: String,
    },
    Approve {
        id: String,
        #[arg(long,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,default_value_t=false,value_parser=compat::boolean)]
        disabled: bool,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    Remove {
        id: String,
    },
}
#[derive(Subcommand, Debug)]
pub enum FleetCommand {
    Quiescence {
        #[arg(long,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,default_value_t=false,value_parser=compat::boolean)]
        quiet: bool,
    },
    Idle {
        #[arg(long,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true,default_value_t=false,value_parser=compat::boolean)]
        quiet: bool,
    },
}
#[derive(Subcommand, Debug)]
pub enum PluginCommand {
    Dev {
        directory: PathBuf,
        #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
        build: bool,
        #[arg(long, default_value_t = 400)]
        poll_ms: u64,
        #[arg(long, value_parser=dev_watch::duration, allow_hyphen_values=true, conflicts_with="poll_ms")]
        debounce: Option<std::time::Duration>,
        #[command(flatten)]
        serve: ServeArgs,
    },
}
impl CommandLine {
    pub fn try_parse_compatible_from<I, T>(args: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString>,
    {
        Self::try_parse_from(compat::normalize(args)?)
    }
    pub fn parse_compatible() -> Self {
        Self::try_parse_compatible_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }
    pub fn directory(&self) -> Result<PathBuf> {
        self.config_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(config_directory)
    }
    pub fn tokens_path(&self) -> Result<PathBuf> {
        match &self.tokens_file {
            Some(path) => Ok(path.clone()),
            None => Ok(self.directory()?.join("tokens.json")),
        }
    }
    pub fn credential(&self) -> Result<String> {
        if let Some(token) = self.token.as_ref().filter(|s| !s.is_empty()) {
            return Ok(token.clone());
        }
        if self.token.is_none() {
            if let Ok(token) = std::env::var("HUB_TOKEN") {
                if !token.is_empty() {
                    return Ok(token);
                }
            }
        }
        match std::fs::read_to_string(self.directory()?.join("remote-token")) {
            Ok(token) => Ok(token.trim().into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn authority(&self, port: u16) -> String {
        if self.host.is_empty() {
            return format!("127.0.0.1:{port}");
        }
        if let Ok(ip) = self
            .host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
        {
            return crate::net_address::dial_addr(std::net::SocketAddr::new(ip, port)).to_string();
        }
        format!(
            "{}:{port}",
            if self.host.contains(':') && !self.host.starts_with('[') {
                format!("[{}]", self.host)
            } else {
                self.host.clone()
            }
        )
    }
}
pub async fn run(args: CommandLine) -> Result<i32> {
    execute(&args, &mut std::io::stdout(), &mut std::io::stderr()).await
}
pub async fn execute(args: &CommandLine, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32> {
    match &args.command {
        Command::Serve(serve) => serve::run(args, serve, None, out, err).await,
        Command::Plugin {
            command:
                PluginCommand::Dev {
                    directory,
                    build,
                    poll_ms,
                    debounce,
                    serve,
                },
        } => {
            serve::run(
                args,
                serve,
                Some((
                    directory.clone(),
                    *build,
                    debounce.unwrap_or_else(|| std::time::Duration::from_millis(*poll_ms)),
                )),
                out,
                err,
            )
            .await
        }
        Command::Token { command } => identity::token(args, command, out),
        Command::Status { claudemon_api_port } => {
            admin::status(args, *claudemon_api_port, out).await
        }
        Command::Jobs { command } => admin::jobs(args, command, out).await,
        Command::Fleet { command } => match admin::fleet(args, command, out).await {
            Ok(code) => Ok(code),
            Err(error) => {
                writeln!(err, "workspacer fleet: {error:#}")?;
                Ok(2)
            }
        },
        Command::InstallCli { dir } => install::run(dir.as_deref(), out),
    }
}
pub(super) fn print_json(out: &mut dyn Write, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}
