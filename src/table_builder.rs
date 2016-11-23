use block::{BlockBuilder, BlockContents};
use blockhandle::BlockHandle;
use options::{CompressionType, BuildOptions};
use iterator::{Comparator, StandardComparator};

use std::io::{Result, Write};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::cmp::Ordering;

use crc::crc32;
use crc::Hasher32;
use integer_encoding::FixedInt;

pub const FOOTER_LENGTH: usize = 40;
pub const FULL_FOOTER_LENGTH: usize = FOOTER_LENGTH + 8;
pub const MAGIC_FOOTER_NUMBER: u64 = 0xdb4775248b80fb57;
pub const MAGIC_FOOTER_ENCODED: [u8; 8] = [0x57, 0xfb, 0x80, 0x8b, 0x24, 0x75, 0x47, 0xdb];

fn find_shortest_sep<C: Comparator>(c: &C, lo: &[u8], hi: &[u8]) -> Vec<u8> {
    let min;

    if lo.len() < hi.len() {
        min = lo.len();
    } else {
        min = hi.len();
    }

    let mut diff_at = 0;

    while diff_at < min && lo[diff_at] == hi[diff_at] {
        diff_at += 1;
    }

    if diff_at == min {
        return Vec::from(lo);
    } else {
        if lo[diff_at] < 0xff && lo[diff_at] + 1 < hi[diff_at] {
            let mut result = Vec::from(&lo[0..diff_at + 1]);
            result[diff_at] += 1;
            assert_eq!(c.cmp(&result, hi), Ordering::Less);
            return result;
        }
        return Vec::from(lo);
    }
}

/// Footer is a helper for encoding/decoding a table footer.
#[derive(Debug)]
pub struct Footer {
    pub index: BlockHandle,
}

impl Footer {
    pub fn new(index: BlockHandle) -> Footer {
        Footer { index: index }
    }

    pub fn decode(from: &[u8]) -> Footer {
        assert!(from.len() >= FULL_FOOTER_LENGTH);
        assert_eq!(&from[FOOTER_LENGTH..], &MAGIC_FOOTER_ENCODED);
        let (ix, _) = BlockHandle::decode(&from[0..]);

        Footer { index: ix }
    }

    pub fn encode(&self, to: &mut [u8]) {
        assert!(to.len() >= FOOTER_LENGTH + 8);

        let s1 = self.index.encode_to(&mut to[0..]);

        for i in s1..FOOTER_LENGTH {
            to[i] = 0;
        }
        for i in FOOTER_LENGTH..FULL_FOOTER_LENGTH {
            to[i] = MAGIC_FOOTER_ENCODED[i - FOOTER_LENGTH];
        }
    }
}

/// A table consists of DATA BLOCKs, an INDEX BLOCK and a FOOTER.
///
/// DATA BLOCKs, INDEX BLOCKs, and BLOCKs are built using the code in the `block` module.
///
/// DATA BLOCKs contain the actual data; INDEX BLOCKS contain one entry per block, where the key is
/// a string after the last key of a block, and the value is a encoded BlockHandle pointing to that
/// block.
///
/// The footer is a pointer pointing to the index block, padding to fill up to 40 B and at the end
/// the 8B magic number 0xdb4775248b80fb57.
///
pub struct TableBuilder<C: Comparator, Dst: Write> {
    o: BuildOptions,
    cmp: C,
    dst: Dst,

    offset: usize,
    num_entries: usize,
    prev_block_last_key: Vec<u8>,

    data_block: Option<BlockBuilder<C>>,
    index_block: Option<BlockBuilder<C>>,
}

impl<Dst: Write> TableBuilder<StandardComparator, Dst> {
    /// Create a new TableBuilder with default comparator and BuildOptions.
    pub fn new_defaults(dst: Dst) -> TableBuilder<StandardComparator, Dst> {
        TableBuilder::new(dst, BuildOptions::default(), StandardComparator)
    }
}

impl TableBuilder<StandardComparator, File> {
    /// Open/create a file for writing a table.
    /// This will truncate the file, if it exists.
    pub fn new_to_file(file: &Path) -> Result<TableBuilder<StandardComparator, File>> {
        let f = try!(OpenOptions::new().create(true).write(true).truncate(true).open(file));
        Ok(TableBuilder::new(f, BuildOptions::default(), StandardComparator))
    }
}

impl<C: Comparator, Dst: Write> TableBuilder<C, Dst> {
    /// Create a new TableBuilder.
    pub fn new(dst: Dst, opt: BuildOptions, cmp: C) -> TableBuilder<C, Dst> {
        TableBuilder {
            o: opt,
            cmp: cmp,
            dst: dst,
            offset: 0,
            prev_block_last_key: vec![],
            num_entries: 0,
            data_block: Some(BlockBuilder::new(opt, cmp)),
            index_block: Some(BlockBuilder::new(opt, cmp)),
        }
    }

    /// Returns how many entries have been written.
    pub fn entries(&self) -> usize {
        self.num_entries
    }

    /// Add an entry to this table. The key must be lexicographically greater than the last entry
    /// written.
    pub fn add(&mut self, key: &[u8], val: &[u8]) {
        assert!(self.data_block.is_some());
        assert!(self.num_entries == 0 ||
                self.cmp.cmp(&self.prev_block_last_key, key) == Ordering::Less);

        if self.data_block.as_ref().unwrap().size_estimate() > self.o.block_size {
            self.write_data_block(key);
        }

        let dblock = &mut self.data_block.as_mut().unwrap();

        self.num_entries += 1;
        dblock.add(key, val);
    }

