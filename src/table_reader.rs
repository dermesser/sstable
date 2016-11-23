use block::{Block, BlockContents, BlockIter};
use blockhandle::BlockHandle;
use table_builder::{self, Footer};
use iterator::{Comparator, StandardComparator, SSIterator};
use options::ReadOptions;

use integer_encoding::FixedInt;
use crc::crc32;
use crc::Hasher32;

use std::cmp::Ordering;
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Result};
use std::fs::{File, OpenOptions};
use std::path::Path;

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

/// Reads a block at location.
fn read_block<R: Read + Seek>(f: &mut R, location: &BlockHandle) -> Result<BlockContents> {
    let buf = try!(read_bytes(f, location));
    Ok(buf)
}

pub struct Table<R: Read + Seek, C: Comparator> {
    file: R,
    file_size: usize,

    opt: ReadOptions,
    cmp: C,

    indexblock: Block<C>,
}

impl<R: Read + Seek> Table<R, StandardComparator> {
    /// Open a table for reading.
    pub fn new_defaults(file: R, size: usize) -> Result<Table<R, StandardComparator>> {
        Table::new(file, size, ReadOptions::default(), StandardComparator)
    }
}

impl Table<File, StandardComparator> {
    /// Directly open a file for reading.
    pub fn new_from_file(file: &Path) -> Result<Table<File, StandardComparator>> {
        let f = try!(OpenOptions::new().read(true).open(file));
        let len = try!(f.metadata()).len() as usize;

        Table::new(f, len, ReadOptions::default(), StandardComparator)
    }
}

impl<R: Read + Seek, C: Comparator> Table<R, C> {
    /// Open a table for reading. Note: The comparator must be the same that was chosen when
    /// building the table.
    pub fn new(mut file: R, size: usize, opt: ReadOptions, cmp: C) -> Result<Table<R, C>> {
        let footer = try!(read_footer(&mut file, size));

        let indexblock = Block::new(try!(read_block(&mut file, &footer.index)), cmp);

        Ok(Table {
            file: file,
            file_size: size,
            opt: opt,
            cmp: cmp,
            indexblock: indexblock,
        })
    }

    fn read_block_(&mut self, location: &BlockHandle) -> Result<BlockContents> {
        read_block(&mut self.file, location)
    }

    /// Returns the offset of the block that contains `key`.
    pub fn approx_offset_of(&self, key: &[u8]) -> usize {
        let mut iter = self.indexblock.iter();

        iter.seek(key);

        if let Some((_, val)) = iter.current() {
            let location = BlockHandle::decode(&val).0;
            return location.offset();
        }

        return self.file_size;
    }

    // Iterators read from the file; thus only one iterator can be borrowed (mutably) per scope
    pub fn iter<'a>(&'a mut self) -> TableIterator<'a, R, C> {
        let iter = TableIterator {
            current_block: self.indexblock.iter(), // just for filling in here
            index_block: self.indexblock.iter(),
            table: self,
            init: false,
        };
        iter
    }

    /// Retrieve an entry from the table.
    ///
    /// Note: As this doesn't keep state, using a TableIterator and seek() may be more efficient
    /// when retrieving several entries from the same underlying block.
    pub fn get(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let mut iter = self.iter();

        iter.seek(key);

        if let Some((k, v)) = iter.current() {
            if k == key {
                return Some(v);
            }
        }
        return None;
    }
}

/// Iterator over a Table.
pub struct TableIterator<'a, R: 'a + Read + Seek, C: 'a + Comparator> {
    table: &'a mut Table<R, C>,
    current_block: BlockIter<C>,
    index_block: BlockIter<C>,

    init: bool,
}

impl<'a, C: Comparator, R: Read + Seek> TableIterator<'a, R, C> {
    /// Skips to the entry referenced by the next entry in the index block.
    /// This is called once a block has run out of entries.
    /// Returns Ok(false) if the end has been reached, returns Err(...) if it should be retried
    /// (e.g., because there's a corrupted block)
    fn skip_to_next_entry(&mut self) -> Result<bool> {
        if let Some((_key, val)) = self.index_block.next() {
            let r = self.load_block(&val);

            if let Err(e) = r { Err(e) } else { Ok(true) }
        } else {
            Ok(false)
        }
    }

