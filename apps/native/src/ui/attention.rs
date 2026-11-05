//! Design v2's attention-first session list: which group a session belongs
//! to (Needs you, Working, Idle, Ended) and the order the sidebar, `g n` and
//! j/k walk them in. Pure: the sidebar draws what this returns.
use wks_native::model::Session;

/// The sidebar's groups, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Bucket {
    /// An approval or a question is waiting on the user.
    NeedsYou,
    /// r7-status (R7.3): a finished turn left uncommitted changes. A
    /// Needs-you state with its own band, under Needs you.
    Review,
    /// Starting or mid-turn.
    Working,
    /// Ready for the next message, or paused (resumable).
    Idle,
    /// Ended on purpose; folded away by default.
    Ended,
}

impl Bucket {
    pub(crate) const ALL: [Self; 5] = [
        Self::NeedsYou,
        Self::Review,
        Self::Working,
        Self::Idle,
        Self::Ended,
    ];

    /// r7-status (R7.3): the groups that ask for the user: Needs you and
    /// Ready for review. "All clear" means neither has a member.
    pub(crate) fn wants_you(self) -> bool {
        matches!(self, Self::NeedsYou | Self::Review)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Review => "Ready for review",
            Self::Working => "Working",
            Self::Idle => "Idle",
            Self::Ended => "Ended",
        }
    }

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    /// The selector suffix for the group's header.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs-you",
            Self::Review => "review",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Ended => "ended",
        }
    }
}

/// An approval or a question is waiting on the user.
pub(crate) fn needs_you(session: &Session) -> bool {
    session.approval.is_some()
        || session.questions.is_some()
        || matches!(
            session.state.as_str(),
            "approval" | "question" | "waiting_approval" | "waiting_input"
        )
}

/// A session tree in display order (`(index, depth)`, roots at depth 0) cut
/// into groups. Each top-level tree goes where its most urgent member
/// belongs, so children stay under their parent; `Needs you` puts the
/// longest-waiting tree first (`waiting_since`, epoch ms; unknown last), the
/// rest keep the hub's order. Empty groups are left out.
pub(crate) fn group(
    tree: &[(usize, usize)],
    bucket_of: impl Fn(usize) -> Bucket,
    waiting_since: impl Fn(usize) -> Option<i64>,
) -> Vec<(Bucket, Vec<(usize, usize)>)> {
    let mut trees: Vec<Vec<(usize, usize)>> = Vec::new();
    for &(ix, depth) in tree {
        match trees.last_mut() {
            Some(current) if depth > 0 => current.push((ix, depth)),
            _ => trees.push(vec![(ix, depth)]),
        }
    }
    // Each group's trees, with when their longest wait began.
    type Waiting = (Option<i64>, Option<i64>, Vec<(usize, usize)>);
    let mut groups: Vec<(Bucket, Vec<Waiting>)> =
        Bucket::ALL.iter().map(|b| (*b, Vec::new())).collect();
    for members in trees {
        let bucket = members
            .iter()
            .map(|&(ix, _)| bucket_of(ix))
            .min()
            .unwrap_or(Bucket::Idle);
        let since = members
            .iter()
            .filter(|&&(ix, _)| bucket_of(ix) == Bucket::NeedsYou)
            .filter_map(|&(ix, _)| waiting_since(ix))
            .min();
        // r2-shell: the latest activity anywhere in the tree.
        let latest = members
            .iter()
            .filter_map(|&(ix, _)| waiting_since(ix))
            .max();
        groups[bucket.index()].1.push((since, latest, members));
    }
    // Oldest wait first; a tree with no known wait keeps its place after them.
    groups[Bucket::NeedsYou.index()]
        .1
        .sort_by_key(|(since, _, _)| since.unwrap_or(i64::MAX));
    // r2-shell: Idle and Ended put the most recent first (unknown last).
    for quiet in [Bucket::Review, Bucket::Idle, Bucket::Ended] {
        groups[quiet.index()]
            .1
            .sort_by_key(|(_, latest, _)| std::cmp::Reverse(latest.unwrap_or(i64::MIN)));
    }
    groups
        .into_iter()
        .filter(|(_, trees)| !trees.is_empty())
        .map(|(bucket, trees)| {
            (
                bucket,
                trees
                    .into_iter()
                    .flat_map(|(_, _, members)| members)
                    .collect(),
            )
        })
        .collect()
}

