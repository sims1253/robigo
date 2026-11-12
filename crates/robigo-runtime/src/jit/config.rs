//! The JIT's switches, read once from the environment.
//!
//! `ROBIGO_JIT=0` turns native code off (the VM runs everything). Every other
//! switch is a token of the comma-separated list `ROBIGO_JIT_OPT` (after
//! relax's `RELAX_OPT`):
//!
//! | token | effect |
//! |---|---|
//! | `-loops`, `-closures` | no native loops / no closures compiled whole |
//! | `-summary` | no closed forms of loops (`jit::summary`) |
//! | `-inline` | the VM never runs closure bodies inline, nor natively |
//! | `eager` | compile at the first opportunity: loops at their first entry, closures at their first call, without the cost model (small bodies, code that keeps leaving) |
//! | `sample=SEED:P` | only a deterministic share `P` of the loops and closures (by their [`site`] hash and `SEED`) get native code |
//! | `only=ID:ID...` | only these sites (the report's ids) get native code |
//!
//! The selection is of compilation roots: native loops and closures compiled
//! whole. A closure they run inline goes with its root, so that the ids of
//! the report's native sites select exactly the native code of a run.
//! | `verify` | Cranelift's IR verifier and register-allocation checker on every compilation; a failure panics |
//! | `report` | at exit, what became of each loop and closure (`jit::report`) |
//! | `trace` | every decision as it happens, on stderr |
//! | `dump` | the Cranelift IR and machine code of each compilation |
//!
//! `+name` turns an optimization back on. An unknown token is an error, so
//! that a misspelt switch never silently tests the default.
//!
//! None of these changes what an R program prints: results, warnings and
//! errors are the VM's in every combination (`scripts/fuzz-eval.py --tiers`
//! checks that).

use std::sync::OnceLock;

use crate::compile::Code;

/// Which loops and closures get native code.
#[derive(Clone, Debug, PartialEq)]
pub enum Select {
    All,
    /// A site is selected when its hash mixed with `seed` falls below
    /// `share` of the 64-bit range.
    Sample {
        seed: u64,
        share: f64,
    },
    Only(Vec<u64>),
}

#[derive(Clone, Debug)]
pub struct Config {
    /// `ROBIGO_JIT` is not `0`.
    pub on: bool,
    pub loops: bool,
    pub closures: bool,
    pub summary: bool,
    pub inline: bool,
    pub eager: bool,
    pub verify: bool,
    pub report: bool,
    pub trace: bool,
    pub dump: bool,
    pub select: Select,
}

impl Config {
    /// The configuration `jit` (`ROBIGO_JIT`) and `opt` (`ROBIGO_JIT_OPT`)
    /// describe.
    pub fn parse(jit: Option<&str>, opt: Option<&str>) -> Result<Config, String> {
        let on = jit != Some("0");
        let mut c = Config {
            on,
            loops: on,
            closures: on,
            summary: true,
            inline: true,
            eager: false,
            verify: false,
            report: false,
            trace: false,
            dump: false,
            select: Select::All,
        };
        for tok in opt.unwrap_or("").split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let (sign, name) = match tok.as_bytes()[0] {
                b'-' => (Some(false), &tok[1..]),
                b'+' => (Some(true), &tok[1..]),
                _ => (None, tok),
            };
            match (sign, name) {
                (Some(v), "loops") => c.loops = v && on,
                (Some(v), "closures") => c.closures = v && on,
                (Some(v), "summary") => c.summary = v,
                (Some(v), "inline") => c.inline = v,
                (None, "eager") => c.eager = true,
                (None, "verify") => c.verify = true,
                (None, "report") => c.report = true,
                (None, "trace") => c.trace = true,
                (None, "dump") => c.dump = true,
                (None, t) if t.starts_with("sample=") => {
                    let spec = &t["sample=".len()..];
                    let parsed =
                        spec.split_once(':').and_then(|(s, p)| Some((s.parse::<u64>().ok()?, p.parse::<f64>().ok()?)));
                    match parsed {
                        Some((seed, share)) if (0.0..=1.0).contains(&share) => {
                            c.select = Select::Sample { seed, share }
                        }
                        _ => return Err(format!("'{}': expected sample=SEED:SHARE, SHARE in [0, 1]", tok)),
                    }
                }
                (None, t) if t.starts_with("only=") => {
                    let ids: Result<Vec<u64>, _> = t["only=".len()..]
                        .split(':')
                        .filter(|s| !s.is_empty())
                        .map(|s| u64::from_str_radix(s, 16))
                        .collect();
                    match ids {
                        Ok(ids) => c.select = Select::Only(ids),
                        Err(_) => return Err(format!("'{}': expected only=ID:ID... (hexadecimal ids)", tok)),
                    }
                }
                _ => {
                    return Err(format!(
                        "unknown token '{}' (known: -loops -closures -summary -inline eager sample=SEED:SHARE \
                         only=ID:... verify report trace dump)",
                        tok
                    ));
                }
            }
        }
        Ok(c)
    }

    /// Something wants the sites' ids and labels: the report or a selection.
    #[inline]
    pub fn sites(&self) -> bool {
        self.report || self.select != Select::All
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// The process's configuration (`ROBIGO_JIT`, `ROBIGO_JIT_OPT`).
#[inline]
pub fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let jit = std::env::var("ROBIGO_JIT").ok();
        let opt = std::env::var("ROBIGO_JIT_OPT").ok();
        Config::parse(jit.as_deref(), opt.as_deref()).unwrap_or_else(|e| {
            eprintln!("robigo: ROBIGO_JIT_OPT: {}", e);
            std::process::exit(2);
        })
    })
}

