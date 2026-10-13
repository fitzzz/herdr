//! Bounded bookkeeping for grids kept on the device, independent of CUDA so
//! the bounds are tested without a GPU.

/// Identity of a resident grid. The shape is part of the key: a generation
/// reused with another width or cell count is never read as the old grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridKey {
    pub(crate) generation: u64,
    pub(crate) width: usize,
    pub(crate) cells: usize,
}

/// A grid could not fit within the residency bounds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OverBudget;

struct Entry<H> {
    key: GridKey,
    handle: H,
    bytes: usize,
    last_used: u64,
}

pub(crate) struct Residency<H> {
    entries: Vec<Entry<H>>,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl<H: Copy> Residency<H> {
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: Vec::new(),
            clock: 0,
            max_entries,
            max_bytes,
        }
    }

    /// Starts one comparison; grids used by it count as most recent.
    pub(crate) fn tick(&mut self) {
        self.clock += 1;
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn used_bytes(&self) -> usize {
        self.entries.iter().map(|entry| entry.bytes).sum()
    }

    /// The handle holding exactly `key`. An entry with the same generation and
    /// another shape is removed and its handle pushed to `evicted`.
    pub(crate) fn get(&mut self, key: GridKey, evicted: &mut Vec<H>) -> Option<H> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.key.generation == key.generation)?;
        if self.entries[index].key != key {
            evicted.push(self.entries.swap_remove(index).handle);
            return None;
        }
        let entry = &mut self.entries[index];
        entry.last_used = self.clock;
        Some(entry.handle)
    }

    /// Evicts least recently used grids other than `keep` until a grid of
    /// `bytes` fits. Fails without evicting anything when it cannot fit.
    pub(crate) fn reserve(
        &mut self,
        bytes: usize,
        keep: Option<GridKey>,
        evicted: &mut Vec<H>,
    ) -> Result<(), OverBudget> {
        let (kept_entries, kept_bytes) = self
            .entries
            .iter()
            .filter(|entry| Some(entry.key) == keep)
            .fold((0, 0usize), |(count, total), entry| {
                (count + 1, total + entry.bytes)
            });
        if kept_entries + 1 > self.max_entries || kept_bytes.saturating_add(bytes) > self.max_bytes
        {
            return Err(OverBudget);
        }
        while self.entries.len() + 1 > self.max_entries
            || self.used_bytes().saturating_add(bytes) > self.max_bytes
        {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| Some(entry.key) != keep)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
                .ok_or(OverBudget)?;
            evicted.push(self.entries.swap_remove(oldest).handle);
        }
        Ok(())
    }

    /// Records a grid after a successful [`Self::reserve`].
    pub(crate) fn insert(&mut self, key: GridKey, handle: H, bytes: usize) {
        self.entries.push(Entry {
            key,
            handle,
            bytes,
            last_used: self.clock,
        });
    }

    /// Removes and returns every handle.
    pub(crate) fn drain(&mut self) -> Vec<H> {
        self.entries.drain(..).map(|entry| entry.handle).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(generation: u64) -> GridKey {
        GridKey {
            generation,
            width: 10,
            cells: 100,
        }
    }

    fn add(
        residency: &mut Residency<u64>,
        key: GridKey,
        bytes: usize,
        keep: Option<GridKey>,
    ) -> Vec<u64> {
        let mut evicted = Vec::new();
        residency.tick();
        residency.reserve(bytes, keep, &mut evicted).unwrap();
        residency.insert(key, key.generation, bytes);
        evicted
    }

    #[test]
    fn a_grid_larger_than_the_byte_bound_is_refused_without_evicting() {
        let mut residency = Residency::new(4, 100);
        add(&mut residency, key(1), 60, None);
        let mut evicted = Vec::new();
        assert_eq!(residency.reserve(101, None, &mut evicted), Err(OverBudget));
        assert!(evicted.is_empty());
        assert_eq!(residency.len(), 1);
    }

    #[test]
    fn two_grids_that_cannot_both_fit_are_refused_while_the_baseline_is_kept() {
        let mut residency = Residency::new(4, 100);
        add(&mut residency, key(1), 60, None);
        let mut evicted = Vec::new();
        assert_eq!(
            residency.reserve(60, Some(key(1)), &mut evicted),
            Err(OverBudget)
        );
        assert!(evicted.is_empty());
        assert_eq!(residency.len(), 1);
    }

    #[test]
    fn least_recently_used_grids_are_evicted_and_bounds_hold() {
        let mut residency = Residency::new(2, 1000);
        add(&mut residency, key(1), 10, None);
        add(&mut residency, key(2), 10, None);
        residency.tick();
        assert_eq!(residency.get(key(1), &mut Vec::new()), Some(1));
        assert_eq!(add(&mut residency, key(3), 10, Some(key(1))), vec![2]);
        assert_eq!(residency.len(), 2);
        assert_eq!(add(&mut residency, key(4), 10, Some(key(3))), vec![1]);
    }

    #[test]
    fn a_reused_generation_with_another_shape_is_not_resident() {
        let mut residency = Residency::new(4, 1000);
        add(&mut residency, key(7), 10, None);
        let mut evicted = Vec::new();
        let wider = GridKey {
            width: 20,
            cells: 100,
            ..key(7)
        };
        assert_eq!(residency.get(wider, &mut evicted), None);
        assert_eq!(evicted, vec![7]);
        assert_eq!(residency.len(), 0);
    }
}
