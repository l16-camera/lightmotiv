use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_tenbit(c: &mut Criterion) {
	let count = 4160 * 3120;
	let required = count * 10 / 8;
	let packd: Vec<u8> = (0..required).map(|i| (i % 251) as u8).collect();
	let mut out = vec![0u16; count];

	c.bench_function("tenbit_4160x3120", |b| {
		b.iter(|| {
			lri_rs::unpack::tenbit(black_box(&packd), count, black_box(&mut out)).expect("unpack");
		});
	});
}

criterion_group!(benches, bench_tenbit);
criterion_main!(benches);
