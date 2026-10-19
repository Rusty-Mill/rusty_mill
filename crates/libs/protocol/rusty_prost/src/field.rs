//! Helpers for the field shapes a `merge_field` impl meets most: sub-messages and maps.
//! Scalars, strings and packed runs are read straight off the [`Field`].

use std::collections::BTreeMap;

use crate::{Field, Message, Result};

/// Merges a sub-message occurrence into an optional field, creating it if absent.
pub fn message<M: Message>(slot: &mut Option<M>, f: &Field<'_>) -> Result<()> {
    let reader = f.message()?;
    slot.get_or_insert_with(M::default).merge(reader)
}

/// Appends a sub-message occurrence to a repeated field.
pub fn push_message<M: Message>(items: &mut Vec<M>, f: &Field<'_>) -> Result<()> {
    let mut m = M::default();
    m.merge(f.message()?)?;
    items.push(m);
    Ok(())
}

/// Reads one map entry (key is field 1, value field 2, either may be absent and then takes its
/// default) and inserts it; a repeated key replaces the earlier one.
pub fn map_entry<K: Default + Ord, V: Default>(
    map: &mut BTreeMap<K, V>,
    f: &Field<'_>,
    key: impl Fn(&Field<'_>) -> Result<K>,
    value: impl Fn(&Field<'_>) -> Result<V>,
) -> Result<()> {
    let (mut k, mut v) = (K::default(), V::default());
    for e in f.message()? {
        let e = e?;
        match e.number {
            1 => k = key(&e)?,
            2 => v = value(&e)?,
            _ => {}
        }
    }
    map.insert(k, v);
    Ok(())
}
