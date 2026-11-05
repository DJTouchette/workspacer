//! Shared, pure formatting for people: durations, clock times, truncation and
//! avatar monograms. UI-independent so every screen (and the tests) formats
//! the same value the same way (`docs/design-v2.md`, "Formatting").
use chrono::{DateTime, Local, Utc};

/// A running timer without a heartbeat for this long has lost its agent:
/// it reads [`NOT_RESPONDING`] (R3.1 retired "Lost heartbeat" and "stalled").
pub const HEARTBEAT_STALE_MS: i64 = 5 * 60_000;

/// A duration for people: `"850ms"`, `"42s"`, `"12m 05s"`, `"5h 12m"`, and
/// from 48 hours on `"34d 3h"` (never `"819h 09m"`). Negative reads as zero.
pub fn duration(milliseconds: i64) -> String {
    let milliseconds = milliseconds.max(0);
    if milliseconds < 1000 {
        return format!("{milliseconds}ms");
    }
    let seconds = milliseconds / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        3600..172_800 => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
        _ => format!("{}d {}h", seconds / 86_400, seconds % 86_400 / 3600),
    }
}

/// Whether a timer last heard from at `last_beat` (ms) is stale at `now`:
/// over [`HEARTBEAT_STALE_MS`] without a beat, or never beaten at all.
/// Stale timers show [`not_responding`] rather than a growing duration.
pub fn is_stale(last_beat: Option<i64>, now: i64) -> bool {
    stale_after(last_beat, now, HEARTBEAT_STALE_MS)
}

/// [`is_stale`] with an explicit threshold (ms).
pub fn stale_after(last_beat: Option<i64>, now: i64, threshold: i64) -> bool {
    last_beat.is_none_or(|beat| now.saturating_sub(beat) > threshold)
}

/// A timestamp without seconds, in local time: `"14:07"` today, else
/// `"Oct 14 · 14:07"` (`"2:07 PM"` on a 12-hour clock). Unknown times read
/// `"Time unavailable"`.
pub fn timestamp(at: Option<i64>, now: i64, twelve_hour: bool) -> String {
    let local = |ms| DateTime::<Utc>::from_timestamp_millis(ms).map(|t| t.with_timezone(&Local));
    let Some(time) = at.and_then(local) else {
        return "Time unavailable".into();
    };
    let today = local(now).map(|t| t.date_naive()) == Some(time.date_naive());
    time.format(match (today, twelve_hour) {
        (true, false) => "%H:%M",
        (false, false) => "%b %-d · %H:%M",
        (true, true) => "%-I:%M %p",
        (false, true) => "%b %-d · %-I:%M %p",
    })
    .to_string()
}

/// `text` cut to at most `max` characters with a trailing ellipsis, or
/// `None` when it already fits (then show it whole, with no tooltip).
pub fn truncate_end(text: &str, max: usize) -> Option<String> {
    if text.chars().count() <= max {
        return None;
    }
    let keep = max.saturating_sub(1);
    let mut cut: String = text.chars().take(keep).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    Some(cut)
}

/// `text` cut in the middle to at most `max` characters, keeping both ends
/// (paths, branch names, ids), or `None` when it fits.
pub fn truncate_middle(text: &str, max: usize) -> Option<String> {
    let count = text.chars().count();
    if count <= max {
        return None;
    }
    let keep = max.saturating_sub(1);
    let tail = keep / 2;
    let head = keep - tail;
    let mut out: String = text.chars().take(head).collect();
    out.push('…');
    out.extend(text.chars().skip(count - tail));
    Some(out)
}

