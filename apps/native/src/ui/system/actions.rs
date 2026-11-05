//! R5.2 Button rules that depend on state, in one place.
use super::super::primitives::ButtonVariant;

/// Send (and Queue) is the Primary only when it can send; when it cannot it
/// is the *disabled* Primary (a grey fill), never an outlined Secondary.
/// Returns the variant and whether the button is enabled.
pub(crate) fn send_state(can_send: bool) -> (ButtonVariant, bool) {
    (ButtonVariant::Primary, can_send)
}

/// The composer's Send (R5.4): while a decision waits, the decision card
/// owns the surface's one Primary, so Send is Secondary (enabled when it
/// can send); otherwise [`send_state`]. Returns the variant and whether the
/// button is enabled.
pub(crate) fn composer_send_state(can_send: bool, deciding: bool) -> (ButtonVariant, bool) {
    if deciding {
        (ButtonVariant::Secondary, can_send)
    } else {
        send_state(can_send)
    }
}

/// Stop, Deny and Discard interrupt or remove: always Destructive, never
/// the brand-blue Primary.
pub(crate) fn stop_variant() -> ButtonVariant {
    ButtonVariant::Destructive
}

/// R7.2 The three button tiers. Every verb has one ([`tier`]); a surface
/// never picks a tier by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// The one action a focused region exists for (`ButtonVariant::Primary`).
    Filled,
    /// Peers and anything that stops or removes (`Secondary`, or
    /// `Destructive`: the same shape with a red label).
    Bordered,
    /// Quiet actions (`Tertiary`).
    Text,
}

/// R7.2: every verb a decision, the composer or a page header offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verb {
    /// Send (and Queue) a message.
    Send,
    /// The positive answer of a decision: Allow, Approve plan, Nudge on a
    /// stalled subagent, Send answer.
    Allow,
    /// Allow all, Approve · run with Codex: a variant of the positive answer.
    AllowVariant,
    /// Deny (a request, a job proposal).
    Deny,
    /// A "no" that removes nothing: Keep planning.
    KeepPlanning,
    /// Stop a turn or a subagent.
    Stop,
    /// Always allow (a grant).
    AlwaysAllow,
    /// Deny with note.
    DenyWithNote,
    /// Details, View output, Open, Cancel.
    Details,
    /// Nudge when it is not the decision's answer (a toolbar).
    Nudge,
    /// A page header's main action (New job, Add…).
    PageAction,
    /// Discard, Remove, End: removes something.
    Remove,
}

impl Verb {
    /// Every verb, for the table in `design-system.md` and its test.
    pub(crate) const ALL: [Verb; 12] = [
        Self::Send,
        Self::Allow,
        Self::AllowVariant,
        Self::Deny,
        Self::KeepPlanning,
        Self::Stop,
        Self::AlwaysAllow,
        Self::DenyWithNote,
        Self::Details,
        Self::Nudge,
        Self::PageAction,
        Self::Remove,
    ];

    /// The verb's tier.
    pub(crate) fn tier(self) -> Tier {
        match self {
            Self::Send | Self::Allow => Tier::Filled,
            Self::AllowVariant
            | Self::Deny
            | Self::KeepPlanning
            | Self::Stop
            | Self::PageAction
            | Self::Remove => Tier::Bordered,
            Self::AlwaysAllow | Self::DenyWithNote | Self::Details | Self::Nudge => Tier::Text,
        }
    }
}

/// R7.2: the button variant a verb draws in. A Bordered verb that stops or
/// removes takes the red label (`Destructive`).
pub(crate) fn tier(verb: Verb) -> ButtonVariant {
    match (verb.tier(), verb) {
        (Tier::Filled, _) => ButtonVariant::Primary,
        (Tier::Bordered, Verb::Deny | Verb::Stop | Verb::Remove) => ButtonVariant::Destructive,
        (Tier::Bordered, _) => ButtonVariant::Secondary,
        (Tier::Text, _) => ButtonVariant::Tertiary,
    }
}

/// R7.2 One Filled per focused region: a Filled verb outside the focused
/// region (an unfocused Fleet card or row, a side panel while a decision
/// waits) draws Bordered.
pub(crate) fn region_variant(verb: Verb, focused: bool) -> ButtonVariant {
    match tier(verb) {
        ButtonVariant::Primary if !focused => ButtonVariant::Secondary,
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verb_has_one_of_three_tiers() {
        for verb in Verb::ALL {
            let v = tier(verb);
            match verb.tier() {
                Tier::Filled => assert_eq!(v, ButtonVariant::Primary, "{verb:?}"),
                Tier::Bordered => assert!(
                    matches!(v, ButtonVariant::Secondary | ButtonVariant::Destructive),
                    "{verb:?}"
                ),
                Tier::Text => assert_eq!(v, ButtonVariant::Tertiary, "{verb:?}"),
            }
        }
        // The examples R7.2 names.
        assert_eq!(Verb::Send.tier(), Tier::Filled);
        assert_eq!(Verb::Allow.tier(), Tier::Filled);
        assert_eq!(Verb::Stop.tier(), Tier::Bordered);
        assert_eq!(Verb::Nudge.tier(), Tier::Text);
        assert_eq!(tier(Verb::Stop), stop_variant());
        // Only the focused region fills.
        assert_eq!(region_variant(Verb::Allow, true), ButtonVariant::Primary);
        assert_eq!(region_variant(Verb::Allow, false), ButtonVariant::Secondary);
        assert_eq!(
            region_variant(Verb::Deny, false),
            ButtonVariant::Destructive
        );
    }

    #[test]
    fn send_fills_only_when_it_can_send_and_stop_is_never_primary() {
        assert_eq!(send_state(true), (ButtonVariant::Primary, true));
        assert_eq!(send_state(false), (ButtonVariant::Primary, false));
        assert_ne!(stop_variant(), ButtonVariant::Primary);
    }

    #[test]
    fn a_pending_decision_owns_the_primary_so_send_is_secondary() {
        assert_eq!(
            composer_send_state(true, true),
            (ButtonVariant::Secondary, true)
        );
        assert_eq!(
            composer_send_state(false, true),
            (ButtonVariant::Secondary, false)
        );
        assert_eq!(composer_send_state(true, false), send_state(true));
        assert_eq!(composer_send_state(false, false), send_state(false));
    }
}
