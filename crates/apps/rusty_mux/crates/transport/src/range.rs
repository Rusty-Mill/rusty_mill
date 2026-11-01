//! A set of disjoint, non-adjacent byte ranges `[start, end)`.

use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(crate) struct RangeSet {
    /// Start to end; no two touch.
    ranges: BTreeMap<u64, u64>,
    len: u64,
}

impl RangeSet {
    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub(crate) fn first(&self) -> Option<(u64, u64)> {
        self.ranges.first_key_value().map(|(&s, &e)| (s, e))
    }

    pub(crate) fn last(&self) -> Option<(u64, u64)> {
        self.ranges.last_key_value().map(|(&s, &e)| (s, e))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.ranges.iter().map(|(&s, &e)| (s, e))
    }

    /// Adds `[start, end)`, merging what it touches.
    pub(crate) fn insert(&mut self, mut start: u64, mut end: u64) {
        if start >= end {
            return;
        }
        let touching: Vec<(u64, u64)> = self
            .ranges
            .range(..=end)
            .rev()
            .take_while(|&(_, &e)| e >= start)
            .map(|(&s, &e)| (s, e))
            .collect();
        for (s, e) in touching {
            self.ranges.remove(&s);
            self.len -= e - s;
            start = start.min(s);
            end = end.max(e);
        }
        self.ranges.insert(start, end);
        self.len += end - start;
    }

    /// Takes `[start, end)` out, splitting what it cuts.
    pub(crate) fn remove(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let cut: Vec<(u64, u64)> = self
            .ranges
            .range(..end)
            .rev()
            .take_while(|&(_, &e)| e > start)
            .map(|(&s, &e)| (s, e))
            .collect();
        for (s, e) in cut {
            self.ranges.remove(&s);
            self.len -= e - s;
            if s < start {
                self.ranges.insert(s, start);
                self.len += start - s;
            }
            if e > end {
                self.ranges.insert(end, e);
                self.len += e - end;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(set: &RangeSet) -> Vec<(u64, u64)> {
        set.iter().collect()
    }

    #[test]
    fn inserts_merge_what_they_touch() {
        let mut set = RangeSet::default();
        set.insert(10, 20);
        set.insert(30, 40);
        set.insert(20, 25);
        assert_eq!(all(&set), vec![(10, 25), (30, 40)]);
        set.insert(5, 35);
        assert_eq!(all(&set), vec![(5, 40)]);
        set.insert(7, 7);
        assert_eq!(set.len(), 35);
    }

    #[test]
    fn removes_split_what_they_cut() {
        let mut set = RangeSet::default();
        set.insert(0, 10);
        set.insert(20, 30);
        set.remove(5, 25);
        assert_eq!(all(&set), vec![(0, 5), (25, 30)]);
        set.remove(0, 100);
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }
}
