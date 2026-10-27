//! Instructions-per-element benchmark for the numeric-policy kernels,
//! meant to run under callgrind (one kernel per invocation, `none` is
//! the loop-overhead baseline):
//!
//! ```sh
//! cargo build --release --example policy_bench
//! V=~/.cache/rr-tools/bin/valgrind
//! E=/home/m0hawk/.cache/cargo-target/release/examples/policy_bench
//! for k in none sum cumsum sqdev cross add_each; do
//!   for p in exact fast; do
//!     $V -q --tool=callgrind --callgrind-out-file=/tmp/cg-$k-$p.out $E $k $p
//!   done
//! done
//! ```
//!
//! The checksum print keeps the work from being optimized away; the
//! `scripts/ldouble-callgrind.sh` driver prints the per-element table.

use robigo_numerics::policy::{self, NumericPolicy};

fn main() {
    let kernel = std::env::args().nth(1).unwrap_or_else(|| "sum".into());
    let policy = match std::env::args().nth(2).as_deref() {
        Some("fast") => NumericPolicy::Fast,
        _ => NumericPolicy::Exact,
    };
    const N: usize = 8192;
    const ITERS: usize = 300;

    // deterministic data with a mix of magnitudes
    let mut xs = vec![0.0f64; N];
    let mut s = 0x9E3779B97F4A7C15u64;
    for x in xs.iter_mut() {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        let v = (s as f64) / u64::MAX as f64;
        *x = v * 10f64.powi(((s % 14) as i32) - 7);
    }
    let mut out = vec![0.0f64; N];
    let mut acc = vec![0.0f64; 16];
    let mut checksum = 0.0f64;
    // raw software long-double ops (the off-x86_64 exact path): fold
    // add/sub/mul/div over the 80-bit values, rounded back at the end.
    #[cfg(target_arch = "x86_64")]
    let soft_ld: Vec<robigo_numerics::ldouble::LDouble> = {
        use robigo_numerics::ldouble::LDouble;
        xs.iter().map(|&x| LDouble::from_f64(x)).collect()
    };

    for _ in 0..ITERS {
        match kernel.as_str() {
            "none" => {
                for x in &xs {
                    checksum += (*x > 0.5) as u8 as f64;
                }
            }
            "sum" => checksum += policy::sum(policy, &xs),
            "cumsum" => {
                policy::cumsum(policy, 0.0, &xs, &mut out);
                checksum += out[N - 1];
            }
            "sqdev" => checksum += policy::sum_sq_dev(policy, 0.5, &xs),
            "cross" => checksum += policy::sum_cross_dev(policy, 0.5, &xs, 0.25, &xs),
            "add_each" => {
                policy::add_each(policy, &mut acc, &xs[..16]);
                checksum += acc[0] + acc[15];
            }
            #[cfg(target_arch = "x86_64")]
            "soft_add" | "soft_mul" | "soft_div" | "soft_sub" => {
                use robigo_numerics::ldouble::{soft, LDouble};
                let mut a = soft_ld[0];
                for k in 1..N {
                    a = match kernel.as_str() {
                        "soft_add" => soft::add(&a, &soft_ld[k]),
                        "soft_sub" => soft::sub(&a, &soft_ld[k]),
                        "soft_mul" => soft::mul(&a, &soft_ld[k]),
                        _ => soft::div(&a, &soft_ld[k]),
                    };
                }
                checksum += a.to_f64();
                let _ = LDouble::ZERO;
            }
            other => panic!("unknown kernel {other}"),
        }
    }
    println!("{kernel} {checksum}");
}
