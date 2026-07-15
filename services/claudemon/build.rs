fn main() {
    // Windows Task Manager labels a process with its exe's FileDescription
    // version resource (falling back to the bare file name when absent).
    // Stamp a friendly one so the daemon reads as "claudemon" next to the
    // Workspacer app instead of an anonymous exe. Cosmetic only — never fail
    // the build over it (e.g. rc.exe missing from a minimal toolchain).
    if std::env::var_os("CARGO_CFG_TARGET_OS").is_some_and(|os| os == "windows") {
        let mut res = winresource::WindowsResource::new();
        res.set("FileDescription", "claudemon");
        res.set("ProductName", "claudemon");
        if let Err(err) = res.compile() {
            println!("cargo:warning=skipping Windows version resource: {err}");
        }
    }
}
