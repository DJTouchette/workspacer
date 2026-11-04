//! Host resource sampling for the work queue's admission gate.
//!
//! Every metric is optional: `None` means unknown (not measured on this
//! platform, unreadable, or not sampled yet), never zero, and an unknown
//! metric never closes the gate. Linux PSI comes from `/proc/pressure/*`
//! ("some avg10"); elsewhere those fields stay unknown.
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

/// How often the monitor samples.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub sampled_at: i64,
    pub cores: Option<u64>,
    pub load1: Option<f64>,
    pub usage_pct: Option<f64>,
    pub cpu_pressure: Option<f64>,
    pub memory_total_mb: Option<u64>,
    pub memory_available_mb: Option<u64>,
    pub memory_pressure: Option<f64>,
    pub io_pressure: Option<f64>,
    pub disk_path: String,
    pub disk_free_mb: Option<u64>,
    pub disk_total_mb: Option<u64>,
}
impl Sample {
    /// The `cpu`/`memory`/`io`/`disk` members of the `host.resources` wire
    /// object. The caller adds `host`, `sampledAt` and `queue`.
    pub fn wire(&self) -> Value {
        json!({
            "cpu": {"cores": self.cores, "load1": self.load1, "usagePct": self.usage_pct, "pressureAvg10": self.cpu_pressure},
            "memory": {"totalMb": self.memory_total_mb, "availableMb": self.memory_available_mb, "pressureAvg10": self.memory_pressure},
            "io": {"pressureAvg10": self.io_pressure},
            "disk": {"path": self.disk_path, "freeMb": self.disk_free_mb, "totalMb": self.disk_total_mb},
        })
    }
}

/// The gate thresholds (`queue.*` config keys).
#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    pub max_cpu_pressure: f64,
    pub max_memory_pressure: f64,
    pub min_available_memory_mb: u64,
    pub min_free_disk_mb: u64,
}

/// The "some avg10" figure of a PSI file (`/proc/pressure/{cpu,memory,io}`):
///
/// ```text
/// some avg10=1.23 avg60=0.50 avg300=0.10 total=12345
/// full avg10=0.00 avg60=0.00 avg300=0.00 total=0
/// ```
///
/// Anything unparseable is unknown.
pub fn parse_psi_some_avg10(text: &str) -> Option<f64> {
    let line = text
        .lines()
        .find(|line| line.split_whitespace().next() == Some("some"))?;
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// Every reason the gate is closed; empty means open. Unknown metrics are
/// skipped, so a host that cannot measure something is never blocked by it.
pub fn gate_reasons(sample: &Sample, limits: &Thresholds) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(pressure) = sample.cpu_pressure
        && pressure >= limits.max_cpu_pressure
    {
        reasons.push(format!(
            "cpu pressure {pressure:.1} >= {:.1}",
            limits.max_cpu_pressure
        ));
    }
    if let Some(pressure) = sample.memory_pressure
        && pressure >= limits.max_memory_pressure
    {
        reasons.push(format!(
            "memory pressure {pressure:.1} >= {:.1}",
            limits.max_memory_pressure
        ));
    }
    if let Some(available) = sample.memory_available_mb
        && available < limits.min_available_memory_mb
    {
        reasons.push(format!(
            "available memory {available} MB < {} MB",
            limits.min_available_memory_mb
        ));
    }
    if let Some(free) = sample.disk_free_mb
        && free < limits.min_free_disk_mb
    {
        reasons.push(format!(
            "free disk {free} MB < {} MB",
            limits.min_free_disk_mb
        ));
    }
    reasons
}

fn read_psi(root: &Path, name: &str) -> Option<f64> {
    std::fs::read_to_string(root.join(name))
        .ok()
        .as_deref()
        .and_then(parse_psi_some_avg10)
}

const MB: u64 = 1024 * 1024;

