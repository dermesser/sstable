use cmp::{Cmp, DefaultCmp};
use types::{current_key_val, SSIterator};

use std::cmp::Ordering;

/// TestSSIter is an SSIterator over a vector, to be used for testing purposes.
pub struct TestSSIter<'a> {
    v: Vec<(&'a [u8], &'a [u8])>,
    ix: usize,
    init: bool,
}

impl<'a> TestSSIter<'a> {
    pub fn new(c: Vec<(&'a [u8], &'a [u8])>) -> TestSSIter<'a> {
        return TestSSIter {
            v: c,
            ix: 0,
            init: false,
        };
    }
}

impl<'a> SSIterator for TestSSIter<'a> {
    fn advance(&mut self) -> bool {
        if self.ix == self.v.len() - 1 {
            self.ix += 1;
            false
        } else if !self.init {
            self.init = true;
            true
        } else {
            self.ix += 1;
            true
        }
    }
    fn reset(&mut self) {
        self.ix = 0;
        self.init = false;
    }
    fn current(&self, key: &mut Vec<u8>, val: &mut Vec<u8>) -> bool {
        if self.init && self.ix < self.v.len() {
            key.clear();
            val.clear();
            key.extend_from_slice(self.v[self.ix].0);
            val.extend_from_slice(self.v[self.ix].1);
            true
        } else {
            false
        }
    }
    fn valid(&self) -> bool {
        self.init && self.ix < self.v.len()
    }
    fn seek(&mut self, k: &[u8]) {
        self.ix = 0;
        self.init = true;
        while self.ix < self.v.len() && DefaultCmp.cmp(self.v[self.ix].0, k) == Ordering::Less {
            self.ix += 1;
        }
    }
    fn prev(&mut self) -> bool {
        if !self.init || self.ix == 0 {
            self.init = false;
            false
        } else {
            self.ix -= 1;
            true
        }
    }
}

/// SSIteratorIter implements std::iter::Iterator for an SSIterator.
pub struct SSIteratorIter<'a, It: 'a> {
    inner: &'a mut It,
}

impl<'a, It: SSIterator> SSIteratorIter<'a, It> {
    pub fn wrap(it: &'a mut It) -> SSIteratorIter<'a, It> {
        SSIteratorIter { inner: it }
    }
}

impl<'a, It: SSIterator> Iterator for SSIteratorIter<'a, It> {
    type Item = (Vec<u8>, Vec<u8>);
    fn next(&mut self) -> Option<Self::Item> {
        SSIterator::next(self.inner)
    }
}

/// This shared test takes an iterator over a set of exactly four elements and tests that it
/// fulfills the generic iterator properties. Every iterator defined in this code base should pass
/// this test.
pub fn test_iterator_properties<It: SSIterator>(mut it: It) {
    assert!(!it.valid());
    assert!(it.advance());
    assert!(it.valid());
    let first = current_key_val(&it);
    assert!(it.advance());
    let second = current_key_val(&it);
    assert!(it.advance());
    let third = current_key_val(&it);
    // fourth (last) element
    assert!(it.advance());
    assert!(it.valid());
    let fourth = current_key_val(&it);
    // past end is invalid
    assert!(!it.advance());
    assert!(!it.valid());

    it.reset();
    it.seek(&fourth.as_ref().unwrap().0);
    assert!(it.valid());
    it.seek(&second.as_ref().unwrap().0);
    assert!(it.valid());
    it.prev();
    assert_eq!(first, current_key_val(&it));

    it.reset();
    assert!(!it.valid());
    assert!(it.advance());
    assert_eq!(first, current_key_val(&it));
    assert!(it.advance());
    assert_eq!(second, current_key_val(&it));
    assert!(it.advance());
    assert_eq!(third, current_key_val(&it));
    assert!(it.prev());
    assert_eq!(second, current_key_val(&it));
    assert!(it.prev());
    assert_eq!(first, current_key_val(&it));
    assert!(!it.prev());
    assert!(!it.valid());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_util_basic() {
        let v = vec![
            ("abc".as_bytes(), "def".as_bytes()),
            ("abd".as_bytes(), "deg".as_bytes()),
        ];
        let mut iter = TestSSIter::new(v);
        assert_eq!(
            iter.next(),
            Some((Vec::from("abc".as_bytes()), Vec::from("def".as_bytes())))
        );
    }

    #[test]
    fn test_test_util_ssiter_properties() {
        time_test!();
        let v;
        {
            time_test!("init");
            v = vec![
                ("abc".as_bytes(), "def".as_bytes()),
                ("abd".as_bytes(), "deg".as_bytes()),
                ("abe".as_bytes(), "deg".as_bytes()),
                ("abf".as_bytes(), "deg".as_bytes()),
            ];
        }
        test_iterator_properties(TestSSIter::new(v));
    }
}
