//! Shared, pure formatting for people: durations, clock times, truncation and
//! avatar monograms. UI-independent so every screen (and the tests) formats
//! the same value the same way (`docs/design-v2.md`, "Formatting").
use chrono::{DateTime, Local, Utc};

/// A running timer without a heartbeat for this long has lost its agent.
pub const HEARTBEAT_STALE_MS: i64 = 5 * 60_000;

/// What a stale timer says instead of its duration (in the warning colour).
pub const LOST_HEARTBEAT: &str = "Lost heartbeat";

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
/// Stale timers show [`LOST_HEARTBEAT`] rather than a growing duration.
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

#[cfg(test)]
mod tests {
    use super::*;

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