/// The next session that needs the user after `current` (wrapping), in
/// the order the list shows them; `None` when nothing is waiting.
pub(crate) fn next_needing(
    order: &[usize],
    needing: &[usize],
    current: Option<usize>,
) -> Option<usize> {
    if needing.is_empty() {
        return None;
    }
    let start = current
        .and_then(|c| order.iter().position(|&ix| ix == c))
        .map_or(0, |at| at + 1);
    (0..order.len())
        .map(|step| order[(start + step) % order.len()])
        .find(|ix| needing.contains(ix))
        .or_else(|| needing.first().copied())
}

// ── r2-shell: the inbox ────────────────────────────────────────────────

/// A running subagent that has gone quiet past the heartbeat threshold:
/// "Not responding", a fault, never also "Working".
pub(crate) fn child_stalled(child: &wks_native::child_agents::ChildAgent, now: i64) -> bool {
    child.running()
        && wks_native::format::is_stale(
            child
                .telemetry
                .last_activity_ms
                .or(child.telemetry.started_at_ms),
            now,
        )
}

/// What a waiting row offers inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InboxAction {
    /// A tool waits for permission (the request is named in the reason).
    /// r3-shell: its verb is "Review", which opens the session at the
    /// request; the sidebar never allows an unseen command blind.
    Allow,
    /// Open the session at its question.
    Answer,
    /// Open the session (plans, stalls, failures).
    Open,
    /// r7-status (R7.3): open a finished session to review its changes.
    Review,
}

impl InboxAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Allow => "Review",
            Self::Answer => "Answer",
            Self::Open => "Open",
            Self::Review => "Review",
        }
    }
}

/// Why a session is in Needs you, in a few words, and what to do about it.
/// `fault` marks reasons drawn in the fault colour (stalls, failures).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reason {
    pub text: String,
    pub action: InboxAction,
    pub fault: bool,
}

