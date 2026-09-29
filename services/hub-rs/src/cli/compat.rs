//! Retain Go's single-dash long options without rewriting values or -- tails.
use clap::CommandFactory;
use std::{collections::BTreeMap, ffi::OsString};

pub(super) fn normalize<I, T>(args: I) -> Result<Vec<OsString>, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    fn collect(command: &clap::Command, out: &mut BTreeMap<String, (usize, usize, bool, bool)>) {
        for arg in command.get_arguments() {
            let (minimum, maximum) = if arg.get_action().takes_values() {
                arg.get_num_args()
                    .map(|range| (range.min_values(), range.max_values()))
                    .unwrap_or((1, 1))
            } else {
                (0, 0)
            };
            if let Some(long) = arg.get_long() {
                out.insert(
                    long.into(),
                    (minimum, maximum, true, arg.is_require_equals_set()),
                );
            }
            if let Some(short) = arg.get_short() {
                out.insert(
                    short.to_string(),
                    (minimum, maximum, false, arg.is_require_equals_set()),
                );
            }
        }
        for sub in command.get_subcommands() {
            collect(sub, out)
        }
    }
    let mut command = super::CommandLine::command();
    command.build();
    let mut flags = BTreeMap::new();
    collect(&command, &mut flags);
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let mut out = Vec::with_capacity(args.len());
    let mut value = false;
    let mut ended = false;
    for (index, arg) in args.iter().enumerate() {
        if value {
            value = false;
            continue;
        }
        if index == 0 || ended {
            out.push(arg.clone());
            continue;
        }
        let Some(text) = arg.to_str() else {
            out.push(arg.clone());
            continue;
        };
        if text == "--" {
            ended = true;
            out.push(arg.clone());
            continue;
        }
        let Some(body) = text.strip_prefix("--").or_else(|| text.strip_prefix('-')) else {
            out.push(arg.clone());
            continue;
        };
        let (name, inline) = body
            .split_once('=')
            .map(|(name, _)| (name, true))
            .unwrap_or((body, false));
        if ["claudemon-bin", "hub-bin", "brain-bin", "mcp-bin"].contains(&name) {
            return Err(clap::Error::raw(
                clap::error::ErrorKind::UnknownArgument,
                format!(
                    "--{name} is no longer supported: Workspacer owns its Rust engine, hub and MCP facade in process; there is no child backend binary to override"
                ),
            ));
        }
        if let Some((minimum, maximum, long, equals)) = flags.get(name) {
            let mut flag = OsString::from(format!("{}{body}", if *long { "--" } else { "-" }));
            if !inline && *maximum > 0 && !*equals {
                let consumes = *minimum > 0
                    || args.get(index + 1).is_some_and(|next| {
                        next.to_str().is_some_and(|next| !next.starts_with('-'))
                    });
                if consumes {
                    if let Some(next) = args.get(index + 1) {
                        // An equals separator makes a leading-dash value literal
                        // to Clap, just as Go's flag parser treats the next argv.
                        // Append the OsString itself so non-UTF8 filenames survive.
                        flag.push("=");
                        flag.push(next);
                        value = true;
                    }
                }
            }
            out.push(flag);
        } else {
            out.push(arg.clone())
        }
    }
    Ok(out)
}

pub(super) fn boolean(value: &str) -> Result<bool, String> {
    match value {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Ok(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Ok(false),
        _ => Err("expected a boolean (true or false)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Command, CommandLine, TokenCommand};
    #[test]
    fn legacy_aliases_preserve_flag_values_inline_values_and_end_of_options() {
        let args = CommandLine::try_parse_compatible_from([
            "workspacer",
            "-json",
            "-token",
            "-hub-port",
            "status",
            "-hub-port=19095",
        ])
        .unwrap();
        assert!(args.json);
        assert_eq!(args.token.as_deref(), Some("-hub-port"));
        assert_eq!(args.hub_port, 19095);
        let args = CommandLine::try_parse_compatible_from([
            "workspacer",
            "token",
            "revoke",
            "--",
            "-hub-port",
        ])
        .unwrap();
        let Command::Token {
            command: TokenCommand::Revoke { reference },
        } = args.command
        else {
            panic!()
        };
        assert_eq!(reference, "-hub-port");
        let error = CommandLine::try_parse_compatible_from([
            "workspacer",
            "serve",
            "-brain-bin",
            "old-brain",
        ])
        .unwrap_err();
        assert!(error.to_string().contains("in process"));
        let args =
            CommandLine::try_parse_compatible_from(["workspacer", "-token=--brain-bin", "status"])
                .unwrap();
        assert_eq!(args.token.as_deref(), Some("--brain-bin"));
        let args =
            CommandLine::try_parse_compatible_from(["workspacer", "jobs", "add", "-f", "-token"])
                .unwrap();
        let Command::Jobs {
            command: crate::cli::JobsCommand::Add { file },
        } = args.command
        else {
            panic!()
        };
        assert_eq!(file, std::path::Path::new("-token"));
        let args = CommandLine::try_parse_compatible_from([
            "workspacer",
            "token",
            "create",
            "-scope",
            "operator",
            "-label",
            "--brain-bin",
        ])
        .unwrap();
        let Command::Token {
            command: TokenCommand::Create { label, .. },
        } = args.command
        else {
            panic!()
        };
        assert_eq!(label, "--brain-bin");
        let args =
            CommandLine::try_parse_compatible_from(["workspacer", "serve", "-host=", "--hub-only"])
                .unwrap();
        assert_eq!(args.authority(7895), "127.0.0.1:7895");
        let Command::Serve(serve) = &args.command else {
            panic!()
        };
        assert!(
            crate::cli::plan_serve(&args, serve)
                .unwrap()
                .listen
                .ip()
                .is_unspecified()
        );

        for value in ["true", "True", "TRUE", "1", "t"] {
            let args = CommandLine::try_parse_compatible_from([
                "workspacer",
                &format!("-json={value}"),
                "serve",
                "-allow-new-token=false",
                "-no-claudemon-init=0",
            ])
            .unwrap();
            assert!(args.json);
            let Command::Serve(serve) = args.command else {
                panic!()
            };
            assert_eq!(serve.allow_new_token, Some(false));
            assert!(!serve.no_claudemon_init);
        }
    }
}
