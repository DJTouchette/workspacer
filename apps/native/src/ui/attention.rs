//! Design v2's attention-first session list: which group a session belongs
//! to (Needs you, Working, Idle, Ended) and the order the sidebar, `g n` and
//! j/k walk them in. Pure: the sidebar draws what this returns.
use wks_native::model::Session;

/// The sidebar's groups, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Bucket {
    /// An approval or a question is waiting on the user.
    NeedsYou,
    /// Starting or mid-turn.
    Working,
    /// Ready for the next message, or paused (resumable).
    Idle,
    /// Ended on purpose; folded away by default.
    Ended,
}

impl Bucket {
    pub(crate) const ALL: [Self; 4] = [Self::NeedsYou, Self::Working, Self::Idle, Self::Ended];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
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

/// One session's own group. `paused` is a stopped session the hub keeps
/// open (resumable from its composer): idle, not ended.
pub(crate) fn bucket(session: &Session, paused: bool) -> Bucket {
    if session.stopped() {
        return if paused { Bucket::Idle } else { Bucket::Ended };
    }
    if needs_you(session) {
        Bucket::NeedsYou
    } else if session.working() || session.starting() {
        Bucket::Working
    } else {
        Bucket::Idle
    }
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
    type Waiting = (Option<i64>, Vec<(usize, usize)>);
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
        groups[bucket.index()].1.push((since, members));
    }
    // Oldest wait first; a tree with no known wait keeps its place after them.
    groups[Bucket::NeedsYou.index()]
        .1
        .sort_by_key(|(since, _)| since.unwrap_or(i64::MAX));
    groups
        .into_iter()
        .filter(|(_, trees)| !trees.is_empty())
        .map(|(bucket, trees)| {
            (
                bucket,
                trees.into_iter().flat_map(|(_, members)| members).collect(),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn sessions_land_in_the_vocabulary_groups() {
        let mut s = Session {
            state: "idle".into(),
            ..Default::default()
        };
        assert_eq!(bucket(&s, false), Bucket::Idle);
        s.approval = Some(serde_json::json!({}));
        assert_eq!(bucket(&s, false), Bucket::NeedsYou);
        s.approval = None;
        s.state = "working".into();
        assert_eq!(bucket(&s, false), Bucket::Working);
        s.state = "ended".into();
        assert_eq!(bucket(&s, false), Bucket::Ended);
        assert_eq!(bucket(&s, true), Bucket::Idle);
    }
}
