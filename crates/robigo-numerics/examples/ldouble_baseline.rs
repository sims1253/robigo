//! Direct exact-kernel baseline; kept independent of the policy dispatcher.
use robigo_numerics::ldouble::LDouble as A;
use std::{hint::black_box, time::Instant};
fn main() {
    let kernel = std::env::args().nth(1).unwrap_or_else(|| "sum".into());
    let iters: usize = std::env::args().nth(2).unwrap_or_else(|| "300".into()).parse().unwrap();
    let xs: Vec<f64> = (0..8192).map(|i| 1.0 + ((i * 17 % 101) as f64 - 50.0) * 1e-5).collect();
    let ys: Vec<f64> = xs.iter().rev().copied().collect();
    let mut out = vec![0.0; xs.len()];
    let mut acc = vec![A::ZERO; xs.len()];
    let now = Instant::now();
    for _ in 0..iters {
        let x = black_box(xs.as_slice());
        let y = black_box(ys.as_slice());
        let m = A::from_f64(0.5);
        let result = match kernel.as_str() {
            "none" => 0.0,
            "sum" => A::ZERO.sum_from(x).to_f64(),
            "prod" => A::ONE.prod_from(x).to_f64(),
            "mean" => {
                let n = A::from_i64(x.len() as i64);
                let s = A::ZERO.sum_from(x) / n;
                (s + s.sum_dev(x) / n).to_f64()
            }
            "cumsum" => A::ZERO.cumsum_from(x, black_box(&mut out)).to_f64(),
            "cumprod" => A::ONE.cumprod_from(x, black_box(&mut out)).to_f64(),
            "dev" => m.sum_dev(x).to_f64(),
            "sqdev" => m.sum_sq_dev(x).to_f64(),
            "cross" => m.sum_cross_dev(x, m, y).to_f64(),
            "add_each" => {
                A::add_each(black_box(&mut acc), x);
                black_box(acc[0]).to_f64()
            }
            _ => panic!("unknown kernel {kernel}"),
        };
        black_box(result);
    }
    println!("{kernel} {} ns/element", now.elapsed().as_nanos() as f64 / (xs.len() * iters) as f64);
    black_box(out);
    black_box(acc);
}
