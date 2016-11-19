//! A collection of fundamental and/or simple types used by other modules

use std::cmp::Ordering;

/// Trait used to influence how SkipMap determines the order of elements. Use StandardComparator
/// for the normal implementation using numerical comparison.
pub trait Comparator: Copy {
    fn cmp(&self, &[u8], &[u8]) -> Ordering;
}

#[derive(Clone, Copy, Default)]
pub struct StandardComparator;

impl Comparator for StandardComparator {
    fn cmp(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
    }
}

/// An extension of the standard `Iterator` trait that supports some methods necessary for LevelDB.
/// This works because the iterators used are stateful and keep the last returned element.
///
/// Note: Implementing types are expected to hold `!valid()` before the first call to `next()`.
pub trait SSIterator: Iterator {
    // We're emulating LevelDB's Slice type here using actual slices with the lifetime of the
    // iterator. The lifetime of the iterator is usually the one of the backing storage (Block,
    // MemTable, SkipMap...)
    // type Item = (&'a [u8], &'a [u8]);

    /// Seek the iterator to `key` or the next bigger key. If the seek is invalid (past last
    /// element), the iterator is reset() and not valid.
    fn seek(&mut self, key: &[u8]);
    /// Resets the iterator to be `!valid()` again (before first element)
    fn reset(&mut self);
    /// Returns true if `current()` would return a valid item.
    fn valid(&self) -> bool;
    /// Return the current item.
    fn current(&self) -> Option<Self::Item>;
    /// Go to the previous item.
    fn prev(&mut self) -> Option<Self::Item>;

    fn seek_to_first(&mut self) {
        self.reset();
        self.next();
    }
}
