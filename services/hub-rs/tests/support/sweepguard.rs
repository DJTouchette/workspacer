//! Executed-case accounting for fixture sweeps. Count only after setup/skip gates.
#![allow(dead_code)] // Different test binaries exercise different floor shapes.
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct Tally {
    pub allow: usize,
    pub deny: usize,
    pub other: usize,
    pub skipped: usize,
    reasons: BTreeMap<String, usize>,
}
impl Tally {
    pub fn ran(&mut self, verdict: &str) {
        match verdict.trim().to_lowercase().as_str() {
            "allow" | "accept" | "ok" | "pass" => self.allow += 1,
            "deny" | "refuse" | "reject" | "fail" => self.deny += 1,
            _ => self.other += 1,
        }
    }
    pub fn skip(&mut self, reason: &str) {
        self.skipped += 1;
        let reason = reason.trim();
        *self
            .reasons
            .entry(
                if reason.is_empty() {
                    "unspecified"
                } else {
                    reason
                }
                .into(),
            )
            .or_default() += 1;
    }
    pub fn executed(&self) -> usize {
        self.allow + self.deny + self.other
    }
    pub fn enumerated(&self) -> usize {
        self.executed() + self.skipped
    }
    fn skip_suffix(&self) -> String {
        if self.skipped == 0 {
            return String::new();
        }
        let reasons = self
            .reasons
            .iter()
            .map(|(reason, count)| format!("{reason}×{count}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("; {} case(s) skipped: {reasons}", self.skipped)
    }
    pub fn require(&self, what: &str, min_allow: usize, min_deny: usize) -> Result<(), String> {
        let mut missing = vec![];
        if self.allow < min_allow {
            missing.push(format!("{} allow cases (want >= {min_allow})", self.allow));
        }
        if self.deny < min_deny {
            missing.push(format!("{} deny cases (want >= {min_deny})", self.deny));
        }
        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "{what} executed {} — a sweep that ran none of a verdict class asserted nothing about it and is a PASS that guards nothing{}",
            missing.join(" and "),
            self.skip_suffix()
        ))
    }
    pub fn require_both(&self, what: &str) -> Result<(), String> {
        self.require(what, 1, 1)
    }
    pub fn require_deny(&self, what: &str) -> Result<(), String> {
        self.require(what, 0, 1)
    }
    pub fn require_corpus(
        &self,
        what: &str,
        min_enumerated: usize,
        min_allow: usize,
        min_deny: usize,
    ) -> Result<(), String> {
        if self.enumerated() < min_enumerated {
            return Err(format!(
                "{what} reached {} cases but the floor is {min_enumerated} — the corpus SHRANK; this count is host-independent{}",
                self.enumerated(),
                self.skip_suffix()
            ));
        }
        self.require(what, min_allow, min_deny)
    }
    pub fn require_every(&self, what: &str, minimum: usize) -> Result<(), String> {
        if self.executed() >= minimum {
            return Ok(());
        }
        Err(format!(
            "{what} executed {} of a floor of {minimum} cases — nothing in this block is host-gated{}",
            self.executed(),
            self.skip_suffix()
        ))
    }
}
impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "executed {} cases ({} allow, {} deny, {} other){}",
            self.executed(),
            self.allow,
            self.deny,
            self.other,
            self.skip_suffix()
        )
    }
}
