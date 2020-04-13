//! A collection of fundamental and/or simple types used by other modules. A bit of a grab bag :-)

use crate::error::Result;

use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::sync::Arc;
use std::sync::RwLock;

pub trait RandomAccess: Send + Sync {
    fn read_at(&self, off: usize, dst: &mut [u8]) -> Result<usize>;
}

/// BufferBackedFile is a simple type implementing RandomAccess on a Vec<u8>. Used for some tests.
#[allow(unused)]
pub type BufferBackedFile = Vec<u8>;

impl RandomAccess for BufferBackedFile {
    fn read_at(&self, off: usize, dst: &mut [u8]) -> Result<usize> {
        if off > self.len() {
            return Ok(0);
        }
        let remaining = self.len() - off;
        let to_read = if dst.len() > remaining {
            remaining
        } else {
            dst.len()
        };
        (&mut dst[0..to_read]).copy_from_slice(&self[off..off + to_read]);
        Ok(to_read)
    }
}

#[cfg(unix)]
impl RandomAccess for File {
    fn read_at(&self, off: usize, dst: &mut [u8]) -> Result<usize> {
        Ok((self as &dyn FileExt).read_at(dst, off as u64)?)
    }
}

#[cfg(windows)]
impl RandomAccess for File {
    fn read_at(&self, off: usize, dst: &mut [u8]) -> Result<usize> {
        Ok((self as &dyn FileExt).seek_read(dst, off as u64)?)
    }
}

/// A shared thingy with guarded by a lock.
pub type Shared<T> = Arc<RwLock<T>>;

pub fn share<T>(t: T) -> Arc<RwLock<T>> {
    Arc::new(RwLock::new(t))
}

/// An extension of the standard `Iterator` trait that supporting some additional functionality.
///
/// Note: Implementing types are expected to hold `!valid()` before the first call to `advance()`,
/// and after `advance()` has returned `false` for the first time.
///
/// test_util::test_iterator_properties() verifies that all properties hold for a given
/// implementation.
pub trait SSIterator {
    /// Advances the position of the iterator by one element (which can be retrieved using
    /// current(). If no more elements are available, advance() returns false, and the iterator
    /// becomes invalid (i.e. as if reset() had been called).
    fn advance(&mut self) -> bool;
    /// Return the current item (i.e. the item most recently returned by `next()`).
    fn current(&self, key: &mut Vec<u8>, val: &mut Vec<u8>) -> bool;
    /// Return a reference to the key of the current item (i.e. the item most recently returned by `next()`).
    fn current_key(&self) -> Option<&[u8]>;
    /// Seek the iterator to `key` or the next bigger key. If the seek is invalid (past last
    /// element, or before first element), the iterator is `reset()` and not valid.
    fn seek(&mut self, key: &[u8]);
    /// Resets the iterator to be `!valid()`, i.e. positioned before the first element.
    fn reset(&mut self);
    /// Returns true if the iterator is not positioned before the first or after the last element,
    /// i.e. if `current()` would succeed.
    fn valid(&self) -> bool;
    /// Go to the previous item; if the iterator is moved beyond the first element, `prev()`
    /// returns false and it will be `!valid()`. This is inefficient for most iterator
    /// implementations.
    fn prev(&mut self) -> bool;

    // default implementations.

    /// next is like Iterator::next(). It's implemented here because Rust disallows implementing a
    /// foreign trait for any type, thus we can't do `impl<T: SSIterator> Iterator<Item=Vec<u8>>
    /// for T {}`.
    fn next(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        if !self.advance() {
            return None;
        }
        let (mut key, mut val) = (vec![], vec![]);
        if self.current(&mut key, &mut val) {
            Some((key, val))
        } else {
            None
        }
    }

    /// seek_to_first seeks to the first element.
    fn seek_to_first(&mut self) {
        self.reset();
        self.advance();
    }
}

/// current_key_val is a helper allocating two vectors and filling them with the current key/value
/// of the specified iterator.
pub fn current_key_val<It: SSIterator + ?Sized>(it: &It) -> Option<(Vec<u8>, Vec<u8>)> {
    let (mut k, mut v) = (vec![], vec![]);
    if it.current(&mut k, &mut v) {
        Some((k, v))
    } else {
        None
    }
}

impl SSIterator for Box<dyn SSIterator> {
    fn advance(&mut self) -> bool {
        self.as_mut().advance()
    }
    fn current(&self, key: &mut Vec<u8>, val: &mut Vec<u8>) -> bool {
        self.as_ref().current(key, val)
    }
    fn current_key(&self) -> Option<&[u8]> {
        self.as_ref().current_key()
    }
    fn seek(&mut self, key: &[u8]) {
        self.as_mut().seek(key)
    }
    fn reset(&mut self) {
        self.as_mut().reset()
    }
    fn valid(&self) -> bool {
        self.as_ref().valid()
    }
    fn prev(&mut self) -> bool {
        self.as_mut().prev()
    }
}

const MASK_DELTA: u32 = 0xa282ead8;

pub fn mask_crc(c: u32) -> u32 {
    (c.wrapping_shr(15) | c.wrapping_shl(17)).wrapping_add(MASK_DELTA)
}

pub fn unmask_crc(mc: u32) -> u32 {
    let rot = mc.wrapping_sub(MASK_DELTA);
    rot.wrapping_shr(17) | rot.wrapping_shl(15)
}

#[cfg(test)]
mod tests {}
