#![allow(dead_code)]

use block::{Block, BlockIter};
use blockhandle::BlockHandle;
use filter::BoxedFilterPolicy;
use filter_block::FilterBlockReader;
use options::{self, CompressionType, Options};
use table_builder::{self, Footer};
use iterator::SSIterator;

use std::cmp::Ordering;
use std::io::{self, Read, Seek, SeekFrom, Result};

use integer_encoding::FixedInt;
use crc::crc32::{self, Hasher32};

/// Reads the table footer.
fn read_footer<R: Read + Seek>(f: &mut R, size: usize) -> Result<Footer> {
    try!(f.seek(SeekFrom::Start((size - table_builder::FULL_FOOTER_LENGTH) as u64)));
    let mut buf = [0; table_builder::FULL_FOOTER_LENGTH];
    try!(f.read_exact(&mut buf));
    Ok(Footer::decode(&buf))
}

fn read_bytes<R: Read + Seek>(f: &mut R, location: &BlockHandle) -> Result<Vec<u8>> {
    try!(f.seek(SeekFrom::Start(location.offset() as u64)));

    let mut buf = Vec::new();
    buf.resize(location.size(), 0);

    try!(f.read_exact(&mut buf[0..location.size()]));

    Ok(buf)
}

struct TableBlock {
    block: Block,
    checksum: u32,
    compression: CompressionType,
}

impl TableBlock {
    /// Reads a block at location.
    fn read_block<R: Read + Seek>(opt: Options,
                                  f: &mut R,
                                  location: &BlockHandle)
                                  -> Result<TableBlock> {
        // The block is denoted by offset and length in BlockHandle. A block in an encoded
        // table is followed by 1B compression type and 4B checksum.
        let buf = try!(read_bytes(f, location));
        let compress = try!(read_bytes(f,
                                       &BlockHandle::new(location.offset() + location.size(),
                                                         table_builder::TABLE_BLOCK_COMPRESS_LEN)));
        let cksum = try!(read_bytes(f,
                                    &BlockHandle::new(location.offset() + location.size() +
                                                      table_builder::TABLE_BLOCK_COMPRESS_LEN,
                                                      table_builder::TABLE_BLOCK_CKSUM_LEN)));
        Ok(TableBlock {
            block: Block::new(opt, buf),
            checksum: u32::decode_fixed(&cksum),
            compression: options::int_to_compressiontype(compress[0] as u32)
                .unwrap_or(CompressionType::CompressionNone),
        })
    }

    /// Verify checksum of block
    fn verify(&self) -> bool {
        let mut digest = crc32::Digest::new(crc32::CASTAGNOLI);
        digest.write(&self.block.contents());
        digest.write(&[self.compression as u8; 1]);

        digest.sum32() == self.checksum
    }
}

pub struct Table<R: Read + Seek> {
    file: R,
    opt: Options,

    footer: Footer,
    indexblock: Block,
    #[allow(dead_code)]
    filters: Option<FilterBlockReader>,
}

