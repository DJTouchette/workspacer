use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
pub(super) type Stamp = Vec<(PathBuf, u64, SystemTime)>;
pub(super) fn stamp(root: &Path) -> Result<Stamp> {
    fn walk(root: &Path, out: &mut Stamp) -> Result<()> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if [
                ".git",
                "node_modules",
                "target",
                ".bus-token",
                ".settings.json",
                ".install-source",
                ".disabled",
            ]
            .iter()
            .any(|ignore| name == *ignore)
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), out)?
            } else if meta.is_file() || meta.file_type().is_symlink() {
                out.push((entry.path(), meta.len(), meta.modified()?));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}
pub(super) struct Debounce {
    previous: Stamp,
    dirty: bool,
}
impl Debounce {
    pub fn new(previous: Stamp) -> Self {
        Self {
            previous,
            dirty: false,
        }
    }
    pub fn observe(&mut self, current: Stamp) -> bool {
        if current != self.previous {
            self.previous = current;
            self.dirty = true;
            false
        } else {
            std::mem::take(&mut self.dirty)
        }
    }
    pub fn reset(&mut self, current: Stamp) {
        self.previous = current;
        self.dirty = false;
    }
}
/// Go-style duration units retained for the public --debounce flag.
pub(super) fn duration(raw: &str) -> std::result::Result<Duration, String> {
    let negative = raw.starts_with('-');
    let mut rest = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    let mut seconds = 0.0;
    let mut parts = 0;
    while !rest.is_empty() {
        let end = rest
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        let number = rest[..end]
            .parse::<f64>()
            .map_err(|_| "invalid debounce duration")?;
        rest = &rest[end..];
        let (unit, factor) = [
            ("ns", 1e-9),
            ("us", 1e-6),
            ("µs", 1e-6),
            ("μs", 1e-6),
            ("ms", 1e-3),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 3600.0),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(*unit))
        .ok_or("debounce duration needs a unit (for example 400ms)")?;
        seconds += number * factor;
        rest = &rest[unit.len()..];
        parts += 1;
    }
    if parts == 0 {
        return Err("empty debounce duration".into());
    }
    let parsed =
        Duration::try_from_secs_f64(seconds).map_err(|_| "debounce duration is out of range")?;
    Ok(if negative { Duration::ZERO } else { parsed })
}
pub(super) fn stream_unavailable(
    out: &mut dyn std::io::Write,
    detail: &str,
) -> std::io::Result<()> {
    writeln!(
        out,
        "plugin lifecycle stream unavailable ({}); continuing file watch",
        crate::diagnostics::credential_queries(detail)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_coalesce_until_quiet_and_loader_markers_never_trigger_reload() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("source.txt");
        std::fs::write(&file, "first").unwrap();
        let mut watcher = Debounce::new(stamp(root.path()).unwrap());
        for marker in [
            ".bus-token",
            ".settings.json",
            ".install-source",
            ".disabled",
        ] {
            std::fs::write(root.path().join(marker), "host writes").unwrap();
        }
        for directory in [".git", "node_modules", "target"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
            std::fs::write(root.path().join(directory).join("noise"), "ignored").unwrap();
        }
        assert!(!watcher.observe(stamp(root.path()).unwrap()));
        std::fs::write(&file, "second edit").unwrap();
        assert!(!watcher.observe(stamp(root.path()).unwrap()));
        std::fs::write(&file, "third edit in burst").unwrap();
        assert!(!watcher.observe(stamp(root.path()).unwrap()));
        assert!(watcher.observe(stamp(root.path()).unwrap()));
        assert!(!watcher.observe(stamp(root.path()).unwrap()));
        std::fs::remove_file(file).unwrap();
        assert!(!watcher.observe(stamp(root.path()).unwrap()));
        assert!(watcher.observe(stamp(root.path()).unwrap()));
        assert!(stamp(&root.path().join("missing")).unwrap().is_empty());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path(), root.path().join("self-link")).unwrap();
            let linked = stamp(root.path()).unwrap();
            assert_eq!(
                linked.len(),
                1,
                "count a symlink edit without following its cycle"
            );
            assert!(!watcher.observe(linked.clone()));
            assert!(watcher.observe(linked));
        }
    }
    #[test]
    fn event_subscription_errors_keep_context_without_query_credentials() {
        let mut out = Vec::new();
        stream_unavailable(&mut out, "dial ws://local/bus?token=PRIVATE&pane=1 failed").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("PRIVATE"));
        assert!(text.contains("token=REDACTED&pane=1 failed"));
        assert!(text.contains("continuing file watch"));
    }
    #[test]
    fn debounce_accepts_legacy_duration_units_and_rejects_invalid_values() {
        assert_eq!(duration("400ms").unwrap(), Duration::from_millis(400));
        assert_eq!(duration("1m2.5s").unwrap(), Duration::from_millis(62_500));
        assert_eq!(duration("-2s").unwrap(), Duration::ZERO);
        assert_eq!(duration("0").unwrap(), Duration::ZERO);
        assert_eq!(duration("1µs").unwrap(), Duration::from_micros(1));
        for bad in ["", "forever", "20", "1.2.3s", "1s-2s"] {
            assert!(duration(bad).is_err(), "{bad}");
        }
    }
}