    /// Writes an index entry for the current data_block where `next_key` is the first key of the
    /// next block.
    fn write_data_block(&mut self, next_key: &[u8]) {
        assert!(self.data_block.is_some());

        let block = self.data_block.take().unwrap();
        let sep = find_shortest_sep(&self.cmp, block.last_key(), next_key);
        self.prev_block_last_key = Vec::from(block.last_key());
        let contents = block.finish();

        let handle = BlockHandle::new(self.offset, contents.len());
        let mut handle_enc = [0 as u8; 16];
        let enc_len = handle.encode_to(&mut handle_enc);

        self.index_block.as_mut().unwrap().add(&sep, &handle_enc[0..enc_len]);
        self.data_block = Some(BlockBuilder::new(self.o, self.cmp));

        let ctype = self.o.compression_type;
        self.write_block(contents, ctype);
    }

    /// Writes a block to disk, with a trailing 4 byte CRC checksum.
    fn write_block(&mut self, c: BlockContents, t: CompressionType) -> BlockHandle {
        // compression is still unimplemented
        assert_eq!(t, CompressionType::CompressionNone);

        let mut buf = [0 as u8; 4];
        let mut digest = crc32::Digest::new(crc32::CASTAGNOLI);

        digest.write(&c);
        digest.write(&[self.o.compression_type as u8; 1]);
        digest.sum32().encode_fixed(&mut buf);

        // TODO: Handle errors here.
        let _ = self.dst.write(&c);
        let _ = self.dst.write(&[t as u8; 1]);
        let _ = self.dst.write(&buf);

        let handle = BlockHandle::new(self.offset, c.len());

        self.offset += c.len() + 1 + buf.len();

        handle
    }

    /// Finish building this table. This *must* be called at the end, otherwise not all data may
    /// land on disk.
    pub fn finish(mut self) {
        assert!(self.data_block.is_some());
        let ctype = self.o.compression_type;

        // If there's a pending data block, write that one
        let flush_last_block = self.data_block.as_ref().unwrap().entries() > 0;
        if flush_last_block {
            self.write_data_block(&[0xff as u8; 1]);
        }

        // write index block
        let index_cont = self.index_block.take().unwrap().finish();
        let ix_handle = self.write_block(index_cont, ctype);

        // write footer.
        let footer = Footer::new(ix_handle);
        let mut buf = [0; FULL_FOOTER_LENGTH];
        footer.encode(&mut buf);

        self.offset += self.dst.write(&buf[..]).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::{find_shortest_sep, Footer, TableBuilder};
    use iterator::StandardComparator;
    use blockhandle::BlockHandle;
    use options::BuildOptions;

    #[test]
    fn test_shortest_sep() {
        assert_eq!(find_shortest_sep(&StandardComparator, "abcd".as_bytes(), "abcf".as_bytes()),
                   "abce".as_bytes());
        assert_eq!(find_shortest_sep(&StandardComparator,
                                     "abcdefghi".as_bytes(),
                                     "abcffghi".as_bytes()),
                   "abce".as_bytes());
        assert_eq!(find_shortest_sep(&StandardComparator, "a".as_bytes(), "a".as_bytes()),
                   "a".as_bytes());
        assert_eq!(find_shortest_sep(&StandardComparator, "a".as_bytes(), "b".as_bytes()),
                   "a".as_bytes());
        assert_eq!(find_shortest_sep(&StandardComparator, "abc".as_bytes(), "zzz".as_bytes()),
                   "b".as_bytes());
        assert_eq!(find_shortest_sep(&StandardComparator, "".as_bytes(), "".as_bytes()),
                   "".as_bytes());
    }

    #[test]
    fn test_footer() {
        let f = Footer::new(BlockHandle::new(55, 5));
        let mut buf = [0; 48];
        f.encode(&mut buf[..]);

        let f2 = Footer::decode(&buf);
        assert_eq!(f2.index.offset(), 55);
        assert_eq!(f2.index.size(), 5);

    }

    #[test]
    fn test_table_builder() {
        let mut d = Vec::with_capacity(512);
        let mut opt = BuildOptions::default();
        opt.block_restart_interval = 3;
        let mut b = TableBuilder::new(&mut d, opt, StandardComparator);

        let data = vec![("abc", "def"), ("abd", "dee"), ("bcd", "asa"), ("bsr", "a00")];

        for &(k, v) in data.iter() {
            b.add(k.as_bytes(), v.as_bytes());
        }

        b.finish();
    }

    #[test]
    #[should_panic]
    fn test_bad_input() {
        let mut d = Vec::with_capacity(512);
        let mut opt = BuildOptions::default();
        opt.block_restart_interval = 3;
        let mut b = TableBuilder::new(&mut d, opt, StandardComparator);

        // Test two equal consecutive keys
        let data = vec![("abc", "def"), ("abc", "dee"), ("bcd", "asa"), ("bsr", "a00")];

        for &(k, v) in data.iter() {
            b.add(k.as_bytes(), v.as_bytes());
        }
    }
}
