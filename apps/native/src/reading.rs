//! Reading anchors survive list virtualization, snapshot replacement and front eviction.
use crate::model::Row;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Anchor {
    pub role: String,
    pub timestamp: Option<String>,
    pub tool_id: String,
    pub prefix: String,
    pub occurrence: usize,
}
impl Anchor {
    pub fn at(rows: &VecDeque<Arc<Row>>, ix: usize) -> Option<Self> {
        let row = rows.get(ix)?;
        let mut anchor = Self {
            role: row.role.clone(),
            timestamp: row.timestamp.clone(),
            tool_id: row.tool.as_ref().map(|t| t.id.clone()).unwrap_or_default(),
            prefix: crate::transcript::head(&row.text, 64),
            occurrence: 0,
        };
        anchor.occurrence = rows.iter().take(ix).filter(|r| anchor.matches(r)).count();
        Some(anchor)
    }
    fn matches(&self, row: &Row) -> bool {
        self.role == row.role
            && if !self.tool_id.is_empty() {
                row.tool.as_ref().is_some_and(|t| t.id == self.tool_id)
            } else if self.timestamp.is_some() {
                self.timestamp == row.timestamp
            } else {
                row.text.starts_with(&self.prefix)
            }
    }
    pub fn locate(&self, rows: &VecDeque<Arc<Row>>) -> Option<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| self.matches(r))
            .nth(self.occurrence)
            .map(|(i, _)| i)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn row(s: &str) -> Arc<Row> {
        Arc::new(Row {
            role: "Assistant".into(),
            text: s.into(),
            ..Default::default()
        })
    }
    #[test]
    fn anchors_restore_after_front_eviction() {
        let rows = VecDeque::from([row("old"), row("current")]);
        let anchor = Anchor::at(&rows, 1).unwrap();
        let newer = VecDeque::from([row("current grows"), row("next")]);
        assert_eq!(anchor.locate(&newer), Some(0));
    }
    #[test]
    fn repeated_messages_have_distinct_anchors() {
        let rows = VecDeque::from([row("yes"), row("yes")]);
        assert_eq!(Anchor::at(&rows, 1).unwrap().locate(&rows), Some(1));
    }
}
