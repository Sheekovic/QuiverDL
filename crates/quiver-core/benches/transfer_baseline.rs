use std::{
    hint::black_box,
    io::{Read, Write},
    time::Instant,
};

use sha2::{Digest, Sha256};

const MEBIBYTE: usize = 1024 * 1024;
const DATA_SIZE: usize = 64 * MEBIBYTE;

fn main() {
    let data = vec![0x5a_u8; DATA_SIZE];

    let started = Instant::now();
    for _ in 0..4 {
        black_box(Sha256::digest(black_box(&data)));
    }
    report("SHA-256 CPU throughput", DATA_SIZE * 4, started);

    let directory = tempfile::tempdir().expect("temporary benchmark directory");
    let path = directory.path().join("transfer.bin");
    let started = Instant::now();
    let mut file = std::fs::File::create(&path).expect("create benchmark file");
    file.write_all(&data).expect("write benchmark data");
    file.sync_all().expect("flush benchmark data");
    report("durable disk write", DATA_SIZE, started);

    let started = Instant::now();
    let mut file = std::fs::File::open(path).expect("open benchmark file");
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer).expect("read benchmark data");
        if count == 0 {
            break;
        }
        bytes += count;
        black_box(&buffer[..count]);
    }
    report("sequential disk read", bytes, started);

    println!(
        "peak benchmark buffer: {} MiB data + 1 MiB read buffer",
        DATA_SIZE / MEBIBYTE
    );
}

fn report(label: &str, bytes: usize, started: Instant) {
    let elapsed = started.elapsed();
    let throughput = bytes as f64 / MEBIBYTE as f64 / elapsed.as_secs_f64();
    println!("{label}: {throughput:.1} MiB/s ({elapsed:.3?})");
}
