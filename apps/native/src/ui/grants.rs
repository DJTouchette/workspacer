//! r5-fleet (R5.4): the "Always allow" grants a session was given, listed
//! in Session details and revocable where the app can revoke them.
//!
//! The hub has no call that lists or removes a session's permission rules
//! (`claude.approve {alwaysOption}` adds one; nothing takes one back). So
//! this window remembers each grant it sends (`Workspace::note_grant`, from
//! `command`) and lists those. Revoke does what the app controls:
//!
//! - "Allow all edits" (`setMode`) is the session's access mode, which
//!   `claude.setPermissionMode` sets back to Ask.
//! - A rule (`addRules`, `addDirectories`) cannot be removed through the
//!   hub yet: its Revoke is disabled and says so; the rule ends with the
//!   session.
use super::decision::grant_rule;
use super::primitives::{self, ButtonSize, ButtonVariant, Col};
use super::*;

/// One grant sent from this window.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Granted {
    pub session: String,
    /// The rule as people read it (`cargo test *`) and the provider's exact
    /// one (`Bash(cargo test:*)`).
    pub rule: String,
    pub exact: String,
    pub scope: String,
    /// The hub's kind: `addRules`, `setMode`, `addDirectories`.
    pub kind: String,
    pub at: i64,
}

impl Granted {
    /// Whether the app can take it back (`setMode`: the access mode).
    pub(super) fn revocable(&self) -> bool {
        self.kind == "setMode"
    }
}

/// Most grants remembered per window, oldest dropped first.
const MAX_GRANTS: usize = 64;

/// Why a rule's Revoke is disabled.
pub(super) const CANNOT_REVOKE: &str =
    "The hub cannot remove a rule yet. It lasts until the session ends.";

impl Workspace {
    /// Remember an "Always allow" as it is sent (called from `command`).
    pub(super) fn note_grant(&mut self, command: &Command) {
        let Command::Act {
            session,
            action: Action::AlwaysAllow { option, label, .. },
        } = command
        else {
            return;
        };
        let approval = self
            .view
            .sessions
            .iter()
            .find(|s| &s.id == session)
            .and_then(|s| s.approval.as_ref());
        let kind = approval
            .map(wks_native::session_controls::always_options)
            .unwrap_or_default()
            .into_iter()
            .find(|o| &o.id == option)
            .map(|o| o.kind)
            .unwrap_or_default();
        let command_text = approval
            .map(|a| super::composer_v2::approval_summary(a).1)
            .unwrap_or_default();
        let (rule, exact, scope) = grant_rule(label, &command_text);
        let grant = Granted {
            session: session.clone(),
            rule,
            exact,
            scope,
            kind,
            at: timing::now_ms(),
        };
        let grants = &mut self.fleet.grants;
        grants.retain(|g| !(g.session == grant.session && g.exact == grant.exact));
        grants.push(grant);
        if grants.len() > MAX_GRANTS {
            grants.remove(0);
        }
    }

    /// The session's grants, newest first.
    pub(super) fn session_grants(&self, session: &str) -> Vec<(usize, Granted)> {
        let mut grants: Vec<_> = self
            .fleet
            .grants
            .iter()
            .enumerate()
            .filter(|(_, g)| g.session == session)
            .map(|(ix, g)| (ix, g.clone()))
            .collect();
        grants.reverse();
        grants
    }

    /// Revoke grant `ix`: an access-mode grant goes back to Ask; a rule
    /// cannot be removed (its button is disabled).
    fn revoke_grant(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(grant) = self.fleet.grants.get(ix).cloned() else {
            return;
        };
        if !grant.revocable() {
            return;
        }
        let provider = self
            .view
            .sessions
            .iter()
            .find(|s| s.id == grant.session)
            .map(|s| s.provider_id().to_owned())
            .unwrap_or_else(|| "claude".into());
        let ask = wks_native::launch::Permission::Ask;
        let Ok(mode) = ask.wire(&provider) else {
            return;
        };
        self.command(
            Command::Act {
                session: grant.session.clone(),
                action: Action::SetPermission {
                    mode: mode.to_owned(),
                    label: ask.label().to_owned(),
                },
            },
            cx,
        );
        self.fleet.grants.remove(ix);
        cx.notify();
    }

    /// Session details' "Always allow" section (design v2): each grant's
    /// rule, scope and age, with Revoke in a fixed slot. `None` under v1 or
    /// when this window granted nothing for the session.
    pub(super) fn render_grants_card(&self, s: &Session, cx: &mut Context<Self>) -> Option<Div> {
        if !self.design().is_v2() {
            return None;
        }
        let grants = self.session_grants(&s.id);
        if grants.is_empty() {
            return None;
        }
        let p = self.palette();
        let t = p.tokens;
        let pg = t.pages();
        let slot = primitives::action_slots_width(1, &p);
        let cols = [
            ("Rule", Col::Flex),
            ("Lasts", Col::Fixed(pg.age)),
            ("Granted", Col::Fixed(pg.age)),
            ("", Col::Keep(slot)),
        ];
        let now = timing::now_ms();
        let live = self.view.connected && !s.stopped() && !self.view.busy;
        let rows = grants.into_iter().map(|(ix, grant)| {
            let revocable = grant.revocable() && live;
            let tip = if grant.revocable() {
                "Set this session's access back to Ask before edits".to_owned()
            } else {
                CANNOT_REVOKE.to_owned()
            };
            let revoke = primitives::slot_button(
                primitives::button(
                    ("grant-revoke", ix),
                    "Revoke",
                    None,
                    ButtonVariant::Destructive,
                    ButtonSize::Small,
                    revocable,
                    &p,
                ),
                &p,
            )
            .debug_selector(move || format!("grant-revoke-{ix}"))
            .tooltip(primitives::tooltip_view(tip, None, &p))
            .when(revocable, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| this.revoke_grant(ix, cx)))
            });
            primitives::page_table_row(&p)
                .debug_selector(move || format!("grant-{ix}"))
                .child(
                    primitives::page_cell(cols[0].1)
                        .gap(px(t.space.sm))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font())
                                .child(grant.rule.clone()),
                        )
                        .when(grant.exact != grant.rule, |d| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(t.text.meta))
                                    .text_color(rgb(p.roles.text_tertiary))
                                    .child(grant.exact.clone()),
                            )
                        }),
                )
                .child(
                    primitives::page_cell(cols[1].1)
                        .text_color(rgb(p.roles.text_secondary))
                        .child(grant.scope.clone()),
                )
                .child(
                    primitives::page_cell(cols[2].1).child(primitives::tabular(
                        div()
                            .text_color(rgb(p.roles.text_secondary))
                            .child(wks_native::format::age(grant.at, now)),
                    )),
                )
                .child(
                    primitives::page_cell(cols[3].1).child(primitives::action_slots(
                        vec![primitives::action_slot(
                            pg.action_slot,
                            Some(revoke.into_any_element()),
                        )],
                        &p,
                    )),
                )
        });
        Some(
            div()
                .debug_selector(|| "session-grants".into())
                .flex()
                .flex_col()
                .gap(px(t.space.md))
                .child(primitives::section_header("Always allow", None, &p))
                .child(
                    div()
                        .text_size(px(t.text.meta))
                        .text_color(rgb(p.roles.text_tertiary))
                        .child("Granted from this window. Each lasts until the session ends."),
                )
                .child(
                    primitives::page_table(&p)
                        .child(primitives::page_table_head(&cols, &p))
                        .children(rows),
                ),
        )
    }
}
