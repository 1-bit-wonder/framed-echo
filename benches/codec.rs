//! Criterion benchmarks for encode/decode throughput at a few frame sizes.
//!
//! Run with `cargo bench`. Numbers are hardware-specific — treat any figure in
//! the README as "measured on that machine", not a portable claim.

use bytes::{Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use framed_echo::FrameCodec;
use tokio_util::codec::{Decoder, Encoder};

const SIZES: [usize; 3] = [64, 1024, 64 * 1024];

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");
    for &size in &SIZES {
        let payload = Bytes::from(vec![0x5Au8; size]);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            let mut codec = FrameCodec::new();
            b.iter(|| {
                let mut dst = BytesMut::with_capacity(payload.len() + 4);
                codec.encode(payload.clone(), &mut dst).unwrap();
                dst
            });
        });
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    for &size in &SIZES {
        let payload = Bytes::from(vec![0x5Au8; size]);
        let mut encoded = BytesMut::new();
        FrameCodec::new().encode(payload, &mut encoded).unwrap();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &encoded, |b, encoded| {
            let mut codec = FrameCodec::new();
            b.iter(|| {
                // Clone the pre-encoded bytes so each iteration decodes a fresh
                // buffer; the clone is a refcount bump, not a payload copy.
                let mut src = encoded.clone();
                codec.decode(&mut src).unwrap().unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_encode, bench_decode);
criterion_main!(benches);
