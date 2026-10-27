use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    fn value(&mut self) -> f64 {
        let exponent = (self.next() % 81) as i32 - 40;
        (self.unit() * 2.0 - 1.0) * 2.0f64.powi(exponent)
    }
}
fn gamma(k: usize) -> f64 {
    let ku = k as f64 * (f64::EPSILON / 2.0);
    ku / (1.0 - ku)
}
fn close(op: &str, exact: f64, fast: f64, bound: f64) {
    assert!(exact.is_finite() && fast.is_finite());
    assert!((exact - fast).abs() <= bound, "{op}: exact={exact:e}, fast={fast:e}, bound={bound:e}");
}
fn bits(a: f64, b: f64) {
    assert_eq!(a.to_bits(), b.to_bits(), "{a:?} != {b:?}");
}

#[test]
fn generated_fast_results_obey_operation_bounds() {
    let mut rng = Rng(0xb121_4caf_f6d2_09a7);
    for case in 0..4096 {
        let n = (rng.next() % 258) as usize;
        let mut xs: Vec<f64> = (0..n).map(|_| rng.value()).collect();
        let ys: Vec<f64> = (0..n).map(|_| rng.value()).collect();
        // Include cancellation and nearly constant inputs, beyond the random mix.
        if case % 3 == 0 {
            for x in &mut xs {
                *x += 1e12;
            }
        }
        if case % 3 == 1 {
            for i in 1..n {
                if i % 2 == 1 {
                    xs[i] = -xs[i - 1];
                }
            }
        }
        let (m, ym, init) = (rng.value(), rng.value(), rng.value());
        let ld = LDouble::from_f64;
        let scale = init.abs() + xs.iter().map(|x| x.abs()).sum::<f64>();
        close("sum", ld(init).sum_from(&xs).to_f64(), init.sum_from(&xs), gamma(2 * n + 8) * scale);
        let dev_scale = xs.iter().map(|x| x.abs() + m.abs()).sum::<f64>();
        close("dev", ld(m).sum_dev(&xs).to_f64(), m.sum_dev(&xs), gamma(3 * n + 8) * dev_scale);
        let sq_scale = xs.iter().map(|x| (x.abs() + m.abs()).powi(2)).sum::<f64>();
        close("sqdev", ld(m).sum_sq_dev(&xs).to_f64(), m.sum_sq_dev(&xs), gamma(4 * n + 8) * sq_scale);
        let cross_scale = xs.iter().zip(&ys).map(|(x, y)| (x.abs() + m.abs()) * (y.abs() + ym.abs())).sum::<f64>();
        close(
            "cross",
            ld(m).sum_cross_dev(&xs, ld(ym), &ys).to_f64(),
            m.sum_cross_dev(&xs, ym, &ys),
            gamma(5 * n + 8) * cross_scale,
        );
        if n != 0 {
            let scale = xs.iter().map(|x| x.abs()).sum::<f64>() / n as f64;
            close("mean", mean(NumericPolicy::Exact, &xs), mean(NumericPolicy::Fast, &xs), gamma(8 * n + 32) * scale);
        }
        let mut e = vec![0.0; n + 2];
        let mut f = e.clone();
        let es = ld(init).cumsum_from(&xs, &mut e);
        let fs = init.cumsum_from(&xs, &mut f);
        let mut scale = init.abs();
        for j in 0..n {
            scale += xs[j].abs();
            close("cumsum", e[j], f[j], gamma(2 * (j + 1) + 8) * scale);
        }
        assert_eq!(&e[n..], &[0.0, 0.0]);
        assert_eq!(&f[n..], &[0.0, 0.0]);
        close("cumsum state", es.to_f64(), fs, gamma(2 * n + 8) * scale);

        let factors: Vec<f64> = (0..n)
            .map(|_| {
                let v = 1.0 + (rng.unit() - 0.5) * 0.01;
                if rng.next() & 1 == 0 { v } else { -v }
            })
            .collect();
        let initial = 1.0 + rng.unit();
        let ep = ld(initial).prod_from(&factors).to_f64();
        close("prod", ep, initial.prod_from(&factors), gamma(2 * n + 8) * ep.abs());
        let ep = ld(initial).cumprod_from(&factors, &mut e).to_f64();
        let fp = initial.cumprod_from(&factors, &mut f);
        for j in 0..n {
            close("cumprod", e[j], f[j], gamma(2 * (j + 1) + 8) * e[j].abs());
        }
        close("cumprod state", ep, fp, gamma(2 * n + 8) * ep.abs());

        let mut ea = vec![ld(init); n + 2];
        let mut fa = vec![init; n + 2];
        for col in [&xs, &ys, &xs, &ys] {
            <LDouble as Accumulator>::add_each(&mut ea, col);
            <f64 as Accumulator>::add_each(&mut fa, col);
        }
        for j in 0..n {
            close("add_each", ea[j].to_f64(), fa[j], gamma(16) * (init.abs() + 2.0 * (xs[j].abs() + ys[j].abs())));
        }
        bits(ea[n].to_f64(), init);
        bits(fa[n], init);
    }
}

