use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn varint_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("varint");

    for &value in &[0u64, 127, 128, 16383, 16384, u32::MAX as u64, u64::MAX] {
        group.bench_with_input(BenchmarkId::new("write", value), &value, |b, &v| {
            b.iter(|| {
                let mut buf = Vec::with_capacity(10);
                gix_reftable::write_varint(v, &mut buf);
                buf
            });
        });

        let mut encoded = Vec::new();
        gix_reftable::write_varint(value, &mut encoded);
        group.bench_with_input(BenchmarkId::new("read", value), &encoded, |b, data| {
            b.iter(|| gix_reftable::read_varint(data).unwrap());
        });
    }

    group.finish();
}

fn header_footer_parse(c: &mut Criterion) {
    let header = gix_reftable::serialize_header(&gix_reftable::Header {
        version: gix_reftable::Version::V2,
        block_size: 4096,
        min_update_index: 1,
        max_update_index: 100,
        object_hash: gix_hash::Kind::Sha1,
    });

    c.bench_function("parse_header", |b| {
        b.iter(|| gix_reftable::parse_header(&header).unwrap());
    });

    c.bench_function("serialize_header", |b| {
        b.iter(|| {
            gix_reftable::serialize_header(&gix_reftable::Header {
                version: gix_reftable::Version::V2,
                block_size: 4096,
                min_update_index: 1,
                max_update_index: 100,
                object_hash: gix_hash::Kind::Sha1,
            })
        });
    });
}

criterion_group!(benches, varint_roundtrip, header_footer_parse);
criterion_main!(benches);
