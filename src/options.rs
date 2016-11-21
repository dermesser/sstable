use std::default::Default;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CompressionType {
    CompressionNone = 0,
    CompressionSnappy = 1,
}

/// [not all member types implemented yet]
///
#[derive(Clone, Copy)]
pub struct BuildOptions {
    pub block_size: usize,
    pub block_restart_interval: usize,
    // Note: Compression is not implemented.
    pub compression_type: CompressionType,
}

impl Default for BuildOptions {
    fn default() -> BuildOptions {
        BuildOptions {
            block_size: 4 * (1 << 10),
            block_restart_interval: 16,
            compression_type: CompressionType::CompressionNone,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ReadOptions {
    pub skip_bad_blocks: bool,
}

impl Default for ReadOptions {
    fn default() -> ReadOptions {
        ReadOptions { skip_bad_blocks: true }
    }
}
