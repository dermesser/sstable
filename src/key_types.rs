// The following typedefs are used to distinguish between the different key formats used internally
// by different modules.

/// A UserKey is the actual key supplied by the calling application, without any internal
/// decorations.
pub type UserKey<'a> = &'a [u8];

/// An InternalKey consists of [key, tag], so it's basically a MemtableKey without the initial
/// length specification. This type is used as item type of MemtableIterator, and as the key
/// type of tables.
pub type InternalKey<'a> = &'a [u8];