/// Up to two letters for an avatar tile: the initials of the first two
/// words ("Hub RS" → "HR"), else the first two letters ("workspacer" →
/// "WO"). Punctuation and separators split words; empty names give "?".
pub fn monogram(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => return "?".into(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    letters.to_uppercase()
}

/// Which of `tones` avatar fills a name gets: a stable FNV-1a hash of the
/// lowercased name, so a project keeps its colour across runs and machines.
pub fn avatar_index(name: &str, tones: usize) -> usize {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.trim().to_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash as usize % tones.max(1)
}

// area-shell
/// How long ago `at` (epoch ms) was, as a sidebar row's compact age: "now",
/// "42s", "12m", "5h", "3d", "6w". Future stamps (clock skew) read as "now".
pub fn age(at: i64, now: i64) -> String {
    let seconds = (now - at).max(0) / 1000;
    match seconds {
        s if s < 10 => "now".into(),
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s if s < 14 * 86_400 => format!("{}d", s / 86_400),
        s => format!("{}w", s / (7 * 86_400)),
    }
}

// r8-attention (R8.1): one time format per meaning, everywhere.
/// Recency in a sentence or a cell: "23m ago", "just now" (under 10s),
/// with [`age`]'s thresholds, so a sidebar row's "23m" and History's
/// "23m ago" are the same fact in the same units.
pub fn ago(at: i64, now: i64) -> String {
    match age(at, now).as_str() {
        "now" => "just now".into(),
        age => format!("{age} ago"),
    }
}

/// Recency with its meaning named: "Last active 23m ago".
pub fn last_active(at: i64, now: i64) -> String {
    format!("Last active {}", ago(at, now))
}

/// How long something ran: "Ran 3m 31s" ([`duration`]).
pub fn ran(milliseconds: i64) -> String {
    format!("Ran {}", duration(milliseconds))
}

/// r8-attention (R8.6): one name per agent, everywhere an agent (not an
/// account's usage) is named: "Claude Code", "Codex CLI". Unknown ids keep
/// their own name; none reads "Agent".
pub fn agent_name(provider: &str) -> &str {
    match provider {
        "claude" => "Claude Code",
        "codex" => "Codex CLI",
        "" => "Agent",
        other => other,
    }
}

/// r8-attention (R8.6): a subagent's built-in role in plain words: Claude
/// Code's "Explore" reads "Reading code", "Plan" reads "Planning", and the
/// catch-all "general-purpose" says nothing. Other roles keep their name.
pub fn subagent_role(role: &str) -> &str {
    match role.trim() {
        "Explore" | "explore" => "Reading code",
        "Plan" | "plan" => "Planning",
        "general-purpose" => "",
        other => other,
    }
}

// r2-systems
/// What a stalled agent says instead of "Working": a fault, never also
/// running (`docs/design-v2-direction.md` R2.1).
pub const NOT_RESPONDING: &str = "Not responding";

/// "Not responding · 2m" for an agent last heard from at `last_beat` (ms);
/// plain "Not responding" when it never beat.
pub fn not_responding(last_beat: Option<i64>, now: i64) -> String {
    match last_beat {
        Some(beat) => format!("{NOT_RESPONDING} · {}", age(beat, now)),
        None => NOT_RESPONDING.to_owned(),
    }
}

/// A run of agent and tool events, for the transcript's group header:
/// "Worked 2m 14s · 18 tools" ("1 tool"; no count when there were none).
pub fn worked_summary(elapsed_ms: i64, tools: usize) -> String {
    let worked = format!("Worked {}", duration(elapsed_ms));
    match tools {
        0 => worked,
        1 => format!("{worked} · 1 tool"),
        n => format!("{worked} · {n} tools"),
    }
}

/// The hue (OKLCH degrees) of a name's avatar tile: the same stable hash as
/// [`avatar_index`], spread over the wheel with the purple band (280–335°)
/// left out.
pub fn avatar_hue(name: &str) -> f32 {
    const SKIP_FROM: f32 = 280.;
    const SKIP: f32 = 55.;
    let steps = 360 - SKIP as usize;
    let hue = avatar_index(name, steps) as f32;
    if hue >= SKIP_FROM { hue + SKIP } else { hue }
}

/// A model id for people: "Sonnet 4.6", "GPT-5.6 Luna", the raw id when it
/// is not recognised. The one formatter for chips, rows and popovers; show
/// the raw id on hover. An empty id reads "Default model" (r2-shell).
pub fn model_name(id: &str) -> String {
    let id = id.trim();
    if id.is_empty() {
        return "Default model".into();
    }
    crate::model::model_display_name(id)
}

// r3-foundation
/// How loud a share of a limit is (usage, context): quiet below
/// `signals().warn_at` (80%), attention from there, fault at the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Fault,
}

/// The [`Level`] of `pct` percent (0–100+).
pub fn level(pct: f64) -> Level {
    let s = crate::tokens::Tokens::V2.signals();
    // Whole percents, so 80% is 80% whatever f32 makes of 0.8.
    let percent = |share: f32| (f64::from(share) * 100.).round();
    if pct >= percent(s.fault_at) {
        Level::Fault
    } else if pct >= percent(s.warn_at) {
        Level::Warn
    } else {
        Level::Normal
    }
}

/// Time left until something happens, for people: `"45s"`, `"13m"`,
/// `"2h 13m"`, from 48 hours `"3d 4h"`. Past or now reads `"now"`.
pub fn countdown(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1000;
    match seconds {
        0 => "now".into(),
        1..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..172_800 => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
        _ => format!("{}d {}h", seconds / 86_400, seconds % 86_400 / 3600),
    }
}