/// FNV-1a, then a finalizer (SplitMix64's) so that nearby inputs spread.
fn hash(bytes: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix(h)
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A site's id: the hash of its bytecode (with its constants), the same in
/// every run of the same program by the same binary, whatever the switches.
/// `loop_at`: the loop's `LoopEnter` index in `code`; `None`: the closure
/// whose body `code` is. (Closures with equal bodies share an id.)
pub fn site(code: &Code, loop_at: Option<usize>) -> u64 {
    let mut s = format!("{:?}", code.ops);
    for k in &code.consts {
        s.push('\u{1}');
        s.push_str(&crate::deparse::deparse_one_line(k));
    }
    let h = hash(s.as_bytes(), 0);
    match loop_at {
        Some(pc) => mix(h ^ (pc as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        None => h,
    }
}

/// Whether the site `id` may get native code.
pub fn selected(id: u64) -> bool {
    match &config().select {
        Select::All => true,
        Select::Sample { seed, share } => *share >= 1.0 || (mix(id ^ mix(*seed)) as f64) < share * (u64::MAX as f64),
        Select::Only(ids) => ids.contains(&id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let c = Config::parse(None, Some("-summary, eager,verify,sample=7:0.25")).unwrap();
        assert!(c.on && c.loops && c.closures && !c.summary && c.inline && c.eager && c.verify && !c.report);
        assert_eq!(c.select, Select::Sample { seed: 7, share: 0.25 });
        let c = Config::parse(Some("0"), Some("+loops,report,only=ff:1a")).unwrap();
        assert!(!c.on && !c.loops && !c.closures && c.report);
        assert_eq!(c.select, Select::Only(vec![0xff, 0x1a]));
        assert!(Config::parse(None, Some("-closures,-inline")).is_ok_and(|c| c.loops && !c.closures && !c.inline));
        assert!(Config::parse(None, Some("eagre")).is_err());
        assert!(Config::parse(None, Some("sample=1:2")).is_err());
        assert!(Config::parse(None, Some("only=xyz")).is_err());
    }

    #[test]
    fn sample_share() {
        // About the requested share of ids is selected, deterministically.
        let pick = |seed: u64, share: f64| {
            (0..10_000u64).filter(|i| (mix(mix(*i) ^ mix(seed)) as f64) < share * (u64::MAX as f64)).count()
        };
        let n = pick(3, 0.5);
        assert!((4_500..5_500).contains(&n), "{n}");
        assert_eq!(n, pick(3, 0.5));
        assert_eq!(pick(3, 0.0), 0);
        assert_eq!(pick(3, 1.0), 10_000);
    }
}
