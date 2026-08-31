//! Append-only history container with cheap clones and incremental checksum.
//!
//! Purpose: give the audit log, chronicle, and outbox a shared immutable bulk
//! so every transactional clone stays cheap even as campaigns age. Mutation is
//! append-only; clones share the bulk through an `Arc` and keep a small
//! exclusive tail. Iteration order and serialized shape match a plain `Vec`.
//! Owns: `HistoryLog<T>` copy-on-write + incremental structural checksum,
//! `HistoryLogIter`/`HistoryLogIterMut`, and the atomic memo.
//! Reads: serialized entries for the running checksum only.
//! Mutates: its own bulk/tail/memo through `push`/`retain`/`clear`.
//! Does not own: authoritative campaign state — `state.rs` owns `AppState`.
//! Canonical operations: `push`, `iter`, `partition_point`, `retain`,
//! `structural_checksum` with incremental fold.
//! Relevant invariants: append-only text is immutable after construction;
//! checksum memo is pure derivation excluded from equality/serialization.
//! Focused tests: `src/core/state_tests.rs` clone-cheapness and checksum.

use crate::core::checksum::ChecksumFolder;
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Memo sentinel meaning the running checksum does not match the log and must be rebuilt.
pub(crate) const HISTORY_CHECKSUM_UNSYNCED: u64 = u64::MAX;

/// Entries appended since the last fold; past this many, an exclusively
/// owned log folds them into the shared bulk so the tail stays a short copy.
pub(crate) const HISTORY_TAIL_FOLD_THRESHOLD: usize = 1024;

/// Append-only history container whose clones stay cheap as campaigns age.
///
/// The audit log, chronicle, and outbox grow without bound, and every
/// transactional commit clones the whole working state and then drops the
/// replaced original. A plain `Vec` makes both sides of that idiom
/// proportional to total campaign history even though histories only ever
/// gain entries. [`HistoryLog`] appends into a small exclusive tail while the
/// immutable bulk is shared through an arc, so a clone is one refcount plus a
/// short copy and dropping a superseded working copy releases its share of
/// the bulk untouched. Iteration order, serialized shape, and observable
/// values are identical to the plain vector it replaces.
///
/// The log also maintains an incremental structural checksum over its entry
/// stream (see [`HistoryLog::structural_checksum`]). Appends extend the fold
/// in constant time, so observation paths that re-read the checksum after
/// every simulated day stay flat-cost across campaign length instead of
/// reserializing the whole history. The memo never affects stored content,
/// equality (which compares element-wise), or serialization.
#[derive(Debug)]
pub struct HistoryLog<T> {
    pub(crate) base: Arc<Vec<T>>,
    pub(crate) tail: Vec<T>,
    /// Number of entries folded into `checksum_state`, or
    /// [`HISTORY_CHECKSUM_UNSYNCED`] when
    /// non-append mutations made the memo stale and the next read must
    /// rebuild it.
    pub(crate) checksum_len: AtomicU64,
    /// Running FNV-1a mid-state covering entries `0..checksum_len`.
    pub(crate) checksum_state: AtomicU64,
}

impl<T: Clone> Clone for HistoryLog<T> {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            tail: self.tail.clone(),
            checksum_len: AtomicU64::new(self.checksum_len.load(Ordering::Relaxed)),
            checksum_state: AtomicU64::new(self.checksum_state.load(Ordering::Relaxed)),
        }
    }
}

impl<T> Default for HistoryLog<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Double-ended, exact-size iterator over a [`HistoryLog`]'s entries in
/// insertion order.
#[derive(Clone, Debug)]
pub struct HistoryLogIter<'a, T> {
    base: std::slice::Iter<'a, T>,
    tail: std::slice::Iter<'a, T>,
}

impl<'a, T> Iterator for HistoryLogIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.base.next().or_else(|| self.tail.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.base.len() + self.tail.len();
        (remaining, Some(remaining))
    }

    fn count(self) -> usize {
        self.base.count() + self.tail.count()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let base_len = self.base.len();
        if n < base_len {
            return self.base.nth(n);
        }
        let _ = self.base.nth(base_len);
        self.tail.nth(n - base_len)
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl<T> DoubleEndedIterator for HistoryLogIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.tail.next_back().or_else(|| self.base.next_back())
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        let tail_len = self.tail.len();
        if n < tail_len {
            return self.tail.nth_back(n);
        }
        let _ = self.tail.nth_back(tail_len);
        self.base.nth_back(n - tail_len)
    }
}

impl<T> ExactSizeIterator for HistoryLogIter<'_, T> {}

