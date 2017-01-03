use options::{CompressionType, int_to_compressiontype};
use types::{ValueType, SequenceNumber};

use integer_encoding::{FixedInt, VarInt};

// The following typedefs are used to distinguish between the different key formats used internally
// by different modules.

// TODO: At some point, convert those into actual types with conversions between them. That's a lot
// of boilerplate, but increases type safety.

/// A UserKey is the actual key supplied by the calling application, without any internal
/// decorations.
pub type UserKey<'a> = &'a [u8];

/// An InternalKey consists of [key, tag], and is used as item type for Table iterators.
pub type InternalKey<'a> = &'a [u8];

/// A LookupKey is the first part of a memtable key, consisting of [keylen: varint32, key: *u8,
/// tag: u64]
/// keylen is the length of key plus 8 (for the tag; this for LevelDB compatibility)
#[derive(Clone, Debug)]
pub struct LookupKey {
    key: Vec<u8>,
    key_offset: usize,
}

impl LookupKey {
    #[allow(unused_assignments)]
    pub fn new(k: &[u8], s: SequenceNumber) -> LookupKey {
        let mut key = Vec::with_capacity(k.len() + k.len().required_space() +
                                         <u64 as FixedInt>::required_space());
        let internal_keylen = k.len() + 8;
        let mut i = 0;

        key.reserve(internal_keylen.required_space() + internal_keylen);

        key.resize(internal_keylen.required_space(), 0);
        i += internal_keylen.encode_var(&mut key[i..]);

        key.extend_from_slice(k);
        i += k.len();

        key.resize(i + <u64 as FixedInt>::required_space(), 0);
        (s << 8 | ValueType::TypeValue as u64).encode_fixed(&mut key[i..]);
        i += <u64 as FixedInt>::required_space();

        LookupKey {
            key: key,
            key_offset: k.len().required_space(),
        }
    }

    // Returns only key
    #[allow(dead_code)]
    pub fn user_key<'a>(&'a self) -> UserKey<'a> {
        &self.key[self.key_offset..self.key.len() - 8]
    }

    // Returns key+tag
    pub fn internal_key<'a>(&'a self) -> InternalKey<'a> {
        &self.key[self.key_offset..]
    }
}

/// Parses a tag into (type, sequence number)
pub fn parse_tag(tag: u64) -> (ValueType, u64) {
    let seq = tag >> 8;
    let typ = tag & 0xff;

    match typ {
        0 => (ValueType::TypeDeletion, seq),
        1 => (ValueType::TypeValue, seq),
        _ => (ValueType::TypeValue, seq),
    }
}

/// Parse a key in InternalKey format.
pub fn parse_internal_key<'a>(ikey: InternalKey<'a>) -> (CompressionType, u64, UserKey<'a>) {
    assert!(ikey.len() >= 8);

    let (ctype, seq) = parse_tag(FixedInt::decode_fixed(&ikey[ikey.len() - 8..]));
    let ctype = int_to_compressiontype(ctype as u32).unwrap_or(CompressionType::CompressionNone);

    return (ctype, seq, &ikey[0..ikey.len() - 8]);
}

#[cfg(test)]
mod tests {}
