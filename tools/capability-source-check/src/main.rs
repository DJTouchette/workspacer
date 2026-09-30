use std::path::PathBuf;
fn main() {
    if let Err(error) = run() {
        eprintln!("capability-source-check: {error}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from(".");
    let mut method = None;
    let mut summary = false;
    let mut check = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = PathBuf::from(args.next().ok_or("--root needs a directory")?),
            "--method" => method = Some(args.next().ok_or("--method needs a name")?),
            "--summary" => summary = true,
            "--check" => check = true,
            "--help" => {
                println!(
                    "capability-source-check --root REPOSITORY\nParses production Rust capability handlers; emits JSON fields, opaque payloads and unresolved source flows. No providers are invoked."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let sources = workspacer_capability_source_check::read_sources(&root)?;
    let report = workspacer_capability_source_check::scan(sources)?;
    if check {
        let policy: workspacer_capability_source_check::policy::Policy =
            serde_json::from_slice(&std::fs::read(
                root.join("apps/desktop/tests/fixtures/capability-parameter-policy.json"),
            )?)?;
        let surface: workspacer_capability_source_check::policy::Surface = serde_json::from_slice(
            &std::fs::read(root.join("contracts/backend-capabilities.json"))?,
        )?;
        let vocabulary = serde_json::from_slice(&std::fs::read(
            root.join("apps/desktop/tests/fixtures/capability-parameter-vocabulary.json"),
        )?)?;
        let mut checked = policy.check(&report, &surface, &vocabulary);
        let original: workspacer_capability_source_check::reference::Reference =
            serde_json::from_slice(&std::fs::read(
                root.join("tools/capability-source-check/go-reference.json"),
            )?)?;
        checked.errors.extend(original.check(&root, &report));
        let spawn_keys = serde_json::from_slice(&std::fs::read(
            root.join("contracts/spawn-parameter-keys.json"),
        )?)?;
        let historical = serde_json::from_slice(&std::fs::read(
            root.join("services/hub-rs/assets/hub-vocabulary.json"),
        )?)?;
        checked.errors.extend(
            workspacer_capability_source_check::policy::check_spawn_keys(
                &report,
                &spawn_keys,
                &historical,
            ),
        );
        println!("{}", serde_json::to_string_pretty(&checked)?);
        if !checked.errors.is_empty() {
            return Err(format!("{} source-policy errors", checked.errors.len()).into());
        }
        return Ok(());
    }
    if let Some(method) = method {
        println!(
            "{}",
            serde_json::to_string_pretty(report.methods.get(&method).ok_or("method not found")?)?
        )
    } else if summary {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"sourceFiles":report.source_files,"methods":report.methods.len(),"fields":report.methods.values().map(|b|b.fields.len()).sum::<usize>(),"opaque":report.methods.iter().filter(|(_,b)|!b.opaque.is_empty()).map(|(m,b)|(m,&b.opaque)).collect::<std::collections::BTreeMap<_,_>>(),"unresolved":report.methods.iter().filter(|(_,b)|!b.unresolved.is_empty()).map(|(m,b)|(m,&b.unresolved)).collect::<std::collections::BTreeMap<_,_>>(),"unresolvedRegistrations":report.unresolved_registrations})
            )?
        )
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?)
    }
    Ok(())
}