impl<R: Read + Seek> Table<R> {
    /// Creates a new table reader.
    pub fn new(opt: Options, mut file: R, size: usize, fp: BoxedFilterPolicy) -> Result<Table<R>> {
        let footer = try!(read_footer(&mut file, size));

        let indexblock = try!(TableBlock::read_block(opt.clone(), &mut file, &footer.index));
        let metaindexblock =
            try!(TableBlock::read_block(opt.clone(), &mut file, &footer.meta_index));

        if !indexblock.verify() || !metaindexblock.verify() {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                                      "Indexblock/Metaindexblock failed verification"));
        }

        // Open filter block for reading
        let mut filter_block_reader = None;
        let filter_name = format!("filter.{}", fp.name()).as_bytes().to_vec();

        let mut metaindexiter = metaindexblock.block.iter();

        metaindexiter.seek(&filter_name);

        if let Some((_key, val)) = metaindexiter.current() {
            let filter_block_location = BlockHandle::decode(&val).0;

            if filter_block_location.size() > 0 {
                let buf = try!(read_bytes(&mut file, &filter_block_location));
                filter_block_reader = Some(FilterBlockReader::new_owned(fp, buf));
            }
        }

        metaindexiter.reset();

        Ok(Table {
            file: file,
            opt: opt,
            footer: footer,
            filters: filter_block_reader,
            indexblock: indexblock.block,
        })
    }

    fn read_block(&mut self, location: &BlockHandle) -> Result<TableBlock> {
        let b = try!(TableBlock::read_block(self.opt.clone(), &mut self.file, location));

        if !b.verify() {
            Err(io::Error::new(io::ErrorKind::InvalidData, "Data block failed verification"))
        } else {
            Ok(b)
        }
    }

    /// Returns the offset of the block that contains `key`.
    pub fn approx_offset_of(&self, key: &[u8]) -> usize {
        let mut iter = self.indexblock.iter();

        iter.seek(key);

        if let Some((_, val)) = iter.current() {
            let location = BlockHandle::decode(&val).0;
            return location.offset();
        }

        return self.footer.meta_index.offset();
    }

    // Iterators read from the file; thus only one iterator can be borrowed (mutably) per scope
    fn iter<'a>(&'a mut self) -> TableIterator<'a, R> {
        let iter = TableIterator {
            current_block: self.indexblock.iter(),
            init: false,
            current_block_off: 0,
            index_block: self.indexblock.iter(),
            opt: self.opt.clone(),
            table: self,
        };
        iter
    }

    /// Retrieve value from table. This function uses the attached filters, so is better suited if
    /// you frequently look for non-existing values (as it will detect the non-existence of an
    /// entry in a block without having to load the block).
    pub fn get(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let mut index_iter = self.indexblock.iter();
        index_iter.seek(key);

        let handle;
        if let Some((last_in_block, h)) = index_iter.current() {
            if self.opt.cmp.cmp(key, &last_in_block) == Ordering::Less {
                handle = BlockHandle::decode(&h).0;
            } else {
                return None;
            }
        } else {
            return None;
        }

        // found correct block.
        // Check bloom filter
        if let Some(ref filters) = self.filters {
            if !filters.key_may_match(handle.offset(), key) {
                return None;
            }
        }

        // Read block (potentially from cache)
        let mut iter;
        if let Ok(tb) = self.read_block(&handle) {
            iter = tb.block.iter();
        } else {
            return None;
        }

        // Go to entry and check if it's the wanted entry.
        iter.seek(key);
        if let Some((k, v)) = iter.current() {
            if self.opt.cmp.cmp(key, &k) == Ordering::Equal {
                Some(v)
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// This iterator is a "TwoLevelIterator"; it uses an index block in order to get an offset hint
/// into the data blocks.
pub struct TableIterator<'a, R: 'a + Read + Seek> {
    table: &'a mut Table<R>,
    opt: Options,
    // We're not using Option<BlockIter>, but instead a separate `init` field. That makes it easier
    // working with the current block in the iterator methods (no borrowing annoyance as with
    // Option<>)
    current_block: BlockIter,
    current_block_off: usize,
    init: bool,
    index_block: BlockIter,
}

impl<'a, R: Read + Seek> TableIterator<'a, R> {
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

        let block = try!(self.table.read_block(&new_block_handle));

        self.current_block = block.block.iter();
        self.current_block_off = new_block_handle.offset();

        Ok(())
    }
}

impl<'a, R: Read + Seek> Iterator for TableIterator<'a, R> {
    type Item = (Vec<u8>, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        // init essentially means that `current_block` is a data block (it's initially filled with
        // an index block as filler).
        if self.init {
            if let Some((key, val)) = self.current_block.next() {
                Some((key, val))
            } else {
                match self.skip_to_next_entry() {
                    Ok(true) => self.next(),
                    Ok(false) => None,
                    // try next block, this might be corruption
                    Err(_) => self.next(),
                }
            }
        } else {
            match self.skip_to_next_entry() {
                Ok(true) => {
                    // Only initialize if we got an entry
                    self.init = true;
                    self.next()
                }
                Ok(false) => None,
                // try next block from index, this might be corruption
                Err(_) => self.next(),
            }
        }
    }
}

impl<'a, R: Read + Seek> SSIterator for TableIterator<'a, R> {
    // A call to valid() after seeking is necessary to ensure that the seek worked (e.g., no error
    // while reading from disk)
    fn seek(&mut self, to: &[u8]) {
        // first seek in index block, rewind by one entry (so we get the next smaller index entry),
        // then set current_block and seek there

        self.index_block.seek(to);

        if let Some((past_block, handle)) = self.index_block.current() {
            if self.opt.cmp.cmp(to, &past_block) <= Ordering::Equal {
                // ok, found right block: continue
                if let Ok(()) = self.load_block(&handle) {
                    self.current_block.seek(to);
                    self.init = true;
                } else {
                    self.reset();
                    return;
                }
            } else {
                self.reset();
                return;
            }
        } else {
            panic!("Unexpected None from current() (bug)");
        }
    }

    fn prev(&mut self) -> Option<Self::Item> {
        // happy path: current block contains previous entry
        if let Some(result) = self.current_block.prev() {
            Some(result)
        } else {
            // Go back one block and look for the last entry in the previous block
            if let Some((_, handle)) = self.index_block.prev() {
                if self.load_block(&handle).is_ok() {
                    self.current_block.seek_to_last();
                    self.current_block.current()
                } else {
                    self.reset();
                    None
                }
            } else {
                None
            }
        }
    }

    fn reset(&mut self) {
        self.index_block.reset();
        self.init = false;
    }

    // This iterator is special in that it's valid even before the first call to next(). It behaves
    // correctly, though.
    fn valid(&self) -> bool {
        self.init && (self.current_block.valid() || self.index_block.valid())
    }

    fn current(&self) -> Option<Self::Item> {
        if self.init {
            self.current_block.current()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use filter::BloomPolicy;
    use options::Options;
    use table_builder::TableBuilder;
    use iterator::SSIterator;

    use std::io::Cursor;

    use super::*;

    fn build_data() -> Vec<(&'static str, &'static str)> {
        vec![// block 1
             ("abc", "def"),
             ("abd", "dee"),
             ("bcd", "asa"),
             // block 2
             ("bsr", "a00"),
             ("xyz", "xxx"),
             ("xzz", "yyy"),
             // block 3
             ("zzz", "111")]
    }

    // Build a table containing raw keys (no format)
    fn build_table() -> (Vec<u8>, usize) {
        let mut d = Vec::with_capacity(512);
        let mut opt = Options::default();
        opt.block_restart_interval = 2;
        opt.block_size = 32;

        {
            // Uses the standard comparator in opt.
            let mut b = TableBuilder::new(opt, &mut d, BloomPolicy::new(4));
            let data = build_data();

            for &(k, v) in data.iter() {
                b.add(k.as_bytes(), v.as_bytes());
            }

            b.finish();

        }

        let size = d.len();

        (d, size)
    }

    #[test]
    fn test_table_reader_checksum() {
        let (mut src, size) = build_table();
        println!("{}", size);

        src[10] += 1;

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();

        assert!(table.filters.is_some());
        assert_eq!(table.filters.as_ref().unwrap().num(), 1);

        {
            let iter = table.iter();
            // first block is skipped
            assert_eq!(iter.count(), 4);
        }

        {
            let iter = table.iter();

            for (k, _) in iter {
                if k == build_data()[5].0.as_bytes() {
                    return;
                }
            }

            panic!("Should have hit 5th record in table!");
        }
    }

    #[test]
    fn test_table_iterator_fwd_bwd() {
        let (src, size) = build_table();
        let data = build_data();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();
        let mut iter = table.iter();
        let mut i = 0;

        while let Some((k, v)) = iter.next() {
            assert_eq!((data[i].0.as_bytes(), data[i].1.as_bytes()),
                       (k.as_ref(), v.as_ref()));
            i += 1;
        }

        assert_eq!(i, data.len());
        assert!(iter.next().is_none());

        // backwards count
        let mut j = 0;

        while let Some((k, v)) = iter.prev() {
            j += 1;
            assert_eq!((data[data.len() - 1 - j].0.as_bytes(),
                        data[data.len() - 1 - j].1.as_bytes()),
                       (k.as_ref(), v.as_ref()));
        }

        // expecting 7 - 1, because the last entry that the iterator stopped on is the last entry
        // in the table; that is, it needs to go back over 6 entries.
        assert_eq!(j, 6);
    }

    #[test]
    fn test_table_iterator_filter() {
        let (src, size) = build_table();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();
        let filter_reader = table.filters.clone().unwrap();
        let mut iter = table.iter();

        loop {
            if let Some((k, _)) = iter.next() {
                assert!(filter_reader.key_may_match(iter.current_block_off, &k));
                assert!(!filter_reader.key_may_match(iter.current_block_off,
                                                     "somerandomkey".as_bytes()));
            } else {
                break;
            }
        }
    }

    #[test]
    fn test_table_iterator_state_behavior() {
        let (src, size) = build_table();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();
        let mut iter = table.iter();

        // behavior test

        // See comment on valid()
        assert!(!iter.valid());
        assert!(iter.current().is_none());
        assert!(iter.prev().is_none());

        assert!(iter.next().is_some());
        let first = iter.current();
        assert!(iter.valid());
        assert!(iter.current().is_some());

        assert!(iter.next().is_some());
        assert!(iter.prev().is_some());
        assert!(iter.current().is_some());

        iter.reset();
        assert!(!iter.valid());
        assert!(iter.current().is_none());
        assert_eq!(first, iter.next());
    }

    #[test]
    fn test_table_iterator_values() {
        let (src, size) = build_table();
        let data = build_data();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();
        let mut iter = table.iter();
        let mut i = 0;

        iter.next();
        iter.next();

        // Go back to previous entry, check, go forward two entries, repeat
        // Verifies that prev/next works well.
        loop {
            iter.prev();

            if let Some((k, v)) = iter.current() {
                assert_eq!((data[i].0.as_bytes(), data[i].1.as_bytes()),
                           (k.as_ref(), v.as_ref()));
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
        let (src, size) = build_table();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();
        let mut iter = table.iter();

        iter.seek("bcd".as_bytes());
        assert!(iter.valid());
        assert_eq!(iter.current(),
                   Some(("bcd".as_bytes().to_vec(), "asa".as_bytes().to_vec())));
        iter.seek("abc".as_bytes());
        assert!(iter.valid());
        assert_eq!(iter.current(),
                   Some(("abc".as_bytes().to_vec(), "def".as_bytes().to_vec())));
    }

    #[test]
    fn test_table_get() {
        let (src, size) = build_table();

        let mut table = Table::new(Options::default(),
                                   Cursor::new(&src as &[u8]),
                                   size,
                                   BloomPolicy::new(4))
            .unwrap();

        assert!(table.get("aaa".as_bytes()).is_none());
        assert_eq!(table.get("abc".as_bytes()), Some("def".as_bytes().to_vec()));
        assert!(table.get("abcd".as_bytes()).is_none());
        assert_eq!(table.get("bcd".as_bytes()), Some("asa".as_bytes().to_vec()));
        assert_eq!(table.get("zzz".as_bytes()), Some("111".as_bytes().to_vec()));
        assert!(table.get("zz1".as_bytes()).is_none());
    }
}
