use criterion::{black_box, criterion_group, criterion_main, Criterion};
use the_sovereign_shadow::v3_math; 

fn bench_math_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("Math_Engine_Latency");

    // 🧪 Test 1: Primitive Tick Lookup (Sub-2ns target)
    group.bench_function("primitive_tick_lookup", |b| {
        b.iter(|| {
            v3_math::get_sqrt_ratio_at_tick(black_box(100000)) 
        })
    });

    // 🧪 Test 2: Multi-Hop Multiplier (Path Math)
    // Ek real arbitrage mein hum 3-4 baar multiplication aur rounding karte hain.
    group.bench_function("triple_hop_multiplier", |b| {
        let price = v3_math::get_sqrt_ratio_at_tick(100000);
        let factor = 0xfff97272373d413259a46990580e213au128;
        
        b.iter(|| {
            let p1 = v3_math::mul_shift_128(black_box(price), black_box(factor));
            let p2 = v3_math::mul_shift_128(p1, black_box(factor));
            let _p3 = v3_math::mul_shift_128(p2, black_box(factor));
            black_box(_p3);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_math_performance);
criterion_main!(benches);