#[macro_use]
extern crate bencher;

use std::fs;

use bencher::Bencher;
use rand::random;

use sstable::{SSIterator, Table, TableBuilder};

fn random_string(n: usize) -> String {
    let mut v = vec![0; n];
    for c in v.iter_mut() {
        *c = random::<u8>() % 26 + 65;
    }
    String::from_utf8(v).unwrap()
}

fn write_tmp_table(entries: usize) {
    let mut v = vec![(String::new(), String::new()); entries];
    for i in 0..entries {
        v[i] = (random_string(16), random_string(16));
    }
    v.sort();

    let dst = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open("/tmp/.sstabletestfile")
        .unwrap();
    let opt = sstable::Options::default();
    let mut tb = TableBuilder::new(opt, dst);

    for (k, v) in v {
        tb.add(k.as_bytes(), v.as_bytes()).unwrap();
    }
    tb.finish().unwrap();
}

fn rm_tmp_table() {
    fs::remove_file("/tmp/.sstabletestfile").ok();
}

fn bench_write(b: &mut Bencher) {
    let n = 100000;
    b.iter(|| {
        write_tmp_table(n);
        rm_tmp_table();
    });
}

fn bench_read(b: &mut Bencher) {
    rm_tmp_table();
    write_tmp_table(100000);
    b.iter(|| {
        let tbr = Table::new_from_file(
            sstable::Options::default(),
            std::path::Path::new("/tmp/.sstabletestfile"),
        )
        .unwrap();
        let mut iter = tbr.iter();

        let mut count = 0;
        let mut entries = 0;
        while let Some((k, v)) = iter.next() {
            count += k.len() + v.len();
            entries += 1;
        }
        assert_eq!(entries, 100000);
        assert_eq!(count, 3200000);
    });
}

benchmark_group!(benches, bench_write, bench_read);
benchmark_main!(benches);