/// The reason a waiting session gives, in the status order (R4.1): a
/// failure first, then a decision, then a stall. `stalled_for` is how long
/// its quietest stalled subagent (or itself) has been silent, in ms;
/// `failure` names its newest open failure ("cargo test failed").
pub(crate) fn reason(
    session: &Session,
    stalled_for: Option<i64>,
    failure: Option<String>,
) -> Option<Reason> {
    if let Some(failure) = failure {
        return Some(Reason {
            text: failure,
            action: InboxAction::Open,
            fault: true,
        });
    }
    if let Some(approval) = session.approval.as_ref() {
        let tool = approval
            .get("toolName")
            .or_else(|| approval.get("tool"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if tool == "ExitPlanMode" {
            return Some(Reason {
                text: "Plan ready for review".into(),
                action: InboxAction::Open,
                fault: false,
            });
        }
        let target = approval
            .pointer("/toolInput/command")
            .or_else(|| approval.pointer("/toolInput/file_path"))
            .or_else(|| approval.pointer("/toolInput/path"))
            .or_else(|| approval.pointer("/toolInput/url"))
            .and_then(serde_json::Value::as_str)
            .and_then(|s| s.lines().next())
            .map(str::trim)
            .unwrap_or("");
        let file = target.rsplit('/').next().unwrap_or(target);
        let text = match tool {
            "Bash" | "shell" | "exec" if !target.is_empty() => format!("Run {target}?"),
            "Edit" | "MultiEdit" | "Write" | "apply_patch" if !file.is_empty() => {
                format!("Edit {file}?")
            }
            "WebFetch" if !target.is_empty() => format!("Open {target}?"),
            "" => "Approval needed".into(),
            other if !target.is_empty() => format!("{other}: {target}?"),
            other => format!("Use {other}?"),
        };
        return Some(Reason {
            text,
            action: InboxAction::Allow,
            fault: false,
        });
    }
    if session.questions.is_some() || matches!(session.state.as_str(), "question" | "waiting_input")
    {
        let header = session
            .questions
            .as_ref()
            .and_then(|q| q.get(0))
            .and_then(|q| q.get("question").or_else(|| q.get("header")))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty());
        return Some(Reason {
            text: header.map_or_else(|| "Question for you".into(), str::to_owned),
            action: InboxAction::Answer,
            fault: false,
        });
    }
    if needs_you(session) {
        return Some(Reason {
            text: "Approval needed".into(),
            action: InboxAction::Open,
            fault: false,
        });
    }
    if let Some(quiet) = stalled_for {
        return Some(Reason {
            text: format!("Not responding · {}", quiet_for(quiet)),
            action: InboxAction::Open,
            fault: true,
        });
    }
    None
}

/// "2m", "1h 05m": how long something has been silent.
pub(crate) fn quiet_for(ms: i64) -> String {
    let minutes = (ms.max(0) / 60_000).max(1);
    if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 48 * 60 {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    } else {
        format!("{}d", minutes / (24 * 60))
    }
}

/// The collapsed line for a session's subagents: "4 subagents · 1 not responding",
/// "2 subagents · 1 working". `None` without any.
pub(crate) fn children_summary(
    total: usize,
    stalled: usize,
    failed: usize,
    working: usize,
) -> Option<String> {
    if total == 0 {
        return None;
    }
    let mut parts = vec![format!(
        "{total} {}",
        if total == 1 { "subagent" } else { "subagents" }
    )];
    if stalled > 0 {
        parts.push(format!("{stalled} not responding"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if stalled == 0 && failed == 0 && working > 0 {
        parts.push(format!("{working} working"));
    }
    Some(wks_native::format::meta(parts))
}

/// Tell apart rows whose titles repeat: each title shared by more than one
/// row gets its age ("3d"), or "#2", "#3" when ages also repeat. `None`
/// for a unique title.
pub(crate) fn disambiguate(rows: &[(String, Option<String>)]) -> Vec<Option<String>> {
    let mut out = vec![None; rows.len()];
    for (ix, (title, age)) in rows.iter().enumerate() {
        let same: Vec<usize> = (0..rows.len()).filter(|&j| rows[j].0 == *title).collect();
        if same.len() < 2 {
            continue;
        }
        let ages_unique = age.is_some() && same.iter().filter(|&&j| rows[j].1 == *age).count() == 1;
        out[ix] = if ages_unique {
            age.clone()
        } else {
            let rank = same.iter().position(|&j| j == ix).unwrap_or(0) + 1;
            Some(format!("#{rank}"))
        };
    }
    out
}

/// Case-insensitive substring ranges of `query` in `text`, for highlights.
pub(crate) fn match_ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let lower = text.to_lowercase();
    let needle = query.to_lowercase();
    // Lowercasing can change byte lengths; only highlight when it did not.
    if lower.len() != text.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&needle) {
        let start = from + at;
        out.push(start..start + needle.len());
        from = start + needle.len().max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // r2-shell
    #[test]
    fn reasons_name_the_request_and_offer_one_action() {
        let mut s = Session {
            approval: Some(serde_json::json!({
                "toolName": "Bash", "toolInput": {"command": "cargo test\n--more"}
            })),
            ..Default::default()
        };
        let r = reason(&s, None, None).unwrap();
        assert_eq!(r.text, "Run cargo test?");
        assert_eq!(r.action, InboxAction::Allow);
        s.approval = Some(serde_json::json!({"toolName": "ExitPlanMode"}));
        assert_eq!(reason(&s, None, None).unwrap().action, InboxAction::Open);
        s.approval = None;
        s.questions = Some(serde_json::json!([{"question": "Which theme?"}]));
        let r = reason(&s, None, None).unwrap();
        assert_eq!(
            (r.text.as_str(), r.action),
            ("Which theme?", InboxAction::Answer)
        );
        s.questions = None;
        s.state = "working".into();
        let r = reason(&s, Some(7 * 60_000), None).unwrap();
        assert_eq!(r.text, "Not responding · 7m");
        assert!(r.fault);
        // R4.1: a failure leads, named.
        let r = reason(&s, Some(7 * 60_000), Some("cargo test failed".into())).unwrap();
        assert_eq!((r.text.as_str(), r.fault), ("cargo test failed", true));
        assert_eq!(reason(&s, None, None), None);
    }

    #[test]
    fn subagents_fold_into_one_line() {
        assert_eq!(children_summary(0, 0, 0, 0), None);
        assert_eq!(
            children_summary(4, 1, 0, 2).as_deref(),
            Some("4 subagents · 1 not responding")
        );
        assert_eq!(
            children_summary(1, 0, 0, 1).as_deref(),
            Some("1 subagent · 1 working")
        );
    }

    #[test]
    fn repeated_titles_are_told_apart() {
        let rows = [
            ("Bus".to_owned(), Some("2d".to_owned())),
            ("Notes".to_owned(), Some("2d".to_owned())),
            ("Bus".to_owned(), Some("5d".to_owned())),
            ("Fly".to_owned(), Some("1h".to_owned())),
            ("Fly".to_owned(), Some("1h".to_owned())),
        ];
        assert_eq!(
            disambiguate(&rows),
            vec![
                Some("2d".into()),
                None,
                Some("5d".into()),
                Some("#1".into()),
                Some("#2".into())
            ]
        );
    }

    #[test]
    fn matches_are_found_case_insensitively() {
        assert_eq!(
            match_ranges("Session Review review", "review"),
            vec![8..14, 15..21]
        );
        assert!(match_ranges("anything", " ").is_empty());
    }

    #[test]
    fn trees_follow_their_most_urgent_member_and_waits_sort_oldest_first() {
        // 0 idle root with a child (1) that needs you; 2 working; 3 ended;
        // 4 needs you and has waited longer than 1.
        let tree = [(0, 0), (1, 1), (2, 0), (3, 0), (4, 0)];
        let buckets = [
            Bucket::Idle,
            Bucket::NeedsYou,
            Bucket::Working,
            Bucket::Ended,
            Bucket::NeedsYou,
        ];
        let since = [None, Some(500), None, None, Some(100)];
        let groups = group(&tree, |ix| buckets[ix], |ix| since[ix]);
        assert_eq!(
            groups,
            vec![
                (Bucket::NeedsYou, vec![(4, 0), (0, 0), (1, 1)]),
                (Bucket::Working, vec![(2, 0)]),
                (Bucket::Ended, vec![(3, 0)]),
            ]
        );
    }

    #[test]
    fn quiet_groups_put_the_most_recent_first() {
        let tree = [(0, 0), (1, 0), (2, 0)];
        let at = [Some(100), None, Some(300)];
        let groups = group(&tree, |_| Bucket::Idle, |ix| at[ix]);
        assert_eq!(groups[0].1, vec![(2, 0), (0, 0), (1, 0)]);
    }

    #[test]
    fn unknown_waits_go_after_known_ones_in_hub_order() {
        let tree = [(0, 0), (1, 0), (2, 0)];
        let groups = group(&tree, |_| Bucket::NeedsYou, |ix| (ix == 2).then_some(7));
        assert_eq!(groups[0].1, vec![(2, 0), (0, 0), (1, 0)]);
    }

    #[test]
    fn next_needing_walks_the_shown_order_and_wraps() {
        let order = [5, 3, 8, 1];
        assert_eq!(next_needing(&order, &[], Some(5)), None);
        assert_eq!(next_needing(&order, &[8, 1], None), Some(8));
        assert_eq!(next_needing(&order, &[8, 1], Some(8)), Some(1));
        assert_eq!(next_needing(&order, &[8, 1], Some(1)), Some(8));
        assert_eq!(next_needing(&order, &[3], Some(3)), Some(3));
        // A current session the list no longer shows starts from the top.
        assert_eq!(next_needing(&order, &[1], Some(42)), Some(1));
    }
}
