//! R5.2 Button rules that depend on state, in one place.
use super::super::primitives::ButtonVariant;

/// Send (and Queue) is the Primary only when it can send; when it cannot it
/// is the *disabled* Primary (a grey fill), never an outlined Secondary.
/// Returns the variant and whether the button is enabled.
pub(crate) fn send_state(can_send: bool) -> (ButtonVariant, bool) {
    (ButtonVariant::Primary, can_send)
}

/// Stop, Deny and Discard interrupt or remove: always Destructive, never
/// the brand-blue Primary.
pub(crate) fn stop_variant() -> ButtonVariant {
    ButtonVariant::Destructive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_fills_only_when_it_can_send_and_stop_is_never_primary() {
        assert_eq!(send_state(true), (ButtonVariant::Primary, true));
        assert_eq!(send_state(false), (ButtonVariant::Primary, false));
        assert_ne!(stop_variant(), ButtonVariant::Primary);
    }
}