/// The usage meter's sentence (R3.5, r4-calm R4.6): "Claude · 42% of
/// 5-hour limit used · resets in 2h 13m". `limit` is the window's long name ("5-hour", "Weekly"),
/// `resets_in` the milliseconds until it resets, when known.
pub fn usage_line(account: &str, limit: &str, pct: f64, resets_in: Option<i64>) -> String {
    let pct = pct.max(0.).round() as u64;
    let limit = limit.to_lowercase();
    let mut line = format!("{account} · {pct}% of {limit} limit used");
    if let Some(ms) = resets_in {
        line.push_str(" · resets ");
        if ms <= 0 {
            line.push_str("now");
        } else {
            line.push_str("in ");
            line.push_str(&countdown(ms));
        }
    }
    line
}

/// The composer's context gauge label (R3.2/R3.5, r4-calm R4.6, r7-status
/// R7.4): "Context 42% used", never "ctx" or a bare share. Colour it only
/// from [`Level`] (`level(pct)`).
pub fn context_label(pct: Option<f64>) -> String {
    match pct {
        Some(pct) => format!("Context {}% used", pct.max(0.).round() as u64),
        None => format!("Context {EMPTY}"),
    }
}

// r7-status (R7.4, R7.8)
/// The stall recovery's verb in plain words (R7.4): it sends a short line
/// asking the agent to carry on.
pub const NUDGE: &str = "Nudge (ask to continue)";

/// The toolbar label and panel title for a session's background work
/// (R7.4): one name for both.
pub const BACKGROUND_TASKS: &str = "Background tasks";

/// The one mark for "no value" in a cell or a meta line: an em dash.
pub const EMPTY: &str = "—";

/// The one separator between meta facts: a middle dot with a space each
/// side.
pub const SEP: &str = " · ";

/// The one join helper for meta text (R7.8): trims each part, drops the
/// empty ones and any stray leading or trailing "·", and joins the rest
/// with [`SEP`]. `meta(["", " Sonnet 4.6", "· 3 tools"])` is
/// "Sonnet 4.6 · 3 tools", never "· Sonnet 4.6  · 3 tools".
pub fn meta<I, S>(parts: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parts
        .into_iter()
        .filter_map(|part| {
            let part = part.as_ref().trim().trim_matches('·').trim().to_owned();
            (!part.is_empty()).then_some(part)
        })
        .collect::<Vec<_>>()
        .join(SEP)
}

/// A usage window's short name for a meter's label: "5h", "Weekly",
/// "Monthly" (the hub's "Week" reads "Weekly").
pub fn window_name(label: &str) -> &str {
    match label {
        "Week" | "7d" => "Weekly",
        "Month" => "Monthly",
        other => other,
    }
}

/// r8-attention (R8.6): the sidebar's usage line, said in full: "Codex ·
/// 93% of 5h limit used" (the window's short name). [`resets_in`] follows
/// it.
pub fn usage_words(account: &str, window: &str, pct: f64) -> String {
    format!(
        "{account} · {}% of {} limit used",
        pct.max(0.).round() as u64,
        window_name(window)
    )
}

/// "resets in 2h 13m", "resets now".
pub fn resets_in(milliseconds: i64) -> String {
    if milliseconds <= 0 {
        "resets now".into()
    } else {
        format!("resets in {}", countdown(milliseconds))
    }
}

/// The sidebar's usage line names its window (R7.4): "Claude · 5h limit
/// 42%".
pub fn usage_short(account: &str, window: &str, pct: f64) -> String {
    format!(
        "{account} · {} limit {}%",
        window_name(window),
        pct.max(0.).round() as u64
    )
}

