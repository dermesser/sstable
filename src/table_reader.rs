use crate::block::{Block, BlockIter};
use crate::blockhandle::BlockHandle;
use crate::cache;
use crate::error::Result;
use crate::filter_block::FilterBlockReader;
use crate::options::Options;
use crate::table_block;
use crate::table_builder::{self, Footer};
use crate::types::{current_key_val, RandomAccess, SSIterator};

use std::cmp::Ordering;
use std::fs;
use std::path;
use std::sync::Arc;

use integer_encoding::FixedIntWriter;

/// Reads the table footer.
fn read_footer(f: &dyn RandomAccess, size: usize) -> Result<Footer> {
    let mut buf = vec![0; table_builder::FULL_FOOTER_LENGTH];
    f.read_at(size - table_builder::FULL_FOOTER_LENGTH, &mut buf)?;
    Ok(Footer::decode(&buf))
}

/// `Table` is used for accessing SSTables.
#[derive(Clone)]
pub struct Table {
    file: Arc<Box<dyn RandomAccess>>,
    file_size: usize,
    cache_id: cache::CacheID,

    opt: Options,

    footer: Footer,
    index_block: Block,
    filters: Option<FilterBlockReader>,
}

impl Table {
    /// Creates a new table reader from a file at `path`.
    pub fn new_from_file(opt: Options, path: &path::Path) -> Result<Table> {
        let f = fs::OpenOptions::new().read(true).open(path)?;
        let size = f.metadata()?.len() as usize;
        Table::new(opt, Box::new(f), size)
    }

    /// Creates a new table reader.
    pub fn new(opt: Options, file: Box<dyn RandomAccess>, size: usize) -> Result<Table> {
        let footer = read_footer(file.as_ref(), size)?;
        let index_block = table_block::read_table_block(opt.clone(), file.as_ref(), &footer.index)?;
        let metaindex_block =
            table_block::read_table_block(opt.clone(), file.as_ref(), &footer.meta_index)?;

        let filter_block_reader = Table::read_filter_block(&metaindex_block, file.as_ref(), &opt)?;
        let cache_id = {
            let mut block_cache = opt.block_cache.write()?;
            block_cache.new_cache_id()
        };

        Ok(Table {
            file: Arc::new(file),
            file_size: size,
            cache_id: cache_id,
            opt: opt,
            footer: footer,
            filters: filter_block_reader,
            index_block: index_block,
        })
    }

    fn read_filter_block(
        metaix: &Block,
        file: &dyn RandomAccess,
        options: &Options,
    ) -> Result<Option<FilterBlockReader>> {
        // Open filter block for reading
        let filter_name = format!("filter.{}", options.filter_policy.name())
            .as_bytes()
            .to_vec();

        let mut metaindexiter = metaix.iter();
        metaindexiter.seek(&filter_name);

        if let Some((_key, val)) = current_key_val(&metaindexiter) {
            let filter_block_location = BlockHandle::decode(&val).0;
            if filter_block_location.size() > 0 {
                return Ok(Some(table_block::read_filter_block(
                    file,
                    &filter_block_location,
                    options.filter_policy.clone(),
                )?));
            }
        }
        Ok(None)
    }

    /// block_cache_handle creates a CacheKey for a block with a given offset to be used in the
    /// block cache.
    fn block_cache_handle(&self, block_off: usize) -> cache::CacheKey {
        let mut dst = [0; 2 * 8];
        (&mut dst[..8])
            .write_fixedint(self.cache_id)
            .expect("error writing to vec");
        (&mut dst[8..])
            .write_fixedint(block_off as u64)
            .expect("error writing to vec");
        dst
    }

    /// Read a block from the current table at `location`, and cache it in the options' block
    /// cache.
    fn read_block(&self, location: &BlockHandle) -> Result<Block> {
        let cachekey = self.block_cache_handle(location.offset());
        let mut block_cache = self.opt.block_cache.write()?;
        if let Some(block) = block_cache.get(&cachekey) {
            return Ok(block.clone());
        }

        // Two times as_ref(): First time to get a ref from Rc<>, then one from Box<>.
        let b =
            table_block::read_table_block(self.opt.clone(), self.file.as_ref().as_ref(), location)?;

        // insert a cheap copy (Arc).
        block_cache.insert(&cachekey, b.clone());

        Ok(b)
    }

    /// Returns the offset of the block that contains `key`.
    pub fn approx_offset_of(&self, key: &[u8]) -> usize {
        let mut iter = self.index_block.iter();

        iter.seek(key);

        if let Some((_, val)) = current_key_val(&iter) {
            let location = BlockHandle::decode(&val).0;
            return location.offset();
        }

        return self.footer.meta_index.offset();
    }

