//! Self-profiling: what this app costs the machine while it runs, written
//! where the user can read it, so "is it the app or the workload?" has an
//! answer.
//!
//! Hot paths record into lock-free [`Stat`]s (a count, a total and a max);
//! hub calls record per method. A sampler thread wakes every
//! [`interval`] and appends one JSON line to `logs/perf.jsonl` beside
//! `wks-native.log`, with:
//! - this process's CPU and resident memory, split by thread name (the UI
//!   thread vs the backend's tokio workers);
//! - the processes it started (agents, PTY wrappers, shells): CPU and memory,
//!   grouped by command name;
//! - the whole system's CPU, load and available memory;
//! - UI renders, controller steps and hub calls over the interval.
//!
//! One summary line per sample also goes to `wks-native.log`, and slow work
//! warns there (at most once a second per kind). `WKS_PERF=0` turns the
//! sampler off; `WKS_PERF_INTERVAL=<seconds>` changes its period (1-300).
//! The per-process figures read `/proc`, so they are Linux-only; other
//! platforms still log the in-app counters.
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

/// Count, total and max duration of one kind of work since the last sample.
pub struct Stat {
    name: &'static str,
    slow: Duration,
    count: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
    warned_at_ms: AtomicU64,
}

impl Stat {
    const fn new(name: &'static str, slow: Duration) -> Self {
        Self {
            name,
            slow,
            count: AtomicU64::new(0),
            total_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            warned_at_ms: AtomicU64::new(0),
        }
    }

    pub fn record(&self, took: Duration) {
        let us = took.as_micros().min(u64::MAX as u128) as u64;
        self.count.fetch_add(1, Relaxed);
        self.total_us.fetch_add(us, Relaxed);
        self.max_us.fetch_max(us, Relaxed);
        if took >= self.slow && rate_limit(&self.warned_at_ms) {
            tracing::warn!(
                target: "wks_native::perf",
                kind = self.name,
                took_ms = us / 1000,
                "slow {}",
                self.name
            );
        }
    }

    /// Times until the guard drops.
    pub fn time(&'static self) -> Timer {
        Timer(self, Instant::now())
    }

    fn take(&self) -> Value {
        let count = self.count.swap(0, Relaxed);
        let total = self.total_us.swap(0, Relaxed);
        let max = self.max_us.swap(0, Relaxed);
        json!({
            "n": count,
            "avg_ms": if count == 0 { 0. } else { ms(total / count) },
            "max_ms": ms(max),
            "total_ms": ms(total),
        })
    }
}