    /// Verifies the CRC checksum of a block.
    fn verify_block(&self, block: &BlockContents) -> bool {
        let payload = &block[0..block.len() - 4];
        let checksum = &block[block.len() - 4..];
        let checksum = u32::decode_fixed(checksum);

        let mut digest = crc32::Digest::new(crc32::CASTAGNOLI);
        digest.write(payload);

        digest.sum32() == checksum
    }

    /// Load the block at `handle` into `self.current_block`
    fn load_block(&mut self, handle: &[u8]) -> Result<()> {
        const TABLE_BLOCK_FOOTER_SIZE: usize = 5;
        let (new_block_handle, _) = BlockHandle::decode(handle);

        // Also read checksum and compression! (5B)
        let full_block_handle = BlockHandle::new(new_block_handle.offset(),
                                                 new_block_handle.size() + TABLE_BLOCK_FOOTER_SIZE);
        let mut full_block = try!(self.table.read_block_(&full_block_handle));

        if !self.verify_block(&full_block) && self.table.opt.skip_bad_blocks {
            Err(Error::new(ErrorKind::InvalidData, "Bad block checksum!".to_string()))
        } else {
            // Truncate by 5, so the checksum and compression type are gone
            full_block.resize(new_block_handle.size(), 0);
            let block = Block::new(full_block, self.table.cmp);
            self.current_block = block.iter();

            Ok(())
        }
    }
}

impl<'a, C: Comparator, R: Read + Seek> Iterator for TableIterator<'a, R, C> {
    type Item = (Vec<u8>, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        if !self.init {
            return match self.skip_to_next_entry() {
                Ok(true) => {
                    self.init = true;
                    self.next()
                }
                Ok(false) => None,
                Err(_) => self.next(),
            };
        }
        if let Some((key, val)) = self.current_block.next() {
            Some((key, val))
        } else {
            match self.skip_to_next_entry() {
                Ok(true) => self.next(),
                Ok(false) => None,
                Err(_) => self.next(),
            }
        }
    }
}

impl<'a, C: Comparator, R: Read + Seek> SSIterator for TableIterator<'a, R, C> {
    // A call to valid() after seeking is necessary to ensure that the seek worked (e.g., no error
    // while reading from disk)
    fn seek(&mut self, to: &[u8]) {
        // first seek in index block, then set current_block and seek there

        self.index_block.seek(to);

        if let Some((k, _)) = self.index_block.current() {
            if self.table.cmp.cmp(to, &k) <= Ordering::Equal {
                // ok, found right block: continue below
            } else {
                self.reset();
            }
        } else {
            panic!("Unexpected None from current() (bug)");
        }

        // Read block and seek to entry in that block
        if let Some((k, handle)) = self.index_block.current() {
            assert!(self.table.cmp.cmp(to, &k) <= Ordering::Equal);

            if let Ok(()) = self.load_block(&handle) {
                self.current_block.seek(to);
                self.init = true;
            } else {
                self.reset();
            }
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

        while let Err(_) = self.skip_to_next_entry() {
        }
    }

    // This iterator is special in that it's valid even before the first call to next(). It behaves
    // correctly, though.
    fn valid(&self) -> bool {
        self.init && (self.current_block.valid() || self.index_block.valid())
    }

    fn current(&self) -> Option<Self::Item> {
        self.current_block.current()
    }
}

#[cfg(test)]
mod tests {
    use options::{BuildOptions, ReadOptions};
    use table_builder::TableBuilder;
    use iterator::{StandardComparator, SSIterator};

    use std::io::Cursor;

    use super::*;

    fn build_data() -> Vec<(&'static str, &'static str)> {
        vec![("abc", "def"),
             ("abd", "dee"),
             ("bcd", "asa"),
             ("bsr", "a00"),
             ("xyz", "xxx"),
             ("xzz", "yyy"),
             ("zzz", "111")]
    }


