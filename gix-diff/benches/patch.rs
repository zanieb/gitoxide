use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn generate_lines(count: usize, prefix: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(count * 30);
    for i in 0..count {
        buf.extend_from_slice(format!("{prefix} line {i}\n").as_bytes());
    }
    buf
}

fn patch_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("patch_write");

    for &num_lines in &[100, 1_000, 10_000] {
        let old = generate_lines(num_lines, "old");
        // Modify ~10% of lines
        let new_lines: Vec<String> = (0..num_lines)
            .map(|i| {
                if i % 10 == 5 {
                    format!("CHANGED line {i}\n")
                } else {
                    format!("old line {i}\n")
                }
            })
            .collect();
        let new: Vec<u8> = new_lines.join("").into_bytes();

        group.bench_with_input(BenchmarkId::new("modified_10pct", num_lines), &num_lines, |b, _| {
            b.iter(|| {
                let mut out = Vec::with_capacity(old.len());
                gix_diff::blob::patch::write(
                    &mut out,
                    Some("abc1234"),
                    Some("def5678"),
                    "file.txt",
                    "file.txt",
                    &old,
                    &new,
                    gix_diff::blob::patch::Options::default(),
                )
                .unwrap();
                out
            });
        });

        // Benchmark with completely different content (worst case)
        let new_different = generate_lines(num_lines, "new");
        group.bench_with_input(BenchmarkId::new("fully_changed", num_lines), &num_lines, |b, _| {
            b.iter(|| {
                let mut out = Vec::with_capacity(old.len() * 2);
                gix_diff::blob::patch::write(
                    &mut out,
                    Some("abc1234"),
                    Some("def5678"),
                    "file.txt",
                    "file.txt",
                    &old,
                    &new_different,
                    gix_diff::blob::patch::Options::default(),
                )
                .unwrap();
                out
            });
        });
    }

    group.finish();
}

criterion_group!(benches, patch_write);
criterion_main!(benches);
