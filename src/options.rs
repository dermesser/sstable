use block::Block;
use cache::Cache;
use cmp::{Cmp, DefaultCmp};
use filter;
use types::{share, Shared};

use std::default::Default;
use std::rc::Rc;

const KB: usize = 1 << 10;
const MB: usize = KB * KB;

const BLOCK_MAX_SIZE: usize = 4 * KB;
const BLOCK_CACHE_CAPACITY: usize = 8 * MB;
const WRITE_BUFFER_SIZE: usize = 4 * MB;
const DEFAULT_BITS_PER_KEY: u32 = 10; // NOTE: This may need to be optimized.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CompressionType {
    CompressionNone = 0,
    CompressionSnappy = 1,
}

pub fn int_to_compressiontype(i: u32) -> Option<CompressionType> {
    match i {
        0 => Some(CompressionType::CompressionNone),
        1 => Some(CompressionType::CompressionSnappy),
        _ => None,
    }
}

/// Options contains general parameters for reading and writing SSTables. Most of the names are
/// self-explanatory; the defaults are defined in the `Default` implementation.
#[derive(Clone)]
pub struct Options {
    pub cmp: Rc<Box<Cmp>>,
    pub write_buffer_size: usize,
    pub block_cache: Shared<Cache<Block>>,
    pub block_size: usize,
    pub block_restart_interval: usize,
    pub compression_type: CompressionType,
    pub filter_policy: filter::BoxedFilterPolicy,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            cmp: Rc::new(Box::new(DefaultCmp)),
            write_buffer_size: WRITE_BUFFER_SIZE,
            // 2000 elements by default
            block_cache: share(Cache::new(BLOCK_CACHE_CAPACITY / BLOCK_MAX_SIZE)),
            block_size: BLOCK_MAX_SIZE,
            block_restart_interval: 16,
            compression_type: CompressionType::CompressionNone,
            filter_policy: Rc::new(Box::new(filter::BloomPolicy::new(DEFAULT_BITS_PER_KEY))),
        }
    }
}