    fn build_table() -> (Vec<u8>, usize) {
        let mut d = Vec::with_capacity(512);
        let mut opt = BuildOptions::default();
        opt.block_restart_interval = 2;
        opt.block_size = 32;

        {
            let mut b = TableBuilder::new(&mut d, opt, StandardComparator);
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
    fn test_table_iterator_fwd() {
        let (src, size) = build_table();
        let data = build_data();

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
            .unwrap();
        let iter = table.iter();
        let mut i = 0;

        for (k, v) in iter {
            assert_eq!((data[i].0.as_bytes(), data[i].1.as_bytes()),
                       (k.as_ref(), v.as_ref()));
            i += 1;
        }
    }

    #[test]
    fn test_table_data_corruption() {
        let (mut src, size) = build_table();

        // Mess with first block
        src[28] += 1;

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
            .unwrap();
        let mut iter = table.iter();

        // defective blocks are skipped, i.e. we should start with the second block

        assert!(iter.next().is_some());
        assert_eq!(iter.current(),
                   Some(("bsr".as_bytes().to_vec(), "a00".as_bytes().to_vec())));
        assert!(iter.next().is_some());
        assert_eq!(iter.current(),
                   Some(("xyz".as_bytes().to_vec(), "xxx".as_bytes().to_vec())));
        assert!(iter.prev().is_some());
        // corrupted blocks are skipped also when reading the other way round
        assert!(iter.prev().is_none());
    }

    #[test]
    fn test_table_data_corruption_regardless() {
        let mut opt = ReadOptions::default();
        opt.skip_bad_blocks = false;

        let (mut src, size) = build_table();

        // Mess with first block
        src[28] += 1;

        let mut table = Table::new(Cursor::new(&src as &[u8]), size, opt, StandardComparator)
            .unwrap();
        let mut iter = table.iter();

        // defective blocks are NOT skipped!

        assert!(iter.next().is_some());
        assert_eq!(iter.current(),
                   Some(("abc".as_bytes().to_vec(), "def".as_bytes().to_vec())));
        assert!(iter.next().is_some());
        assert_eq!(iter.current(),
                   Some(("abd".as_bytes().to_vec(), "dee".as_bytes().to_vec())));
    }

    #[test]
    fn test_table_get() {
        let (src, size) = build_table();

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
            .unwrap();

        assert_eq!(table.get("abc".as_bytes()), Some("def".as_bytes().to_vec()));
        assert_eq!(table.get("zzz".as_bytes()), Some("111".as_bytes().to_vec()));
        assert_eq!(table.get("xzz".as_bytes()), Some("yyy".as_bytes().to_vec()));
        assert_eq!(table.get("xzy".as_bytes()), None);
    }

    #[test]
    fn test_table_iterator_state_behavior() {
        let (src, size) = build_table();

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
            .unwrap();
        let mut iter = table.iter();

        // behavior test

        // See comment on valid()
        assert!(!iter.valid());
        assert!(iter.current().is_none());

        assert!(iter.next().is_some());
        assert!(iter.valid());
        assert!(iter.current().is_some());

        assert!(iter.next().is_some());
        assert!(iter.prev().is_some());
        assert!(iter.current().is_some());

        iter.reset();
        assert!(!iter.valid());
        assert!(iter.current().is_none());
    }

    #[test]
    fn test_table_iterator_values() {
        let (src, size) = build_table();
        let data = build_data();

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
            .unwrap();
        let mut iter = table.iter();
        let mut i = 0;

        iter.next();
        iter.next();

        // Go back to previous entry, check, go forward two entries, repeat
        // Verifies that prev/next works well.
        while iter.valid() && i < data.len() {
            iter.prev();

            if let Some((k, v)) = iter.current() {
                assert_eq!((data[i].0.as_bytes(), data[i].1.as_bytes()),
                           (k.as_ref(), v.as_ref()));
            } else {
                break;
            }

            i += 1;
            iter.next();
            iter.next();
        }

        assert_eq!(i, 7);
    }

    #[test]
    fn test_table_iterator_seek() {
        let (src, size) = build_table();

        let mut table = Table::new(Cursor::new(&src as &[u8]),
                                   size,
                                   ReadOptions::default(),
                                   StandardComparator)
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
}