/// Owns the sysinfo handle (CPU usage is a difference between two refreshes)
/// and the latest sample. `sample()` blocks briefly on file reads; call it
/// from a blocking context.
pub struct Monitor {
    disk_path: PathBuf,
    psi_root: PathBuf,
    host: String,
    system: Mutex<Option<sysinfo::System>>,
    latest: Mutex<Sample>,
}
impl Monitor {
    /// `disk_path` is the hub state directory whose free space the gate reads.
    pub fn new(disk_path: PathBuf) -> Self {
        let host = sysinfo::System::host_name()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "local".into());
        Self {
            latest: Mutex::new(Sample {
                disk_path: disk_path.display().to_string(),
                ..Default::default()
            }),
            disk_path,
            psi_root: PathBuf::from("/proc/pressure"),
            host,
            system: Mutex::new(None),
        }
    }
    /// This machine's name, reported as `host` on the wire.
    pub fn host(&self) -> &str {
        &self.host
    }
    /// The most recent sample (all unknown before the first one).
    pub fn latest(&self) -> Sample {
        self.latest.lock().unwrap().clone()
    }
    /// Take a new sample and keep it as the latest.
    pub fn sample(&self) -> Sample {
        let mut sample = Sample {
            sampled_at: chrono::Utc::now().timestamp_millis(),
            disk_path: self.disk_path.display().to_string(),
            ..Default::default()
        };
        {
            let mut guard = self.system.lock().unwrap();
            let first = guard.is_none();
            let system = guard.get_or_insert_with(sysinfo::System::new);
            system.refresh_cpu_usage();
            system.refresh_memory();
            let cores = system.cpus().len() as u64;
            sample.cores = (cores > 0).then_some(cores).or_else(|| {
                std::thread::available_parallelism()
                    .ok()
                    .map(|n| n.get() as u64)
            });
            // The first refresh has nothing to diff against.
            if !first && cores > 0 {
                let usage = system.global_cpu_usage() as f64;
                sample.usage_pct = usage.is_finite().then_some(usage);
            }
            let total = system.total_memory();
            if total > 0 {
                sample.memory_total_mb = Some(total / MB);
                sample.memory_available_mb = Some(system.available_memory() / MB);
            }
        }
        // Windows has no load average; sysinfo reports zeros there.
        if cfg!(unix) {
            let load = sysinfo::System::load_average().one;
            sample.load1 = (load.is_finite() && load >= 0.0).then_some(load);
        }
        if cfg!(target_os = "linux") {
            sample.cpu_pressure = read_psi(&self.psi_root, "cpu");
            sample.memory_pressure = read_psi(&self.psi_root, "memory");
            sample.io_pressure = read_psi(&self.psi_root, "io");
        }
        let disk = if self.disk_path.exists() {
            self.disk_path.clone()
        } else {
            // Not created yet: measure the nearest existing ancestor.
            self.disk_path
                .ancestors()
                .find(|path| path.exists())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.disk_path.clone())
        };
        sample.disk_free_mb = fs2::available_space(&disk).ok().map(|bytes| bytes / MB);
        sample.disk_total_mb = fs2::total_space(&disk).ok().map(|bytes| bytes / MB);
        *self.latest.lock().unwrap() = sample.clone();
        sample
    }
    #[cfg(test)]
    pub(crate) fn set_latest(&self, sample: Sample) {
        *self.latest.lock().unwrap() = sample;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Thresholds {
        Thresholds {
            max_cpu_pressure: 60.0,
            max_memory_pressure: 25.0,
            min_available_memory_mb: 1024,
            min_free_disk_mb: 2048,
        }
    }

    #[test]
    fn psi_reads_some_avg10_and_treats_anything_else_as_unknown() {
        let text = "some avg10=3.10 avg60=1.00 avg300=0.20 total=123\nfull avg10=9.00 avg60=0.00 avg300=0.00 total=0\n";
        assert_eq!(parse_psi_some_avg10(text), Some(3.1));
        // `full` alone (or first) never stands in for `some`.
        assert_eq!(
            parse_psi_some_avg10(
                "full avg10=9.00 avg60=0 avg300=0 total=0\nsome avg10=0.40 avg60=0 avg300=0 total=1"
            ),
            Some(0.4)
        );
        assert_eq!(
            parse_psi_some_avg10("full avg10=9.00 avg60=0 avg300=0 total=0"),
            None
        );
        // The kernel's cpu file has only a `some` line on older kernels.
        assert_eq!(
            parse_psi_some_avg10("some avg10=0.00 avg60=0.00 avg300=0.00 total=0"),
            Some(0.0)
        );
        for broken in [
            "",
            "some",
            "some avg60=1.0",
            "some avg10=abc",
            "some avg10=-1",
            "some avg10=NaN",
        ] {
            assert_eq!(parse_psi_some_avg10(broken), None, "{broken:?}");
        }
    }

    #[test]
    fn each_threshold_closes_the_gate_at_its_boundary() {
        let healthy = Sample {
            cpu_pressure: Some(10.0),
            memory_pressure: Some(1.0),
            memory_available_mb: Some(8000),
            disk_free_mb: Some(50_000),
            ..Default::default()
        };
        assert!(gate_reasons(&healthy, &limits()).is_empty());

        let mut cpu = healthy.clone();
        cpu.cpu_pressure = Some(60.0);
        assert_eq!(gate_reasons(&cpu, &limits()), ["cpu pressure 60.0 >= 60.0"]);
        cpu.cpu_pressure = Some(59.9);
        assert!(gate_reasons(&cpu, &limits()).is_empty());

        let mut memory = healthy.clone();
        memory.memory_pressure = Some(34.0);
        assert_eq!(
            gate_reasons(&memory, &limits()),
            ["memory pressure 34.0 >= 25.0"]
        );

        let mut available = healthy.clone();
        available.memory_available_mb = Some(700);
        assert_eq!(
            gate_reasons(&available, &limits()),
            ["available memory 700 MB < 1024 MB"]
        );
        available.memory_available_mb = Some(1024);
        assert!(gate_reasons(&available, &limits()).is_empty());

        let mut disk = healthy.clone();
        disk.disk_free_mb = Some(2047);
        assert_eq!(
            gate_reasons(&disk, &limits()),
            ["free disk 2047 MB < 2048 MB"]
        );

        let mut all = healthy;
        all.cpu_pressure = Some(99.0);
        all.memory_pressure = Some(99.0);
        all.memory_available_mb = Some(1);
        all.disk_free_mb = Some(1);
        assert_eq!(gate_reasons(&all, &limits()).len(), 4);
    }

    #[test]
    fn unknown_metrics_never_close_the_gate() {
        let strict = Thresholds {
            max_cpu_pressure: 0.0,
            max_memory_pressure: 0.0,
            min_available_memory_mb: u64::MAX,
            min_free_disk_mb: u64::MAX,
        };
        assert!(gate_reasons(&Sample::default(), &strict).is_empty());
    }

    #[test]
    fn a_real_sample_reports_this_machine_and_its_state_disk() {
        let directory = tempfile::tempdir().unwrap();
        let monitor = Monitor::new(directory.path().join("not-created-yet"));
        assert!(!monitor.host().is_empty());
        assert_eq!(monitor.latest().sampled_at, 0);
        let sample = monitor.sample();
        assert!(sample.sampled_at > 0);
        assert!(sample.cores.unwrap_or(1) >= 1);
        assert!(sample.disk_free_mb.is_some() && sample.disk_total_mb.is_some());
        assert!(sample.usage_pct.is_none(), "first sample has no CPU diff");
        assert_eq!(monitor.latest(), sample);
        let wire = sample.wire();
        assert!(
            wire["disk"]["path"]
                .as_str()
                .unwrap()
                .ends_with("not-created-yet")
        );
        assert!(wire["cpu"].get("pressureAvg10").is_some());
        #[cfg(not(target_os = "linux"))]
        assert!(wire["io"]["pressureAvg10"].is_null());
    }
}