    /// Returns an iterator over an SSTable. Iterators hold internal references to the table, so
    /// make sure to let them expire when not needed anymore.
    pub fn iter(&self) -> TableIterator {
        let iter = TableIterator {
            current_block: None,
            current_block_off: 0,
            index_block: self.index_block.iter(),
            table: self.clone(),
        };
        iter
    }

    /// Retrieve an entry for a key from the table. This function uses the attached filters, so
    /// is better suited if you frequently look for non-existing values (as it will detect the
    /// non-existence of an entry in a block without having to load the block).
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut index_iter = self.index_block.iter();
        index_iter.seek(key);

        let handle;
        if let Some((last_in_block, h)) = current_key_val(&index_iter) {
            if self.opt.cmp.cmp(key, &last_in_block) == Ordering::Less {
                handle = BlockHandle::decode(&h).0;
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }

        // found correct block.

        // Check bloom (or whatever) filter
        if let Some(ref filters) = self.filters {
            if !filters.key_may_match(handle.offset(), key) {
                return Ok(None);
            }
        }

        // Read block (potentially from cache)
        let tb = self.read_block(&handle)?;
        let mut iter = tb.iter();

        // Go to entry and check if it's the wanted entry.
        iter.seek(key);
        if let Some((k, v)) = current_key_val(&iter) {
            if self.opt.cmp.cmp(&k, key) == Ordering::Equal {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
}

/// This iterator is a "TwoLevelIterator"; it uses an index block in order to get an offset hint
/// into the data blocks.
pub struct TableIterator {
    // A TableIterator is independent of its table (on the syntax level -- it doesn't know its
    // Table's lifetime). This is mainly required by the dynamic iterators used everywhere, where a
    // lifetime makes things like returning an iterator from a function neigh-impossible.
    //
    // Instead, reference-counted pointers and locks inside the Table ensure that all
    // TableIterators still share a table.
    table: Table,
    current_block: Option<BlockIter>,
    current_block_off: usize,
    index_block: BlockIter,
}

impl TableIterator {
    // Skips to the entry referenced by the next entry in the index block.
    // This is called once a block has run out of entries.
    // Err means corruption or I/O error; Ok(true) means a new block was loaded; Ok(false) means
    // tht there's no more entries.
    fn skip_to_next_entry(&mut self) -> Result<bool> {
        if let Some((_key, val)) = self.index_block.next() {
            self.load_block(&val).map(|_| true)
        } else {
            Ok(false)
        }
    }

    // Load the block at `handle` into `self.current_block`
    fn load_block(&mut self, handle: &[u8]) -> Result<()> {
        let (new_block_handle, _) = BlockHandle::decode(handle);
        let block = self.table.read_block(&new_block_handle)?;

        self.current_block = Some(block.iter());
        self.current_block_off = new_block_handle.offset();

        Ok(())
    }
}

impl SSIterator for TableIterator {
    fn advance(&mut self) -> bool {
        // Uninitialized case.
        if self.current_block.is_none() {
            match self.skip_to_next_entry() {
                Ok(true) => return self.advance(),
                Ok(false) => {
                    self.reset();
                    return false;
                }
                // try next block from index, this might be corruption
                Err(_) => return self.advance(),
            }
        }

        // Initialized case -- does the current block have more entries?
        if let Some(ref mut cb) = self.current_block {
            if cb.advance() {
                return true;
            }
        }

        // If the current block is exhausted, try loading the next block.
        self.current_block = None;
        match self.skip_to_next_entry() {
            Ok(true) => self.advance(),
            Ok(false) => {
                self.reset();
                false
            }
            // try next block, this might be corruption
            Err(_) => self.advance(),
        }
    }

    // A call to valid() after seeking is necessary to ensure that the seek worked (e.g., no error
    // while reading from disk)
    fn seek(&mut self, to: &[u8]) {
        // first seek in index block, rewind by one entry (so we get the next smaller index entry),
        // then set current_block and seek there
        self.index_block.seek(to);

        // It's possible that this is a seek past-last; reset in that case.
        if let Some((past_block, handle)) = current_key_val(&self.index_block) {
            if self.table.opt.cmp.cmp(to, &past_block) <= Ordering::Equal {
                // ok, found right block: continue
                if let Ok(()) = self.load_block(&handle) {
                    // current_block is always set if load_block() returned Ok.
                    self.current_block.as_mut().unwrap().seek(to);
                    return;
                }
            }
        }
        // Reached in case of failure.
        self.reset();
    }

    fn prev(&mut self) -> bool {
        // happy path: current block contains previous entry
        if let Some(ref mut cb) = self.current_block {
            if cb.prev() {
                return true;
            }
        }

        // Go back one block and look for the last entry in the previous block
        if self.index_block.prev() {
            if let Some((_, handle)) = current_key_val(&self.index_block) {
                if self.load_block(&handle).is_ok() {
                    self.current_block.as_mut().unwrap().seek_to_last();
                    self.current_block.as_ref().unwrap().valid()
                } else {
                    self.reset();
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    fn reset(&mut self) {
        self.index_block.reset();
        self.current_block = None;
    }

    // This iterator is special in that it's valid even before the first call to advance(). It
    // behaves correctly, though.
    fn valid(&self) -> bool {
        self.current_block.is_some() && (self.current_block.as_ref().unwrap().valid())
    }

    fn current(&self, key: &mut Vec<u8>, val: &mut Vec<u8>) -> bool {
        if let Some(ref cb) = self.current_block {
            cb.current(key, val)
        } else {
            false
        }
    }

    fn current_key(&self) -> Option<&[u8]> {
        if let Some(ref cb) = self.current_block {
            cb.current_key()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::options::CompressionType;
    use crate::table_builder::TableBuilder;
    use crate::test_util::{test_iterator_properties, SSIteratorIter};
    use crate::types::{current_key_val, SSIterator};

    use super::*;

    const LOCK_POISONED: &str = "Lock poisoned";

    fn build_data() -> Vec<(&'static str, &'static str)> {
        vec![
            // block 1
            ("abc", "def"),
            ("abd", "dee"),
            ("bcd", "asa"),
            // block 2
            ("bsr", "a00"),
            ("xyz", "xxx"),
            ("xzz", "yyy"),
            // block 3
            ("zzz", "111"),
        ]
    }

    // Build a table containing raw keys (no format). It returns (vector, length) for convenience
    // reason, a call f(v, v.len()) doesn't work for borrowing reasons.
    fn build_table(data: Vec<(&'static str, &'static str)>) -> (Vec<u8>, usize) {
        let mut d = Vec::with_capacity(512);
        let mut opt = Options::default();
        opt.block_restart_interval = 2;
        opt.block_size = 32;
        opt.compression_type = CompressionType::CompressionSnappy;

        {
            // Uses the standard comparator in opt.
            let mut b = TableBuilder::new(opt, &mut d);

            for &(k, v) in data.iter() {
                b.add(k.as_bytes(), v.as_bytes()).unwrap();
            }

            b.finish().unwrap();
        }

        let size = d.len();
        (d, size)
    }

    fn wrap_buffer(src: Vec<u8>) -> Box<dyn RandomAccess> {
        Box::new(src)
    }

    #[test]
    fn test_table_approximate_offset() {
        let (src, size) = build_table(build_data());
        let mut opt = Options::default();
        opt.block_size = 32;
        let table = Table::new(opt, wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();

        let expected_offsets = vec![0, 0, 0, 44, 44, 44, 89];
        let mut i = 0;
        for (k, _) in SSIteratorIter::wrap(&mut iter) {
            assert_eq!(expected_offsets[i], table.approx_offset_of(&k));
            i += 1;
        }

        // Key-past-last returns offset of metaindex block.
        assert_eq!(137, table.approx_offset_of("{aa".as_bytes()));
    }

    #[test]
    fn test_table_block_cache_use() {
        let (src, size) = build_table(build_data());
        let mut opt = Options::default();
        opt.block_size = 32;

        let table = Table::new(opt.clone(), wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();

        // index/metaindex blocks are not cached. That'd be a waste of memory.
        assert_eq!(opt.block_cache.read().expect(LOCK_POISONED).count(), 0);

        iter.next();
        assert_eq!(opt.block_cache.read().expect(LOCK_POISONED).count(), 1);

        // This may fail if block parameters or data change. In that case, adapt it.
        iter.next();
        iter.next();
        iter.next();
        iter.next();
        assert_eq!(opt.block_cache.read().expect(LOCK_POISONED).count(), 2);
    }

    #[test]
    fn test_table_iterator_fwd_bwd() {
        let (src, size) = build_table(build_data());
        let data = build_data();

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();
        let mut i = 0;

        while let Some((k, v)) = iter.next() {
            assert_eq!(
                (data[i].0.as_bytes(), data[i].1.as_bytes()),
                (k.as_ref(), v.as_ref())
            );
            i += 1;
        }

        assert_eq!(i, data.len());
        assert!(!iter.valid());

        // Go forward again, to last entry.
        while let Some((key, _)) = iter.next() {
            if key.as_slice() == b"zzz" {
                break;
            }
        }

        assert!(iter.valid());
        // backwards count
        let mut j = 0;

        while iter.prev() {
            if let Some((k, v)) = current_key_val(&iter) {
                j += 1;
                assert_eq!(
                    (
                        data[data.len() - 1 - j].0.as_bytes(),
                        data[data.len() - 1 - j].1.as_bytes()
                    ),
                    (k.as_ref(), v.as_ref())
                );
            } else {
                break;
            }
        }

        // expecting 7 - 1, because the last entry that the iterator stopped on is the last entry
        // in the table; that is, it needs to go back over 6 entries.
        assert_eq!(j, 6);
    }

    #[test]
    fn test_table_iterator_filter() {
        let (src, size) = build_table(build_data());

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        assert!(table.filters.is_some());
        let filter_reader = table.filters.clone().unwrap();
        let mut iter = table.iter();

        loop {
            if let Some((k, _)) = iter.next() {
                assert!(filter_reader.key_may_match(iter.current_block_off, &k));
                assert!(!filter_reader.key_may_match(iter.current_block_off, b"somerandomkey"));
            } else {
                break;
            }
        }
    }

    #[test]
    fn test_table_iterator_state_behavior() {
        let (src, size) = build_table(build_data());

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();

        // behavior test

        // See comment on valid()
        assert!(!iter.valid());
        assert!(current_key_val(&iter).is_none());
        assert!(!iter.prev());

        assert!(iter.advance());
        let first = current_key_val(&iter);
        assert!(iter.valid());
        assert!(current_key_val(&iter).is_some());

        assert!(iter.advance());
        assert!(iter.prev());
        assert!(iter.valid());

        iter.reset();
        assert!(!iter.valid());
        assert!(current_key_val(&iter).is_none());
        assert_eq!(first, iter.next());
    }

    #[test]
    fn test_table_iterator_behavior_standard() {
        let mut data = build_data();
        data.truncate(4);
        let (src, size) = build_table(data);
        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        test_iterator_properties(table.iter());
    }

    #[test]
    fn test_table_iterator_values() {
        let (src, size) = build_table(build_data());
        let data = build_data();

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();
        let mut i = 0;

        iter.next();
        iter.next();

        // Go back to previous entry, check, go forward two entries, repeat
        // Verifies that prev/next works well.
        loop {
            iter.prev();

            if let Some((k, v)) = current_key_val(&iter) {
                assert_eq!(
                    (data[i].0.as_bytes(), data[i].1.as_bytes()),
                    (k.as_ref(), v.as_ref())
                );
            } else {
                break;
            }

            i += 1;
            if iter.next().is_none() || iter.next().is_none() {
                break;
            }
        }

        // Skipping the last value because the second next() above will break the loop
        assert_eq!(i, 6);
    }

    #[test]
    fn test_table_iterator_seek() {
        let (src, size) = build_table(build_data());

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        let mut iter = table.iter();

        iter.seek(b"bcd");
        assert!(iter.valid());
        assert_eq!(
            current_key_val(&iter),
            Some((b"bcd".to_vec(), b"asa".to_vec()))
        );
        iter.seek(b"abc");
        assert!(iter.valid());
        assert_eq!(
            current_key_val(&iter),
            Some((b"abc".to_vec(), b"def".to_vec()))
        );

        // Seek-past-last invalidates.
        iter.seek("{{{".as_bytes());
        assert!(!iter.valid());
        iter.seek(b"bbb");
        assert!(iter.valid());
    }

    #[test]
    fn test_table_get() {
        let (src, size) = build_table(build_data());

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();
        let table2 = table.clone();

        let mut _iter = table.iter();
        // Test that all of the table's entries are reachable via get()
        for (k, v) in SSIteratorIter::wrap(&mut _iter) {
            let r = table2.get(&k);
            assert_eq!(Ok(Some(v)), r);
        }

        assert_eq!(
            table.opt.block_cache.read().expect(LOCK_POISONED).count(),
            3
        );

        // test that filters work and don't return anything at all.
        assert!(table.get(b"aaa").unwrap().is_none());
        assert!(table.get(b"aaaa").unwrap().is_none());
        assert!(table.get(b"aa").unwrap().is_none());
        assert!(table.get(b"abcd").unwrap().is_none());
        assert!(table.get(b"abb").unwrap().is_none());
        assert!(table.get(b"xyy").unwrap().is_none());
        assert!(table.get(b"zzy").unwrap().is_none());
        assert!(table.get(b"zz1").unwrap().is_none());
        assert!(table.get("zz{".as_bytes()).unwrap().is_none());
    }

    #[test]
    fn test_table_reader_checksum() {
        let (mut src, size) = build_table(build_data());

        src[10] += 1;

        let table = Table::new(Options::default(), wrap_buffer(src), size).unwrap();

        assert!(table.filters.is_some());
        assert_eq!(table.filters.as_ref().unwrap().num(), 1);

        {
            let mut _iter = table.iter();
            let iter = SSIteratorIter::wrap(&mut _iter);
            // first block is skipped
            assert_eq!(iter.count(), 4);
        }

        {
            let mut _iter = table.iter();
            let iter = SSIteratorIter::wrap(&mut _iter);

            for (k, _) in iter {
                if k == build_data()[5].0.as_bytes() {
                    return;
                }
            }

            panic!("Should have hit 5th record in table!");
        }
    }
}
