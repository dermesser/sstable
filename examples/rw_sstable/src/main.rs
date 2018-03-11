use std::fs;
use std::path::{Path, PathBuf};

extern crate sstable;
use sstable::Result;
use sstable::SSIterator;

fn make_table() -> Vec<(Vec<u8>, Vec<u8>)> {
    let data = &[
        ("abc", "111111"),
        ("def", "11222"),
        ("dfg", "0001"),
        ("zzz", "123456"),
    ];
    data.iter()
        .map(|&(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect()
}

fn write_table(p: &Path) -> Result<()> {
    let dst = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(p)?;
    let mut tb = sstable::TableBuilder::new(sstable::Options::default(), dst);

    for (ref k, ref v) in make_table() {
        tb.add(k, v)?;
    }
    tb.finish()?;
    Ok(())
}

fn read_table(p: &Path) -> Result<()> {
    let tr = sstable::Table::new_from_file(sstable::Options::default(), p)?;
    let mut iter = tr.iter();
    while iter.advance() {
        let (k, v) = sstable::current_key_val(&iter).unwrap();
        println!(
            "{} => {}",
            String::from_utf8(k).unwrap(),
            String::from_utf8(v).unwrap()
        );
    }
    Ok(())
}

fn main() {
    let path = PathBuf::from("/tmp/some.random.sstable");
    // Read a couple key/value pairs to a table in /tmp and read them back.
    write_table(&path).expect("writing the table failed");
    read_table(&path).expect("Reading the table failed");
}