#[test]
fn exact_preserves_state_across_chunks_and_columns() {
    fn chunks<A: Accumulator>() -> f64 {
        A::ZERO.sum_from(&[1e16, 1.0]).sum_from(&[-1e16]).to_f64()
    }
    bits(chunks::<LDouble>(), 1.0);
    assert_eq!(chunks::<f64>(), 0.0);
    let mut a = [LDouble::ZERO];
    for x in [1e16, 1.0, -1e16] {
        <LDouble as Accumulator>::add_each(&mut a, &[x]);
    }
    bits(a[0].to_f64(), 1.0);
    // Products must retain extended exponent range between calls too.
    let p = <LDouble as Accumulator>::prod_from(LDouble::ONE, &[1e300, 1e300]);
    bits(p.prod_from(&[1e-300, 1e-300]).to_f64(), 1.0000000000000002);
}

#[test]
fn exact_dispatch_matches_existing_kernels_on_edges() {
    let values = [
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::from_bits(0x7ff00000000007a2),
    ];
    for &a in &values {
        for &b in &values {
            let xs = [a, b, 0.25];
            bits(sum(NumericPolicy::Exact, &xs), LDouble::ZERO.sum_from(&xs).to_f64());
            bits(prod(NumericPolicy::Exact, &xs), LDouble::ONE.prod_from(&xs).to_f64());
            let m = LDouble::from_f64(a);
            bits(<LDouble as Accumulator>::sum_dev(m, &xs).to_f64(), m.sum_dev(&xs).to_f64());
            bits(<LDouble as Accumulator>::sum_sq_dev(m, &xs).to_f64(), m.sum_sq_dev(&xs).to_f64());
            bits(
                <LDouble as Accumulator>::sum_cross_dev(m, &xs, m, &xs).to_f64(),
                m.sum_cross_dev(&xs, m, &xs).to_f64(),
            );
            let mut e = [0.0; 3];
            let mut f = e;
            let es = m.cumsum_from(&xs, &mut e);
            let fs = <LDouble as Accumulator>::cumsum_from(m, &xs, &mut f);
            bits(es.to_f64(), fs.to_f64());
            for j in 0..3 {
                bits(e[j], f[j]);
            }
            let es = m.cumprod_from(&xs, &mut e);
            let fs = <LDouble as Accumulator>::cumprod_from(m, &xs, &mut f);
            bits(es.to_f64(), fs.to_f64());
            for j in 0..3 {
                bits(e[j], f[j]);
            }
        }
    }
}

#[test]
fn empty_special_values_and_range_limits() {
    for p in [NumericPolicy::Exact, NumericPolicy::Fast] {
        bits(sum(p, &[]), 0.0);
        bits(prod(p, &[]), 1.0);
        assert!(mean(p, &[]).is_nan());
        assert!(sum(p, &[f64::NAN]).is_nan());
        assert!(prod(p, &[0.0, f64::INFINITY]).is_nan());
        bits(mean(p, &[f64::MAX, f64::MAX]), f64::MAX);
        bits(mean(p, &[1e308, -1e308]), 0.0);
    }
    // Fast mode explicitly has binary64's narrower exponent range.
    let xs = [1e300, 1e300, 1e-300, 1e-300];
    assert!(prod(NumericPolicy::Exact, &xs).is_finite());
    assert!(prod(NumericPolicy::Fast, &xs).is_nan());
    // Bounds are condition-dependent: severe cancellation can lose all bits.
    bits(sum(NumericPolicy::Exact, &[1e16, 1.0, -1e16]), 1.0);
    bits(sum(NumericPolicy::Fast, &[1e16, 1.0, -1e16]), 0.0);
}

#[test]
fn length_contracts_are_checked_for_both_policies() {
    fn check<A: Accumulator>() {
        assert!(std::panic::catch_unwind(|| A::ZERO.sum_cross_dev(&[1.0], A::ZERO, &[])).is_err());
        assert!(std::panic::catch_unwind(|| A::ZERO.cumsum_from(&[1.0], &mut [])).is_err());
        assert!(std::panic::catch_unwind(|| A::ONE.cumprod_from(&[1.0], &mut [])).is_err());
        assert!(std::panic::catch_unwind(|| A::add_each(&mut [], &[1.0])).is_err());
    }
    check::<LDouble>();
    check::<f64>();
}
