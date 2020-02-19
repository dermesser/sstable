# sstable

[![crates.io](https://img.shields.io/crates/v/sstable.svg)](https://crates.io/crates/sstable)
[![Travis
CI](https://api.travis-ci.org/dermesser/sstable.svg?branch=master)](https://api.travis-ci.org/dermesser/sstable)

[Documentation](https://docs.rs/sstable)

## What

This crate provides an API to work with immutable (string -> string) maps stored
on disk. The main access method are iterators, but there's a simpler API, too.

The general process is

* Writing a table, using `TableBuilder`. The entries have to be added in
  sorted order. The data doesn't have to be written to disk; any type
  implementing `Write` works.
* Reading a table, using `Table`. Again, the source is generic; any type
  implementing `Read + Seek` can be used.

Note that the tables and some other structures are generic over the ordering of
keys; usually you can just use `StandardComparator`, though.

With `Options`, you can influence some details of how tables are laid out on
disk. Usually, you don't need to; just use the `Options::default()` value.

If there's data corruption in the files on disk, defective blocks will be
skipped. How many entries a single block contains depends on the block size,
which can be set in the `Options` struct.

## Why

This crate reuses code originally written for the persistence part of
[rusty-leveldb](https://crates.io/crates/rusty-leveldb), a reimplementation of
Google's LevelDB in Rust. That's the reason for the code being a bit more
complicated than needed at some points.

## Performance

With no compression on a tmpfs volume running on an idle `Intel(R) Xeon(R) CPU
E5-1650 v2 @ 3.50GHz` processor, the benchmark shows that in tables of 10'000
entries of each 16 key bytes and 16 value bytes, this crate will

* read 5.3 million entries per second
* write 1.2 million entries per second

The performance for tables of different sizes may differ.

## Corruption and errors

Checksum verification failures often stem from either corruption (obviously)
or incompletely written or half-overwritten SSTable files.


## Contribute

Contributions are very welcome! Feel free to send pull requests.