/// Records into its [`Stat`] when dropped.
pub struct Timer(&'static Stat, Instant);

impl Drop for Timer {
    fn drop(&mut self) {
        self.0.record(self.1.elapsed());
    }
}

/// The main window's root render (the whole Workspace view).
pub static RENDER: Stat = Stat::new("render", Duration::from_millis(32));
/// A terminal view's render.
pub static TERMINAL_RENDER: Stat = Stat::new("terminal render", Duration::from_millis(32));
/// The controller handling one command from the UI.
pub static COMMAND: Stat = Stat::new("controller command", Duration::from_millis(100));
/// The controller folding one backend event.
pub static EVENT: Stat = Stat::new("controller event", Duration::from_millis(100));
/// The controller handling one finished backend call.
pub static COMPLETION: Stat = Stat::new("controller completion", Duration::from_millis(100));
/// The controller publishing a new view to the UI (includes cloning it).
pub static PUBLISH: Stat = Stat::new("view publish", Duration::from_millis(50));
/// The controller's once-a-second maintenance.
pub static MAINTAIN: Stat = Stat::new("controller maintenance", Duration::from_millis(100));

const SLOW_CALL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Calls {
    n: u64,
    errors: u64,
    total_us: u64,
    max_us: u64,
}

static HUB_CALLS: Mutex<Option<HashMap<String, Calls>>> = Mutex::new(None);
static SLOW_CALL_WARNED: AtomicU64 = AtomicU64::new(0);

/// One hub call: its method, how long it took and whether it failed.
pub fn hub_call(method: &str, took: Duration, ok: bool) {
    let us = took.as_micros().min(u64::MAX as u128) as u64;
    if let Ok(mut calls) = HUB_CALLS.lock() {
        let entry = calls
            .get_or_insert_with(HashMap::new)
            .entry(method.to_owned())
            .or_default();
        entry.n += 1;
        entry.errors += u64::from(!ok);
        entry.total_us += us;
        entry.max_us = entry.max_us.max(us);
    }
    if took >= SLOW_CALL && rate_limit(&SLOW_CALL_WARNED) {
        tracing::warn!(target: "wks_native::perf", method, took_ms = us / 1000, ok, "slow hub call");
    }
}

fn take_calls() -> Value {
    let calls = HUB_CALLS
        .lock()
        .ok()
        .and_then(|mut c| c.take())
        .unwrap_or_default();
    let mut out = Map::new();
    let mut sorted: Vec<_> = calls.into_iter().collect();
    sorted.sort_by(|a, b| b.1.total_us.cmp(&a.1.total_us));
    for (method, c) in sorted {
        out.insert(
            method,
            json!({"n": c.n, "errors": c.errors, "avg_ms": ms(c.total_us / c.n.max(1)), "max_ms": ms(c.max_us)}),
        );
    }
    Value::Object(out)
}

fn ms(us: u64) -> f64 {
    (us as f64 / 100.).round() / 10.
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// True at most once a second per `last`.
fn rate_limit(last: &AtomicU64) -> bool {
    let now = epoch_ms();
    let before = last.load(Relaxed);
    now.saturating_sub(before) >= 1000
        && last.compare_exchange(before, now, Relaxed, Relaxed).is_ok()
}

/// The sampling period, or `None` when `WKS_PERF=0`.
pub fn interval() -> Option<Duration> {
    if std::env::var("WKS_PERF").is_ok_and(|v| v == "0" || v.eq_ignore_ascii_case("off")) {
        return None;
    }
    let secs = std::env::var("WKS_PERF_INTERVAL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| (1..=300).contains(s))
        .unwrap_or(10);
    Some(Duration::from_secs(secs))
}

/// Start sampling into `<logs>/perf.jsonl`, keeping the previous run's file as
/// `perf.jsonl.1`. Does nothing when sampling is off or the file can't open.
pub fn start(logs: PathBuf) {
    let Some(every) = interval() else {
        return;
    };
    let path = logs.join("perf.jsonl");
    let _ = std::fs::rename(&path, logs.join("perf.jsonl.1"));
    let Ok(file) = std::fs::File::create(&path) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("wks-perf".into())
        .spawn(move || sample_loop(file, every));
}

fn sample_loop(mut file: std::fs::File, every: Duration) {
    use std::io::Write as _;
    let mut procs = proc::Sampler::new();
    let mut last = Instant::now();
    loop {
        std::thread::sleep(every);
        let wall = last.elapsed();
        last = Instant::now();
        let secs = wall.as_secs_f64().max(0.001);
        let render = RENDER.take();
        let renders = render["n"].as_u64().unwrap_or(0);
        let machine = procs.sample(wall);
        let line = json!({
            "t": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "interval_s": (secs * 10.).round() / 10.,
            "app": machine.app,
            "agents": machine.agents,
            "system": machine.system,
            "ui": {
                "renders_per_s": (renders as f64 / secs * 10.).round() / 10.,
                "render": render,
                "terminal_render": TERMINAL_RENDER.take(),
            },
            "controller": {
                "command": COMMAND.take(),
                "event": EVENT.take(),
                "completion": COMPLETION.take(),
                "publish": PUBLISH.take(),
                "maintain": MAINTAIN.take(),
            },
            "hub_calls": take_calls(),
        });
        tracing::info!(target: "wks_native::perf", "{}", summary(&line));
        if writeln!(file, "{line}")
            .and_then(|()| file.flush())
            .is_err()
        {
            return;
        }
    }
}

/// "app 3.1% cpu 412 MB (main 2.0%, workspacer-worker 1.1%) · agents 164% cpu
/// in 7 procs · system 38% · 20.0 renders/s (avg 4.2 ms)"
fn summary(line: &Value) -> String {
    let pct = |v: &Value| v.as_f64().map_or("?".into(), |p| format!("{p:.1}%"));
    let app = &line["app"];
    let threads = app["cpu_by_thread"]
        .as_object()
        .map(|t| {
            t.iter()
                .take(3)
                .map(|(name, p)| format!("{name} {}", pct(p)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut out = format!(
        "perf: app {} cpu {} MB",
        pct(&app["cpu_pct"]),
        app["rss_mb"].as_u64().map_or("?".into(), |m| m.to_string())
    );
    if !threads.is_empty() {
        out += &format!(" ({threads})");
    }
    let agents = &line["agents"];
    if agents["procs"].as_u64().is_some() {
        out += &format!(
            " · agents {} cpu in {} procs",
            pct(&agents["cpu_pct"]),
            agents["procs"]
        );
    }
    if line["system"]["cpu_pct"].is_number() {
        out += &format!(" · system {}", pct(&line["system"]["cpu_pct"]));
    }
    let ui = &line["ui"];
    out += &format!(
        " · {} renders/s (avg {} ms)",
        ui["renders_per_s"], ui["render"]["avg_ms"]
    );
    out
}

/// The machine-level figures of one sample.
struct Machine {
    app: Value,
    agents: Value,
    system: Value,
}

#[cfg(target_os = "linux")]
mod proc {
    //! `/proc` readings. CPU times are in USER_HZ ticks, which Linux fixes at
    //! 100 a second for userspace on every architecture it ships.
    use super::Machine;
    use serde_json::{Map, Value, json};
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    const HZ: f64 = 100.;

    /// One `/proc/<pid>/stat`: command name, parent, CPU ticks.
    struct Stat {
        comm: String,
        ppid: u32,
        ticks: u64,
    }

    fn stat(path: &str) -> Option<Stat> {
        let text = std::fs::read_to_string(path).ok()?;
        // The name is in parentheses and may itself hold spaces or ')'.
        let open = text.find('(')?;
        let close = text.rfind(')')?;
        let comm = text[open + 1..close].to_owned();
        let rest: Vec<&str> = text[close + 2..].split(' ').collect();
        // Fields after the name, 0-based: state=0 ppid=1 utime=11 stime=12.
        let field = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok());
        Some(Stat {
            comm,
            ppid: field(1)? as u32,
            ticks: field(11)? + field(12)?,
        })
    }

    fn rss_kb(pid: &str) -> u64 {
        std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("VmRSS:"))
                    .and_then(|v| v.split_whitespace().next()?.parse().ok())
            })
            .unwrap_or(0)
    }

    pub(super) struct Sampler {
        me: u32,
        /// Ticks at the last sample, by thread id / process id / "system".
        threads: HashMap<String, u64>,
        children: HashMap<u32, u64>,
        system: Option<(u64, u64)>,
    }

    impl Sampler {
        pub(super) fn new() -> Self {
            let mut sampler = Self {
                me: std::process::id(),
                threads: HashMap::new(),
                children: HashMap::new(),
                system: None,
            };
            // Prime the baselines so the first sample is a real delta.
            sampler.sample(Duration::from_secs(1));
            sampler
        }

        pub(super) fn sample(&mut self, wall: Duration) -> Machine {
            let secs = wall.as_secs_f64().max(0.001);
            let pct = |ticks: u64| ((ticks as f64 / HZ / secs * 100.) * 10.).round() / 10.;

            // This process, by thread name.
            let mut by_thread: HashMap<String, u64> = HashMap::new();
            let mut seen = HashMap::new();
            let mut app_ticks = 0;
            let mut thread_count = 0;
            if let Ok(dir) = std::fs::read_dir("/proc/self/task") {
                for entry in dir.flatten() {
                    let tid = entry.file_name().to_string_lossy().into_owned();
                    let Some(s) = stat(&format!("/proc/self/task/{tid}/stat")) else {
                        continue;
                    };
                    thread_count += 1;
                    let delta = s
                        .ticks
                        .saturating_sub(*self.threads.get(&tid).unwrap_or(&s.ticks));
                    let name = if tid == self.me.to_string() {
                        "main (ui)".to_owned()
                    } else {
                        s.comm
                    };
                    *by_thread.entry(name).or_default() += delta;
                    app_ticks += delta;
                    seen.insert(tid, s.ticks);
                }
            }
            self.threads = seen;
            let mut threads: Vec<_> = by_thread.into_iter().filter(|(_, t)| *t > 0).collect();
            threads.sort_by(|a, b| b.1.cmp(&a.1));
            let cpu_by_thread: Map<String, Value> = threads
                .into_iter()
                .map(|(n, t)| (n, json!(pct(t))))
                .collect();
            let app = json!({
                "pid": self.me,
                "cpu_pct": pct(app_ticks),
                "rss_mb": rss_kb("self") / 1024,
                "threads": thread_count,
                "cpu_by_thread": cpu_by_thread,
            });

            // Every descendant: agents, their wrappers and whatever they run.
            let mut all = HashMap::new();
            if let Ok(dir) = std::fs::read_dir("/proc") {
                for entry in dir.flatten() {
                    let name = entry.file_name();
                    let Some(pid) = name.to_str().and_then(|p| p.parse::<u32>().ok()) else {
                        continue;
                    };
                    if let Some(s) = stat(&format!("/proc/{pid}/stat")) {
                        all.insert(pid, s);
                    }
                }
            }
            let mut mine = HashSet::from([self.me]);
            let mut grew = true;
            while grew {
                grew = false;
                for (pid, s) in &all {
                    if !mine.contains(pid) && mine.contains(&s.ppid) {
                        mine.insert(*pid);
                        grew = true;
                    }
                }
            }
            mine.remove(&self.me);
            let mut by_name: HashMap<String, (u64, u64, u64)> = HashMap::new();
            let mut children = HashMap::new();
            let (mut agent_ticks, mut agent_kb) = (0, 0);
            for pid in &mine {
                let s = &all[pid];
                // A process new since the last sample counts from its start.
                let delta = s
                    .ticks
                    .saturating_sub(*self.children.get(pid).unwrap_or(&0));
                let kb = rss_kb(&pid.to_string());
                let e = by_name.entry(s.comm.clone()).or_default();
                e.0 += 1;
                e.1 += delta;
                e.2 += kb;
                agent_ticks += delta;
                agent_kb += kb;
                children.insert(*pid, s.ticks);
            }
            self.children = children;
            let mut names: Vec<_> = by_name.into_iter().collect();
            names.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(b.1.2.cmp(&a.1.2)));
            let by_name: Map<String, Value> = names
                .into_iter()
                .map(|(n, (count, t, kb))| {
                    (
                        n,
                        json!({"procs": count, "cpu_pct": pct(t), "rss_mb": kb / 1024}),
                    )
                })
                .collect();
            let agents = json!({
                "procs": mine.len(),
                "cpu_pct": pct(agent_ticks),
                "rss_mb": agent_kb / 1024,
                "by_name": by_name,
            });

            Machine {
                app,
                agents,
                system: self.system(),
            }
        }

        fn system(&mut self) -> Value {
            let mut out = Map::new();
            if let Some(line) = std::fs::read_to_string("/proc/stat")
                .ok()
                .and_then(|s| s.lines().next().map(str::to_owned))
            {
                let v: Vec<u64> = line
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|x| x.parse().ok())
                    .collect();
                let total: u64 = v.iter().take(8).sum();
                let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
                if let Some((t0, i0)) = self.system.replace((total, idle)) {
                    let dt = total.saturating_sub(t0);
                    if dt > 0 {
                        let busy = 1. - idle.saturating_sub(i0) as f64 / dt as f64;
                        out.insert("cpu_pct".into(), json!((busy * 1000.).round() / 10.));
                    }
                }
            }
            out.insert(
                "cores".into(),
                json!(std::thread::available_parallelism().map_or(0, |n| n.get())),
            );
            if let Some(load) = std::fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            {
                out.insert("load1".into(), json!(load));
            }
            if let Some(kb) = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("MemAvailable:"))
                    .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            }) {
                out.insert("mem_available_mb".into(), json!(kb / 1024));
            }
            Value::Object(out)
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn reads_its_own_stat() {
            let s = super::stat("/proc/self/stat").expect("own stat parses");
            assert!(!s.comm.is_empty());
            assert!(s.ppid > 0);
        }

        #[test]
        fn samples_itself_and_a_child() {
            let mut child = std::process::Command::new("sh")
                .args([
                    "-c",
                    "i=0; while [ $i -lt 200000 ]; do i=$((i+1)); done; sleep 2",
                ])
                .spawn()
                .expect("sh runs");
            let mut sampler = super::Sampler::new();
            std::thread::sleep(std::time::Duration::from_millis(600));
            let m = sampler.sample(std::time::Duration::from_millis(600));
            let _ = child.kill();
            let _ = child.wait();
            assert!(m.app["rss_mb"].as_u64().unwrap() > 0);
            assert!(m.app["threads"].as_u64().unwrap() >= 1);
            assert!(m.agents["procs"].as_u64().unwrap() >= 1, "{}", m.agents);
            assert!(m.agents["by_name"]["sh"].is_object(), "{}", m.agents);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod proc {
    use super::Machine;
    use serde_json::json;
    use std::time::Duration;

    pub(super) struct Sampler;

    impl Sampler {
        pub(super) fn new() -> Self {
            Self
        }
        pub(super) fn sample(&mut self, _: Duration) -> Machine {
            Machine {
                app: json!({"pid": std::process::id()}),
                agents: json!({}),
                system: json!({}),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_reset_after_each_sample() {
        static S: Stat = Stat::new("test", Duration::from_secs(60));
        S.record(Duration::from_millis(4));
        S.record(Duration::from_millis(10));
        let v = S.take();
        assert_eq!(v["n"], 2);
        assert_eq!(v["max_ms"], 10.0);
        assert_eq!(v["avg_ms"], 7.0);
        assert_eq!(S.take()["n"], 0);
    }

    #[test]
    fn hub_calls_aggregate_by_method() {
        hub_call("perf.test.a", Duration::from_millis(30), true);
        hub_call("perf.test.a", Duration::from_millis(10), false);
        let v = take_calls();
        assert_eq!(v["perf.test.a"]["n"], 2);
        assert_eq!(v["perf.test.a"]["errors"], 1);
        assert_eq!(v["perf.test.a"]["max_ms"], 30.0);
    }

    #[test]
    fn summary_reads_a_sample() {
        let line = json!({
            "app": {"cpu_pct": 3.1, "rss_mb": 412, "cpu_by_thread": {"main (ui)": 2.0}},
            "agents": {"procs": 7, "cpu_pct": 164.0},
            "system": {"cpu_pct": 38.0},
            "ui": {"renders_per_s": 20.0, "render": {"avg_ms": 4.2}},
        });
        assert_eq!(
            summary(&line),
            "perf: app 3.1% cpu 412 MB (main (ui) 2.0%) · agents 164.0% cpu in 7 procs · system 38.0% · 20.0 renders/s (avg 4.2 ms)"
        );
    }
}
