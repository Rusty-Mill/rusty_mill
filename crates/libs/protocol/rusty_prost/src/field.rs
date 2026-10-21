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

/// Merges one occurrence of a message-typed field into `value`. For a map entry's value this is
/// what `|v, f| ...` should do, so a value split across occurrences ends up as one message.
pub fn merge_message<M: Message>(value: &mut M, f: &Field<'_>) -> Result<()> {
    value.merge(f.message()?)
}

/// Reads one map entry and inserts it. The key is field 1 and the value field 2, and either may
/// be absent (then it is the default). `key` and `value` each merge **one occurrence** into the
/// entry's key or value, so a scalar's last occurrence wins and a message value that is split
/// across occurrences merges into one, as in the reference. A repeated *entry* with a key already
/// in the map replaces it: entries do not merge.
pub fn map_entry<K: Default + Ord, V: Default>(
    map: &mut BTreeMap<K, V>,
    f: &Field<'_>,
    key: impl Fn(&mut K, &Field<'_>) -> Result<()>,
    value: impl Fn(&mut V, &Field<'_>) -> Result<()>,
) -> Result<()> {
    let (mut k, mut v) = (K::default(), V::default());
    for e in f.message()? {
        let e = e?;
        match e.number {
            1 => key(&mut k, &e)?,
            2 => value(&mut v, &e)?,
            _ => {}
        }
    }
    map.insert(k, v);
    Ok(())
}
