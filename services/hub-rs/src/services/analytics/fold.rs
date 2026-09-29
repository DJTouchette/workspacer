use super::*;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
};
#[derive(Default)]
pub struct Fold {
    pub model: Option<String>,
    pub input: f64,
    pub output: f64,
    pub cost: f64,
    pub peak: f64,
    pub models: serde_json::Map<String, Value>,
}
fn path_allowed(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let canonical = super::super::paths::canonicalize(path)?;
    if !roots.iter().any(|root| {
        super::super::paths::canonicalize(root)
            .is_ok_and(|root| super::super::routing::path::path_within(&canonical, &root))
    }) {
        bail!("analytics transcript outside configured Claude roots");
    }
    Ok(canonical)
}
pub fn files(main: &Path, roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let main = path_allowed(main, roots)?;
    let mut files = vec![main.clone()];
    let dir = main.with_extension("").join("subagents");
    if let Ok(entries) = fs::read_dir(dir) {
        let mut sub = Vec::new();
        for entry in entries {
            let p = entry?.path();
            if p.extension().is_some_and(|e| e == "jsonl") {
                sub.push(path_allowed(&p, roots)?);
            }
        }
        sub.sort();
        files.extend(sub);
    }
    Ok(files)
}
pub fn fingerprint(files: &[PathBuf]) -> Result<String> {
    let mut result = Vec::new();
    for path in files {
        let m = fs::metadata(path)?;
        let modified = m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        };
        #[cfg(windows)]
        let identity = {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
            };
            let file = fs::File::open(path)?;
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            (
                info.dwVolumeSerialNumber as u64,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            )
        };
        #[cfg(not(any(unix, windows)))]
        let identity = (0u64, 0u64);
        result.push(json!([path, identity.0, identity.1, m.len(), modified]));
    }
    Ok(serde_json::to_string(&result)?)
}
pub fn recompute(files: &[PathBuf], rates: &Value) -> Result<Option<Fold>> {
    let mut out = Fold::default();
    let mut seen = HashSet::new();
    let mut any = false;
    for (index, path) in files.iter().enumerate() {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if index > 0 && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for line in BufReader::new(file).lines() {
            let line = line?;
            let Ok(row) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if row["type"] != "assistant" || !row["message"]["usage"].is_object() {
                continue;
            }
            any = true;
            let msg = &row["message"];
            let usage = &msg["usage"];
            let side = index > 0 || row["isSidechain"] == true;
            let model = msg["model"].as_str().filter(|s| !s.starts_with('<'));
            let input = n(&[&usage["input_tokens"]])
                + n(&[&usage["cache_creation_input_tokens"]])
                + n(&[&usage["cache_read_input_tokens"]]);
            if !side {
                out.peak = out.peak.max(input);
                if let Some(model) = model.filter(|m| !m.is_empty()) {
                    out.model = Some(model.into());
                }
            }
            let id = msg["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| row["uuid"].as_str().filter(|s| !s.is_empty()));
            if id.is_some_and(|id| !seen.insert(id.to_string())) {
                continue;
            }
            let model = model.or(out.model.as_deref());
            let output = n(&[&usage["output_tokens"]]);
            let cost = super::super::pricing::Pricing::turn_cost_with(rates, model, usage);
            out.input += input;
            out.output += output;
            out.cost += cost;
            let slice = out
                .models
                .entry(model.unwrap_or("(unknown)").to_string())
                .or_insert(json!({"inputTokens":0.,"outputTokens":0.,"costUSD":0.}));
            for (key, value) in [
                ("inputTokens", input),
                ("outputTokens", output),
                ("costUSD", cost),
            ] {
                slice[key] = (n(&[&slice[key]]) + value).into();
            }
        }
    }
    Ok(any.then_some(out))
}
