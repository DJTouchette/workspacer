//! Values derived from transcript rows, kept until those rows change. Rows
//! are `Arc`s the published view shares, so an edited row is a new `Arc`
//! (`Arc::make_mut` copies a shared one); comparing pointers is then enough.
//! An entry holds its rows, so a freed row's address can never be reused by
//! a different row and pass for it.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wks_native::model::Row;

/// Entries kept before the memo starts over (long sessions, many cards).
const MAX_ENTRIES: usize = 2048;

pub(super) struct RowMemo<T> {
    entries: RefCell<HashMap<String, (Vec<Arc<Row>>, Rc<T>)>>,
}

impl<T> Default for RowMemo<T> {
    fn default() -> Self {
        Self {
            entries: RefCell::default(),
        }
    }
}

impl<T> RowMemo<T> {
    /// The value for `key` derived from `rows`, computed by `derive` only
    /// when those rows are not the ones it was last derived from.
    pub(super) fn get<'a>(
        &self,
        key: &str,
        rows: impl IntoIterator<Item = &'a Arc<Row>>,
        derive: impl FnOnce() -> T,
    ) -> Rc<T> {
        let rows: Vec<&Arc<Row>> = rows.into_iter().collect();
        let mut entries = self.entries.borrow_mut();
        if let Some((held, value)) = entries.get(key)
            && held.len() == rows.len()
            && held.iter().zip(&rows).all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return value.clone();
        }
        if entries.len() >= MAX_ENTRIES {
            entries.clear();
        }
        let value = Rc::new(derive());
        entries.insert(
            key.to_owned(),
            (rows.into_iter().cloned().collect(), value.clone()),
        );
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_derived_again_only_when_its_rows_change() {
        let memo = RowMemo::default();
        let rows = vec![Arc::new(Row::default()), Arc::new(Row::default())];
        let mut runs = 0;
        let mut get = |rows: &[Arc<Row>]| {
            *memo.get("card", rows, || {
                runs += 1;
                runs
            })
        };
        assert_eq!(get(&rows), 1);
        assert_eq!(get(&rows), 1, "same rows: kept");
        let mut edited = rows.clone();
        Arc::make_mut(&mut edited[1]).text = "changed".into();
        assert_eq!(get(&edited), 2, "an edited row is a new Arc");
        assert_eq!(get(&edited[..1]), 3, "fewer rows");
    }
}