/// The red chip that names what broke (R3.1): "cargo test failed". A
/// command keeps the words before its first flag ("cargo test --locked" →
/// "cargo test"); the name is then cut to `max` characters. Show the whole
/// name in a tooltip.
pub fn failed_label(name: &str, max: usize) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "A task failed".into();
    }
    let words: Vec<&str> = name
        .split_whitespace()
        .take_while(|w| !w.starts_with('-') && !matches!(*w, "&&" | "|" | ";" | ">"))
        .collect();
    let short = if words.is_empty() {
        name.to_owned()
    } else {
        words.join(" ")
    };
    let short = truncate_end(&short, max).unwrap_or(short);
    format!("{short} failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    // r3-foundation
    #[test]
    fn limits_read_as_sentences_and_thresholds() {
        assert_eq!(level(42.), Level::Normal);
        assert_eq!(level(79.6), Level::Normal);
        assert_eq!(level(80.), Level::Warn);
        assert_eq!(level(99.9), Level::Warn);
        assert_eq!(level(100.), Level::Fault);
        assert_eq!(countdown(-1), "now");
        assert_eq!(countdown(45_000), "45s");
        assert_eq!(countdown(13 * 60_000 + 5_000), "13m");
        assert_eq!(countdown((2 * 3600 + 13 * 60) * 1000), "2h 13m");
        assert_eq!(countdown((76 * 3600) * 1000), "3d 4h");
        assert_eq!(
            usage_line("Claude", "5-hour", 42.2, Some((2 * 3600 + 13 * 60) * 1000)),
            "Claude · 42% of 5-hour limit used · resets in 2h 13m"
        );
        assert_eq!(
            usage_line("Codex", "Weekly", 93., None),
            "Codex · 93% of weekly limit used"
        );
        assert_eq!(context_label(Some(41.6)), "Context 42% used");
        assert_eq!(context_label(None), "Context —");
        assert_eq!(
            meta(["", " Sonnet 4.6", "· 3 tools ", "  "]),
            "Sonnet 4.6 · 3 tools"
        );
        assert_eq!(meta(["Worked 2m", "3 tools  ·"]), "Worked 2m · 3 tools");
        assert_eq!(meta(Vec::<String>::new()), "");
        assert_eq!(usage_short("Claude", "5h", 41.6), "Claude · 5h limit 42%");
        // r8-attention (R8.6)
        assert_eq!(
            usage_words("Codex", "5h", 92.6),
            "Codex · 93% of 5h limit used"
        );
        assert_eq!(resets_in(2 * 3_600_000), "resets in 2h 0m");
        assert_eq!(resets_in(0), "resets now");
        assert_eq!(
            usage_short("Codex", "Week", 12.),
            "Codex · Weekly limit 12%"
        );
        assert_eq!(failed_label("cargo test", 32), "cargo test failed");
        assert_eq!(
            failed_label("cargo test --locked -- --test-threads=1", 32),
            "cargo test failed"
        );
        assert_eq!(
            failed_label("npm run build && npm test", 32),
            "npm run build failed"
        );
        assert_eq!(failed_label("--help", 32), "--help failed");
        assert_eq!(
            failed_label("a very long command line indeed", 10),
            "a very lo… failed"
        );
        assert_eq!(failed_label(" ", 10), "A task failed");
    }

    // r2-systems
    #[test]
    fn stalls_workers_and_avatar_hues_read_right() {
        let now = 1_000_000_000_000;
        assert_eq!(
            not_responding(Some(now - 2 * 60_000), now),
            "Not responding · 2m"
        );
        assert_eq!(not_responding(None, now), "Not responding");
        assert_eq!(worked_summary(134_000, 18), "Worked 2m 14s · 18 tools");
        assert_eq!(worked_summary(42_000, 1), "Worked 42s · 1 tool");
        assert_eq!(worked_summary(42_000, 0), "Worked 42s");
        for name in [
            "workspacer",
            "hub-rs",
            "inbox",
            "docs",
            "web",
            "native",
            "a",
            "zz",
        ] {
            let hue = avatar_hue(name);
            assert!((0. ..360.).contains(&hue), "{name} {hue}");
            assert!(!(280. ..335.).contains(&hue), "{name} is purple ({hue})");
        }
        assert_eq!(avatar_hue("Workspacer "), avatar_hue("workspacer"));
        assert_eq!(model_name("claude-sonnet-4-6"), "Sonnet 4.6");
        assert_eq!(model_name("gpt-5.6-luna"), "GPT-5.6 Luna");
    }

    // r2-shell
    #[test]
    fn model_names_are_human() {
        assert_eq!(model_name("claude-sonnet-4-6"), "Sonnet 4.6");
        assert_eq!(model_name("gpt-5.6-luna"), "GPT-5.6 Luna");
        assert_eq!(model_name("claude-opus-4-1-20250805"), "Opus 4.1");
        assert_eq!(model_name("anthropic/claude-haiku-4-5[1m]"), "Haiku 4.5");
        assert_eq!(model_name("opus"), "Opus");
        assert_eq!(model_name("  "), "Default model");
        assert_eq!(model_name("my-local-model"), "my-local-model");
    }

    // area-shell
    #[test]
    fn ages_are_compact_and_never_negative() {
        let now = 1_000_000_000_000;
        assert_eq!(age(now + 5_000, now), "now");
        assert_eq!(age(now - 3_000, now), "now");
        assert_eq!(age(now - 42_000, now), "42s");
        assert_eq!(age(now - 12 * 60_000, now), "12m");
        assert_eq!(age(now - 5 * 3_600_000, now), "5h");
        assert_eq!(age(now - 3 * 86_400_000, now), "3d");
        assert_eq!(age(now - 30 * 86_400_000, now), "4w");
    }

    // r8-attention (R8.1)
    #[test]
    fn one_time_format_per_meaning() {
        let now = 1_000_000_000_000;
        assert_eq!(ago(now - 3_000, now), "just now");
        assert_eq!(ago(now - 23 * 60_000, now), "23m ago");
        assert_eq!(last_active(now - 23 * 60_000, now), "Last active 23m ago");
        assert_eq!(ran(211_000), "Ran 3m 31s");
    }

    #[test]
    fn durations_read_like_people_write_them() {
        assert_eq!(duration(-5), "0ms");
        assert_eq!(duration(850), "850ms");
        assert_eq!(duration(42_000), "42s");
        assert_eq!(duration(725_000), "12m 05s");
        assert_eq!(duration((5 * 3600 + 12 * 60) * 1000), "5h 12m");
        // Up to 48 hours stays in hours…
        assert_eq!(duration((47 * 3600 + 59 * 60) * 1000), "47h 59m");
        // …then days: 819h 09m is 34 days 3 hours.
        assert_eq!(duration((819 * 3600 + 9 * 60) * 1000), "34d 3h");
        assert_eq!(duration(48 * 3600 * 1000), "2d 0h");
    }

    #[test]
    fn a_timer_without_a_heartbeat_goes_stale() {
        let now = 10 * 60_000;
        assert!(!is_stale(Some(now - 1000), now));
        assert!(!is_stale(Some(now - HEARTBEAT_STALE_MS), now));
        assert!(is_stale(Some(now - HEARTBEAT_STALE_MS - 1), now));
        assert!(is_stale(None, now));
        assert!(!stale_after(Some(now - 30_000), now, 60_000));
        assert!(stale_after(Some(i64::MIN), now, 60_000));
    }

    #[test]
    fn timestamps_never_show_seconds() {
        let now = Utc::now().timestamp_millis();
        for twelve in [false, true] {
            let today = timestamp(Some(now), now, twelve);
            assert_eq!(today.matches(':').count(), 1, "{today}");
            assert!(!today.contains('·'), "{today}");
            let earlier = timestamp(Some(now - 40 * 86_400_000), now, twelve);
            assert_eq!(earlier.matches(':').count(), 1, "{earlier}");
            assert!(earlier.contains(" · "), "{earlier}");
        }
        assert_eq!(timestamp(None, now, false), "Time unavailable");
    }

    #[test]
    fn truncation_marks_what_it_cut() {
        assert_eq!(truncate_end("short", 10), None);
        assert_eq!(truncate_end("exactly10!", 10), None);
        assert_eq!(
            truncate_end("a long session title", 8).as_deref(),
            Some("a long…")
        );
        assert_eq!(truncate_end("héllo wörld", 6).as_deref(), Some("héllo…"));
        assert_eq!(truncate_middle("src/ui/primitives.rs", 20), None);
        let cut = truncate_middle("apps/native/src/ui/primitives.rs", 15).unwrap();
        assert_eq!(cut.chars().count(), 15);
        assert!(
            cut.starts_with("apps/na") && cut.ends_with("ives.rs"),
            "{cut}"
        );
    }

    #[test]
    fn monograms_and_avatar_tones_are_stable() {
        assert_eq!(monogram("workspacer"), "WO");
        assert_eq!(monogram("hub-rs"), "HR");
        assert_eq!(monogram("Inbox zero  project"), "IZ");
        assert_eq!(monogram("🚀 launch"), "LA");
        assert_eq!(monogram("  "), "?");
        assert_eq!(
            avatar_index("workspacer", 6),
            avatar_index(" Workspacer ", 6)
        );
        assert!(avatar_index("anything", 6) < 6);
        assert_eq!(avatar_index("x", 0), 0);
        let spread: std::collections::HashSet<_> =
            ["alpha", "beta", "gamma", "delta", "hub", "web"]
                .iter()
                .map(|n| avatar_index(n, 6))
                .collect();
        assert!(spread.len() >= 3, "names cluster on {spread:?}");
    }
}
