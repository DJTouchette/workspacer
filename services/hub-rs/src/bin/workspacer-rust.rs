#[tokio::main]
async fn main() -> std::process::ExitCode {
    match workspacer_hub::cli::run(workspacer_hub::cli::CommandLine::parse_compatible()).await {
        Ok(code) => std::process::ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("workspacer: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
