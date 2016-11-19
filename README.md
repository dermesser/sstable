# sstable

[Documentation](https://docs.rs/sstable)

This crate provides an API to work with immutable (string -> string) maps stored
on disk. The main access method are iterators, but there's a simpler API, too.

The general process is

* Writing a table to disk, using `TableBuilder`. The entries have to be added in
  sorted order.
* Reading a table from disk, using `TableReader`.

Note that the tables and some other structures are generic over the ordering of
keys; usually you can just use `StandardComparator`, though.

With `Options`, you can influence some details of how tables are laid out on
disk. Usually, you don't need to; just use the `Options::default()` value.

If there's data corruption in the files on disk, defective blocks will be
skipped. How many entries a single block contains depends on the block size,
which can be set in the `Options` struct.