impl<T> HistoryLog<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Arc::new(Vec::new()),
            tail: Vec::new(),
            checksum_len: AtomicU64::new(0),
            checksum_state: AtomicU64::new(ChecksumFolder::new().raw()),
        }
    }

    pub fn push(&mut self, entry: T)
    where
        T: Serialize,
    {
        if self.tail.len() >= HISTORY_TAIL_FOLD_THRESHOLD
            && let Some(base) = Arc::get_mut(&mut self.base)
        {
            base.append(&mut self.tail);
        }
        let total_before = self.len();
        self.tail.push(entry);
        if self.checksum_len.load(Ordering::Relaxed) == total_before as u64
            && let Some(entry) = self.tail.last()
        {
            let mut folder = ChecksumFolder::from_raw(self.checksum_state.load(Ordering::Relaxed));
            let _ = entry.serialize(&mut folder);
            self.checksum_state.store(folder.raw(), Ordering::Relaxed);
            self.checksum_len
                .store(total_before as u64 + 1, Ordering::Relaxed);
        }
    }

    fn invalidate_checksum(&self) {
        self.checksum_len
            .store(HISTORY_CHECKSUM_UNSYNCED, Ordering::Relaxed);
    }

    #[must_use]
    pub fn structural_checksum(&self) -> u64
    where
        T: Serialize,
    {
        let total = self.len();
        let len = total as u64;
        let hashed_len = self.checksum_len.load(Ordering::Relaxed);
        if hashed_len != len {
            let mut folder = ChecksumFolder::new();
            for entry in self {
                let _ = entry.serialize(&mut folder);
            }
            self.checksum_state.store(folder.raw(), Ordering::Relaxed);
            let _ = self.checksum_len.compare_exchange(
                hashed_len,
                len,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        let finisher = ChecksumFolder::from_raw(self.checksum_state.load(Ordering::Relaxed));
        finisher.finish_with_entry_count(total)
    }

    fn fold_tail(&mut self)
    where
        T: Clone,
    {
        if !self.tail.is_empty() {
            Arc::make_mut(&mut self.base).append(&mut self.tail);
        }
    }

    /// Returns the total number of entries in this history log.
    ///
    /// # Panics
    ///
    /// Panics if the combined length of the shared bulk and exclusive tail
    /// exceeds `usize::MAX`. This is unreachable in practice: it would
    /// require more entries than addressable memory can hold.
    #[must_use]
    pub fn len(&self) -> usize {
        self.base
            .len()
            .checked_add(self.tail.len())
            .expect("history length must fit usize")
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.tail.is_empty()
    }

    #[must_use]
    pub fn last(&self) -> Option<&T> {
        self.tail.last().or_else(|| self.base.last())
    }

    #[must_use]
    pub fn iter(&self) -> HistoryLogIter<'_, T> {
        HistoryLogIter {
            base: self.base.iter(),
            tail: self.tail.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> HistoryLogIterMut<'_, T>
    where
        T: Clone,
    {
        self.fold_tail();
        self.invalidate_checksum();
        HistoryLogIterMut {
            entries: Arc::make_mut(&mut self.base).iter_mut(),
        }
    }

    #[must_use]
    pub fn partition_point<F>(&self, mut predicate: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let base_position = self.base.partition_point(|entry| predicate(entry));
        if base_position < self.base.len() {
            base_position
        } else {
            base_position + self.tail.partition_point(predicate)
        }
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
        T: Clone,
    {
        self.fold_tail();
        self.invalidate_checksum();
        Arc::make_mut(&mut self.base).retain(|entry| keep(entry));
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    #[cfg(test)]
    pub fn sort_by_key<K, F>(&mut self, mut compare: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
        T: Clone,
    {
        self.fold_tail();
        self.invalidate_checksum();
        Arc::make_mut(&mut self.base).sort_by_key(|entry| compare(entry));
    }
}

/// Mutable counterpart to [`HistoryLogIter`]; iteration always operates on
/// one folded buffer.
#[derive(Debug)]
pub struct HistoryLogIterMut<'a, T> {
    entries: std::slice::IterMut<'a, T>,
}

impl<'a, T> Iterator for HistoryLogIterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl<T> DoubleEndedIterator for HistoryLogIterMut<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back()
    }
}

impl<T> ExactSizeIterator for HistoryLogIterMut<'_, T> {}

impl<T> PartialEq for HistoryLog<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T> Eq for HistoryLog<T> where T: Eq {}

impl<'a, T> IntoIterator for &'a HistoryLog<T> {
    type Item = &'a T;
    type IntoIter = HistoryLogIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut HistoryLog<T>
where
    T: Clone,
{
    type Item = &'a mut T;
    type IntoIter = HistoryLogIterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> Serialize for HistoryLog<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.len()))?;
        for entry in self {
            sequence.serialize_element(entry)?;
        }
        sequence.end()
    }
}

impl<'de, T> Deserialize<'de> for HistoryLog<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            base: Arc::new(Vec::<T>::deserialize(deserializer)?),
            tail: Vec::new(),
            checksum_len: AtomicU64::new(HISTORY_CHECKSUM_UNSYNCED),
            checksum_state: AtomicU64::new(ChecksumFolder::new().raw()),
        })
    }
}
