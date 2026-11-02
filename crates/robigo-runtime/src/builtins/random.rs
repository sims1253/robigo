//! Random number generation: GNU R's RNG.c (`.Random.seed`, `RNGkind`,
//! `set.seed`), random.c (`.Internal(rnorm(...))` & co, `sample`) and
//! `sample2` from unique.c. The generators themselves live in
//! `crate::nmath::rng` / `crate::nmath::rand`.

use std::collections::HashSet;

use super::internal;
use crate::coerce::{CoerceWarnings, as_integer, as_logical, as_real};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::nmath::rand as nr;
use crate::nmath::rng::{N01Kind, NeedTaocp1997Init, RngKind, SampleKind, time_to_seed};
use crate::sym::Sym;
use crate::value::*;

pub use crate::nmath::rng::Rng;

pub static DEFS: &[Builtin] = &[
    internal("RNGkind", do_rngkind, 3),
    internal("set.seed", do_setseed, 4).invisible(),
    internal("sample", do_sample, 4),
    internal("sample2", do_sample2, 2),
    internal("rchisq", do_rchisq, 2),
    internal("rexp", do_rexp, 2),
    internal("rgeom", do_rgeom, 2),
    internal("rpois", do_rpois, 2),
    internal("rt", do_rt, 2),
    internal("rsignrank", do_rsignrank, 2),
    internal("rbeta", do_rbeta, 3),
    internal("rbinom", do_rbinom, 3),
    internal("rcauchy", do_rcauchy, 3),
    internal("rf", do_rf, 3),
    internal("rgamma", do_rgamma, 3),
    internal("rlnorm", do_rlnorm, 3),
    internal("rlogis", do_rlogis, 3),
    internal("rnbinom", do_rnbinom, 3),
    internal("rnbinom_mu", do_rnbinom_mu, 3),
    internal("rnchisq", do_rnchisq, 3),
    internal("rnorm", do_rnorm, 3),
    internal("runif", do_runif, 3),
    internal("rweibull", do_rweibull, 3),
    internal("rwilcox", do_rwilcox, 3),
    internal("rhyper", do_rhyper, 4),
];

fn seeds_sym() -> Sym {
    Sym::new(".Random.seed")
}

// ----------------------------------------------------------------------
// GetRNGstate / PutRNGstate (RNG.c)

/// R: RNG.c:GetSeedsFromVar. `None` is R_UnboundValue.
fn get_seeds_from_var(i: &mut Interp) -> RResult<Option<Value>> {
    let g = i.global.clone();
    match g.get_local(seeds_sym()) {
        None => Ok(None),
        Some(Value::Promise(p)) => Ok(Some(i.force_promise(&p)?)),
        Some(v) => Ok(Some(v)),
    }
}

/// R: RNG.c:RNG_Init, running `.TAOCP1997init` for Knuth-TAOCP.
fn rng_init(i: &mut Interp, kind: RngKind, seed: u32) -> RResult<()> {
    if kind == RngKind::UserUnif {
        return Err(Flow::error("'user_unif_rand' not in load table"));
    }
    match i.rng.init(kind, seed) {
        Ok(()) => Ok(()),
        Err(NeedTaocp1997Init(s)) => {
            let base = i.base.clone();
            // R: RNG.c:RNG_Init — findVar1(..., CLOSXP) forces a promise
            let _ = &base;
            let fun = match i.base_binding(Sym::new(".TAOCP1997init")) {
                Some(f @ Value::Closure(_)) => f,
                _ => return Err(Flow::error("function '.TAOCP1997init' is missing")),
            };
            let call = Value::lang(fun.clone(), vec![Arg::pos(Value::int(s))]);
            let g = i.global.clone();
            let ans = i.eval(&call, &g)?;
            let words: Vec<i32> = match &ans {
                Value::Int(v) => v.data.clone(),
                other => crate::coerce::to_int(other, &mut CoerceWarnings::default()),
            };
            i.rng.set_taocp1997(&words);
            Ok(())
        }
    }
}

/// R: RNG.c:Randomize
fn randomize(i: &mut Interp, kind: RngKind) -> RResult<()> {
    rng_init(i, kind, time_to_seed())
}

/// R: RNG.c:FixupSeeds (re-randomizing all-zero seeds)
fn fixup_seeds(i: &mut Interp, kind: RngKind, initial: bool) -> RResult<()> {
    if i.rng.fixup_seeds(kind, initial) {
        randomize(i, kind)?;
    }
    Ok(())
}

/// R: RNG.c:GetRNGkind. Returns true when `.Random.seed` was absent or
/// invalid (in which case the generator was reset).
fn get_rng_kind(i: &mut Interp, seeds: Option<Value>) -> RResult<bool> {
    let seeds = match seeds {
        Some(s) => s,
        None => match get_seeds_from_var(i)? {
            Some(s) => s,
            None => return Ok(true),
        },
    };
    'invalid: {
        let is = match &seeds {
            Value::Int(v) if !seeds.inherits("factor") => v.data.clone(),
            other => {
                if other.is_missing_arg() {
                    return Err(Flow::error("argument \".Random.seed\" is missing, with no default"));
                }
                i.warning(format!(
                    "'.Random.seed' is not an integer vector but of type '{}', so ignored",
                    other.type_name()
                ))?;
                break 'invalid;
            }
        };
        let tmp = is.first().copied().unwrap_or(NA_INTEGER);
        if tmp == NA_INTEGER || !(0..=11000).contains(&tmp) {
            i.warning("'.Random.seed[1]' is not a valid integer, so ignored")?;
            break 'invalid;
        }
        let new_rng = tmp % 100;
        let new_n01 = tmp % 10000 / 100;
        let new_sample = tmp / 10000;
        if new_n01 > N01Kind::KindermanRamage as i32 || new_sample > SampleKind::Rejection as i32 {
            i.warning("'.Random.seed[1]' is not a valid Normal type, so ignored")?;
            break 'invalid;
        }
        let kind = match RngKind::from_code(new_rng) {
            Some(RngKind::UserUnif) => {
                i.warning("'.Random.seed[1] = 5' but no user-supplied generator, so ignored")?;
                break 'invalid;
            }
            Some(k) => k,
            None => {
                i.warning("'.Random.seed[1]' is not a valid RNG kind so ignored")?;
                break 'invalid;
            }
        };
        i.rng.kind = kind;
        i.rng.n01_kind = N01Kind::from_code(new_n01).unwrap();
        i.rng.sample_kind = if new_sample == 0 { SampleKind::Rounding } else { SampleKind::Rejection };
        return Ok(false);
    }
    // invalid:
    i.rng.kind = RngKind::MersenneTwister;
    i.rng.n01_kind = N01Kind::Inversion;
    i.rng.sample_kind = SampleKind::Rejection;
    let k = i.rng.kind;
    randomize(i, k)?;
    put_rng_state(i)?;
    Ok(true)
}

/// R: RNG.c:GetRNGstate
pub fn get_rng_state(i: &mut Interp) -> RResult<()> {
    match get_seeds_from_var(i)? {
        None => {
            let k = i.rng.kind;
            randomize(i, k)
        }
        Some(seeds) => {
            if get_rng_kind(i, Some(seeds.clone()))? {
                return Ok(());
            }
            let kind = i.rng.kind;
            let len_seed = kind.n_seed();
            let len = seeds.len();
            if len > 1 && len < len_seed + 1 {
                return Err(Flow::error("'.Random.seed' has wrong length"));
            }
            if len == 1 && kind != RngKind::UserUnif {
                randomize(i, kind)
            } else {
                if let Value::Int(v) = &seeds {
                    for (d, &s) in i.rng.seed.iter_mut().zip(v.data.iter().skip(1).take(len_seed)) {
                        *d = s as u32;
                    }
                }
                fixup_seeds(i, kind, false)
            }
        }
    }
}

/// R: RNG.c:PutRNGstate
pub fn put_rng_state(i: &mut Interp) -> RResult<()> {
    let kind = i.rng.kind;
    let len_seed = kind.n_seed();
    let kinds = kind as i32 + 100 * i.rng.n01_kind as i32 + 10000 * i.rng.sample_kind as i32;
    let mut data = Vec::with_capacity(len_seed + 1);
    data.push(kinds);
    data.extend(i.rng.seed[..len_seed].iter().map(|&s| s as i32));
    let g = i.global.clone();
    i.define(seeds_sym(), Value::ints(data), &g)
}

// ----------------------------------------------------------------------
// RNGkind / set.seed

/// R: RNG.c:RNGkind
fn rng_kind(i: &mut Interp, newkind: i32) -> RResult<()> {
    let newkind = if newkind == -1 { RngKind::MersenneTwister as i32 } else { newkind };
    let kind = match RngKind::from_code(newkind) {
        Some(k) => k,
        None => return Err(Flow::error(format!("RNGkind: unimplemented RNG kind {}", newkind))),
    };
    if kind == RngKind::MarsagliaMulticarry {
        i.warning("RNGkind: Marsaglia-Multicarry has poor statistical properties")?;
    }
    get_rng_state(i)?;
    let u = i.rng.unif_rand();
    if !(0.0..=1.0).contains(&u) {
        i.warning("someone corrupted the random-number generator: re-initializing")?;
        rng_init(i, kind, time_to_seed())?;
    } else {
        rng_init(i, kind, (u * u32::MAX as f64) as u32)?;
    }
    i.rng.kind = kind;
    put_rng_state(i)
}

/// R: RNG.c:Norm_kind
fn norm_kind(i: &mut Interp, kind: i32) -> RResult<()> {
    if kind == N01Kind::KindermanRamage as i32 && i.rng.kind == RngKind::MarsagliaMulticarry {
        i.warning("RNGkind: severe deviations from normality for Kinderman-Ramage + Marsaglia-Multicarry")?;
    }
    if kind == N01Kind::AhrensDieter as i32 && i.rng.kind == RngKind::MarsagliaMulticarry {
        i.warning("RNGkind: deviations from normality for Ahrens-Dieter + Marsaglia-Multicarry")?;
    }
    let kind = if kind == -1 { N01Kind::Inversion as i32 } else { kind };
    let k = match N01Kind::from_code(kind) {
        Some(k) => k,
        None => return Err(Flow::error("invalid Normal type in 'RNGkind'")),
    };
    if k == N01Kind::UserNorm {
        return Err(Flow::error("'user_norm_rand' not in load table"));
    }
    get_rng_state(i)?;
    if k == N01Kind::BoxMuller {
        i.rng.bm_norm_keep = 0.0;
    }
    i.rng.n01_kind = k;
    put_rng_state(i)
}

/// R: RNG.c:Samp_kind
fn samp_kind(i: &mut Interp, kind: i32) -> RResult<()> {
    let kind = if kind == -1 { SampleKind::Rejection as i32 } else { kind };
    let k = match kind {
        0 => SampleKind::Rounding,
        1 => SampleKind::Rejection,
        _ => return Err(Flow::error("invalid sample type in 'RNGkind'")),
    };
    get_rng_state(i)?;
    i.rng.sample_kind = k;
    put_rng_state(i)
}

/// R: errors.c:checkArityCall for `.Internal`s (reported like `error()`).
pub fn check_arity_internal(args: &[Arg], n: usize, name: &str) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}

fn check_arity(args: &[Arg], n: usize, name: &str, _call: &Value) -> RResult<()> {
    check_arity_internal(args, n, name)
}

/// R: RNG.c:do_RNGkind
fn do_rngkind(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity(&args, 3, "RNGkind", call)?;
    get_rng_state(i)?;
    let ans = Value::ints(vec![i.rng.kind as i32, i.rng.n01_kind as i32, i.rng.sample_kind as i32]);
    get_rng_kind(i, None)?;
    if !args[0].value.is_null() {
        let k = scalar_integer_warn(i, &args[0].value)?;
        rng_kind(i, k)?;
    }
    if !args[1].value.is_null() {
        let k = scalar_integer_warn(i, &args[1].value)?;
        norm_kind(i, k)?;
    }
    if !args[2].value.is_null() {
        let k = scalar_integer_warn(i, &args[2].value)?;
        samp_kind(i, k)?;
    }
    Ok(ans)
}

/// R: RNG.c:do_setseed
fn do_setseed(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity(&args, 4, "set.seed", call)?;
    let seed = if !args[0].value.is_null() {
        let s = scalar_integer_warn(i, &args[0].value)?;
        if s == NA_INTEGER {
            return Err(Flow::error("supplied seed is not a valid integer"));
        }
        s as u32
    } else {
        time_to_seed()
    };
    get_rng_kind(i, None)?;
    if !args[1].value.is_null() {
        let k = scalar_integer_warn(i, &args[1].value)?;
        rng_kind(i, k)?;
    }
    if !args[2].value.is_null() {
        let k = scalar_integer_warn(i, &args[2].value)?;
        norm_kind(i, k)?;
    }
    if !args[3].value.is_null() {
        let k = scalar_integer_warn(i, &args[3].value)?;
        samp_kind(i, k)?;
    }
    let k = i.rng.kind;
    rng_init(i, k, seed)?;
    put_rng_state(i)?;
    Ok(Value::Null)
}

// ----------------------------------------------------------------------
// sample (random.c) and sample2 (unique.c)

/// R: sort.c:revsort -- sort a[] decreasingly, permuting ib[] alongside
fn revsort(a: &mut [f64], ib: &mut [i32]) {
    let n = a.len();
    if n <= 1 {
        return;
    }
    // 1-based indexing as in the C code
    let get = |a: &[f64], k: usize| a[k - 1];
    let mut l = (n >> 1) + 1;
    let mut ir = n;
    loop {
        let ra;
        let ii;
        if l > 1 {
            l -= 1;
            ra = get(a, l);
            ii = ib[l - 1];
        } else {
            ra = a[ir - 1];
            ii = ib[ir - 1];
            a[ir - 1] = a[0];
            ib[ir - 1] = ib[0];
            ir -= 1;
            if ir == 1 {
                a[0] = ra;
                ib[0] = ii;
                return;
            }
        }
        let mut i = l;
        let mut j = l << 1;
        while j <= ir {
            if j < ir && a[j - 1] > a[j] {
                j += 1;
            }
            if ra > a[j - 1] {
                a[i - 1] = a[j - 1];
                ib[i - 1] = ib[j - 1];
                i = j;
                j += j;
            } else {
                j = ir + 1;
            }
        }
        a[i - 1] = ra;
        ib[i - 1] = ii;
    }
}

/// R: random.c:ProbSampleReplace
fn prob_sample_replace(rng: &mut Rng, p: &mut [f64], nans: usize) -> Vec<i32> {
    let n = p.len();
    let mut perm: Vec<i32> = (1..=n as i32).collect();
    revsort(p, &mut perm);
    for i in 1..n {
        p[i] += p[i - 1];
    }
    let nm1 = n - 1;
    let mut ans = Vec::with_capacity(nans);
    for _ in 0..nans {
        let ru = rng.unif_rand();
        let mut j = 0;
        while j < nm1 {
            if ru <= p[j] {
                break;
            }
            j += 1;
        }
        ans.push(perm[j]);
    }
    ans
}

/// R: random.c:walker_ProbSampleReplace
fn walker_prob_sample_replace(rng: &mut Rng, p: &[f64], nans: usize) -> Vec<i32> {
    let n = p.len();
    let mut a = vec![0i32; n];
    let mut hl = vec![0usize; n];
    let mut q = vec![0.0f64; n];
    let mut h = 0usize; // HL[0..h) : q < 1
    let mut l = n; // HL[l..n) : q >= 1
    for i in 0..n {
        q[i] = p[i] * n as f64;
        if q[i] < 1.0 {
            hl[h] = i;
            h += 1;
        } else {
            l -= 1;
            hl[l] = i;
        }
    }
    if h > 0 && l < n {
        for k in 0..n - 1 {
            let i = hl[k];
            let j = hl[l];
            a[i] = j as i32;
            q[j] += q[i] - 1.0;
            if q[j] < 1.0 {
                l += 1;
            }
            if l >= n {
                break;
            }
        }
    }
    for (i, qi) in q.iter_mut().enumerate() {
        *qi += i as f64;
    }
    let mut ans = Vec::with_capacity(nans);
    for _ in 0..nans {
        let (k, ru);
        if rng.sample_kind == SampleKind::Rounding {
            let r = rng.unif_rand() * n as f64;
            k = r as i32 as usize;
            ru = r;
        } else {
            k = rng.unif_index(n as f64) as i32 as usize;
            ru = k as f64 + rng.unif_rand();
        }
        ans.push(if ru < q[k] { k as i32 + 1 } else { a[k] + 1 });
    }
    ans
}

/// R: random.c:ProbSampleNoReplace
fn prob_sample_no_replace(rng: &mut Rng, p: &mut [f64], nans: usize) -> Vec<i32> {
    let n = p.len();
    let mut perm: Vec<i32> = (1..=n as i32).collect();
    revsort(p, &mut perm);
    let mut totalmass = 1.0;
    let mut ans = Vec::with_capacity(nans);
    let mut n1 = n as i64 - 1;
    for _ in 0..nans {
        let rt = totalmass * rng.unif_rand();
        let mut mass = 0.0;
        let mut j = 0usize;
        while (j as i64) < n1 {
            mass += p[j];
            if rt <= mass {
                break;
            }
            j += 1;
        }
        ans.push(perm[j]);
        totalmass -= p[j];
        let mut k = j;
        while (k as i64) < n1 {
            p[k] = p[k + 1];
            perm[k] = perm[k + 1];
            k += 1;
        }
        n1 -= 1;
    }
    ans
}

/// R: random.c:FixupProb
fn fixup_prob(p: &mut [f64], require_k: i32, replace: bool) -> RResult<()> {
    let mut sum = 0.0;
    let mut npos = 0;
    for &v in p.iter() {
        if !v.is_finite() {
            return Err(Flow::error("NA in probability vector"));
        }
        if v < 0.0 {
            return Err(Flow::error("negative probability"));
        }
        if v > 0.0 {
            npos += 1;
            sum += v;
        }
    }
    if npos == 0 || (!replace && require_k > npos) {
        return Err(Flow::error("too few positive probabilities"));
    }
    for v in p.iter_mut() {
        *v /= sum;
    }
    Ok(())
}

/// R: builtin.c:asVecSize
fn as_vec_size(i: &mut Interp, x: &Value) -> RResult<i64> {
    if x.is_atomic() && !x.is_empty() {
        match x {
            Value::Int(v) => {
                let r = v.data[0];
                if r == NA_INTEGER {
                    return Err(Flow::error("vector size cannot be NA"));
                }
                return Ok(r as i64);
            }
            Value::Real(_) | Value::Str(_) => {
                // asReal(): coercion warnings for strings
                let mut w = CoerceWarnings::default();
                let d = crate::coerce::to_real(&crate::coerce::first_elt(x), &mut w)[0];
                w.emit(i)?;
                if d.is_nan() {
                    return Err(Flow::error("vector size cannot be NA/NaN"));
                }
                if !d.is_finite() {
                    return Err(Flow::error("vector size cannot be infinite"));
                }
                if d > 4503599627370496.0 {
                    return Err(Flow::error("vector size specified is too large"));
                }
                return Ok(d as i64);
            }
            _ => {}
        }
    }
    Ok(-999)
}

/// R: random.c:do_sample
fn do_sample(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity(&args, 4, "sample", call)?;
    let sn = &args[0].value;
    let sk = &args[1].value;
    if sk.len() != 1 {
        return Err(Flow::error("invalid 'size' argument"));
    }
    let sreplace = &args[2].value;
    if sreplace.len() != 1 {
        return Err(Flow::error("invalid 'replace' argument"));
    }
    let replace = as_logical(sreplace);
    let prob = &args[3].value;
    if replace == NA_LOGICAL {
        return Err(Flow::error("invalid 'replace' argument"));
    }
    let replace = replace != 0;
    get_rng_state(i)?;
    let y;
    if !prob.is_null() {
        let n = as_integer(sn);
        let k = as_integer(sk);
        if n == NA_INTEGER || n < 0 || (k > 0 && n == 0) {
            return Err(Flow::error("invalid first argument"));
        }
        if k == NA_INTEGER || k < 0 {
            return Err(Flow::error("invalid 'size' argument"));
        }
        if !replace && k > n {
            return Err(Flow::error("cannot take a sample larger than the population when 'replace = FALSE'"));
        }
        let mut w = CoerceWarnings::default();
        let mut p = crate::coerce::to_real(prob, &mut w);
        w.emit(i)?;
        if p.len() != n as usize {
            return Err(Flow::error("incorrect number of probabilities"));
        }
        fixup_prob(&mut p, k, replace)?;
        let k = k as usize;
        let out = if replace {
            let nc = p.iter().filter(|&&v| n as f64 * v > 0.1).count();
            if nc > 200 {
                walker_prob_sample_replace(&mut i.rng, &p, k)
            } else {
                prob_sample_replace(&mut i.rng, &mut p, k)
            }
        } else {
            prob_sample_no_replace(&mut i.rng, &mut p, k)
        };
        y = Value::ints(out);
    } else {
        let dn = as_real(sn);
        let k = as_vec_size(i, sk)?;
        if !dn.is_finite() || dn < 0.0 || dn > 4.5e15 || (k > 0 && dn == 0.0) {
            return Err(Flow::error("invalid first argument"));
        }
        if k < 0 {
            return Err(Flow::error("invalid 'size' argument"));
        }
        if !replace && k as f64 > dn {
            return Err(Flow::error("cannot take a sample larger than the population when 'replace = FALSE'"));
        }
        let rng = &mut i.rng;
        if dn > i32::MAX as f64 || k > i32::MAX as i64 {
            let k = k as usize;
            let mut ry = crate::memory::try_vec::<f64>(k, crate::memory::Elt::Real)?;
            if replace {
                for _ in 0..k {
                    ry.push(rng.unif_index(dn) + 1.0);
                }
            } else {
                let mut n = dn as i64;
                let mut x = crate::memory::try_vec::<f64>(n as usize, crate::memory::Elt::Real)?;
                x.extend((0..n).map(|v| v as f64));
                for _ in 0..k {
                    let j = rng.unif_index(n as f64) as i64 as usize;
                    ry.push(x[j] + 1.0);
                    n -= 1;
                    x[j] = x[n as usize];
                }
            }
            y = Value::reals(ry);
        } else {
            let mut n = dn as i32;
            let k = k as usize;
            let mut iy = crate::memory::try_vec::<i32>(k, crate::memory::Elt::Int)?;
            if replace || k < 2 {
                for _ in 0..k {
                    iy.push((rng.unif_index(n as f64) + 1.0) as i32);
                }
            } else {
                let mut x = crate::memory::try_vec::<i32>(n as usize, crate::memory::Elt::Int)?;
                x.extend(0..n);
                for _ in 0..k {
                    let j = rng.unif_index(n as f64) as i32 as usize;
                    iy.push(x[j] + 1);
                    n -= 1;
                    x[j] = x[n as usize];
                }
            }
            y = Value::ints(iy);
        }
    }
    put_rng_state(i)?;
    Ok(y)
}

/// R: unique.c:do_sample2
fn do_sample2(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity(&args, 2, "sample2", call)?;
    let dn = as_real(&args[0].value);
    let sk = &args[1].value;
    if sk.len() != 1 {
        return Err(Flow::error("invalid 'size' argument"));
    }
    let k = as_integer(sk);
    if args[0].value.len() != 1 || !dn.is_finite() || dn < 0.0 || dn > 4.5e15 || (k > 0 && dn == 0.0) {
        return Err(Flow::error("invalid first argument"));
    }
    if k < 0 {
        return Err(Flow::error("invalid 'size' argument"));
    }
    if k as f64 > dn / 2.0 {
        return Err(Flow::error("This algorithm is for size <= n/2"));
    }
    get_rng_state(i)?;
    let k = k as usize;
    let ans = if dn > i32::MAX as f64 {
        let mut seen: HashSet<u64> = HashSet::with_capacity(k);
        let mut ry = Vec::with_capacity(k);
        for _ in 0..k {
            let mut v = 0.0;
            for _ in 0..100 {
                v = i.rng.unif_index(dn) + 1.0;
                if seen.insert(v.to_bits()) {
                    break;
                }
            }
            ry.push(v);
        }
        Value::reals(ry)
    } else {
        let mut seen: HashSet<i32> = HashSet::with_capacity(k);
        let mut iy = Vec::with_capacity(k);
        for _ in 0..k {
            let mut v = 0;
            for _ in 0..100 {
                v = (i.rng.unif_index(dn) + 1.0) as i32;
                if seen.insert(v) {
                    break;
                }
            }
            iy.push(v);
        }
        Value::ints(iy)
    };
    put_rng_state(i)?;
    Ok(ans)
}

// ----------------------------------------------------------------------
// do_random1 / do_random2 / do_random3

fn is_numeric(v: &Value) -> bool {
    match v {
        Value::Int(_) => !v.inherits("factor"),
        Value::Lgl(_) | Value::Real(_) => true,
        _ => false,
    }
}

fn invalid() -> Flow {
    Flow::error("invalid arguments")
}

/// The result vector type of stats/random.c's `random1/2/3`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RandType {
    Int,
    Real,
}

impl RandType {
    fn empty(self) -> Value {
        match self {
            RandType::Int => Value::ints(vec![]),
            RandType::Real => Value::reals(vec![]),
        }
    }
}

/// R: random.c:fillWithNAs
fn fill_with_nas(ty: RandType, n: usize) -> RResult<Value> {
    Ok(match ty {
        RandType::Int => {
            let mut data = crate::memory::try_vec::<i32>(n, crate::memory::Elt::Int)?;
            data.resize(n, NA_INTEGER);
            Value::ints(data)
        }
        RandType::Real => {
            let mut data = crate::memory::try_vec::<f64>(n, crate::memory::Elt::Real)?;
            data.resize(n, NA_REAL);
            Value::reals(data)
        }
    })
}

/// `asReal()` of the length argument, with its coercion warning.
fn scalar_real_warn(i: &mut Interp, v: &Value) -> RResult<f64> {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        let d = crate::coerce::to_real(&crate::coerce::first_elt(v), &mut w)[0];
        w.emit(i)?;
        Ok(d)
    } else {
        Ok(NA_REAL)
    }
}

/// `asInteger()` with its coercion warning.
fn scalar_integer_warn(i: &mut Interp, v: &Value) -> RResult<i32> {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        let n = crate::coerce::to_int(&crate::coerce::first_elt(v), &mut w)[0];
        w.emit(i)?;
        Ok(n)
    } else {
        Ok(NA_INTEGER)
    }
}

/// R: random.c:resultLength
fn random_n(i: &mut Interp, v: &Value) -> RResult<usize> {
    if !v.is_vector() {
        return Err(invalid());
    }
    if v.len() == 1 {
        let dn = scalar_real_warn(i, v)?;
        if dn.is_nan() || dn < 0.0 || dn > 4503599627370496.0 {
            return Err(invalid());
        }
        Ok(dn as usize)
    } else {
        Ok(v.len())
    }
}

fn real_vec(v: &Value) -> Vec<f64> {
    crate::coerce::to_real(v, &mut CoerceWarnings::default())
}

/// R: stats/random.c:random1 / random2 / random3 -- the shared body of the
/// `r<dist>` entry points of the stats package.  `args[0]` is `n`, the
/// rest are the distribution's parameters, recycled as in R.
///
/// Integer-valued distributions return an INTSXP vector; a value outside
/// `int` range switches the whole result to REALSXP at that position, and
/// NaN draws become NA (R warns "NAs produced").
fn random_dispatch(
    i: &mut Interp,
    call: &Value,
    args: Vec<Arg>,
    name: &str,
    arity: usize,
    ty: RandType,
    f: impl Fn(&mut Rng, &[f64]) -> f64,
) -> RResult<Value> {
    check_arity(&args, arity, name, call)?;
    if !args[0].value.is_vector() || !args[1..].iter().all(|a| is_numeric(&a.value)) {
        return Err(invalid());
    }
    let n = random_n(i, &args[0].value)?;
    if n == 0 {
        return Ok(ty.empty());
    }
    if args[1..].iter().any(|a| a.value.len() < 1) {
        let ans = fill_with_nas(ty, n)?;
        i.warning("NAs produced")?;
        return Ok(ans);
    }
    let np = arity - 1;
    let params: Vec<Vec<f64>> = args[1..].iter().map(|a| real_vec(&a.value)).collect();
    get_rng_state(i)?;
    let mut buf = [0.0f64; 3];
    macro_rules! draw {
        ($k:expr) => {{
            for j in 0..np {
                buf[j] = params[j][$k % params[j].len()];
            }
            let v = f(&mut i.rng, &buf[..np]);
            if v.is_nan() {
                if let Some(msg) = crate::nmath::take_alloc_error() {
                    return Err(Flow::error(msg));
                }
            }
            v
        }};
    }
    let mut naflag = false;
    let mut int_data: Vec<i32> = Vec::new();
    let mut real_data: Vec<f64> = Vec::new();
    let mut real_from = n;
    if ty == RandType::Int {
        int_data = crate::memory::try_vec::<i32>(n, crate::memory::Elt::Int)?;
        int_data.resize(n, 0);
        for k in 0..n {
            let rx = draw!(k);
            if rx.is_nan() {
                naflag = true;
                int_data[k] = NA_INTEGER;
            } else if rx > i32::MAX as f64 || rx <= i32::MIN as f64 {
                // integer overflow: coerce the result so far to REALSXP and
                // continue in the real loop below.
                real_data = crate::memory::try_vec::<f64>(n, crate::memory::Elt::Real)?;
                real_data.extend(int_data[..k].iter().map(|&v| if v == NA_INTEGER { NA_REAL } else { v as f64 }));
                real_data.push(rx);
                real_from = k + 1;
                break;
            } else {
                int_data[k] = rx as i32;
            }
        }
    }
    if ty == RandType::Real {
        real_data = crate::memory::try_vec::<f64>(n, crate::memory::Elt::Real)?;
        for k in 0..n {
            let v = draw!(k);
            if v.is_nan() {
                naflag = true;
            }
            real_data.push(v);
        }
    } else if real_from < n {
        for k in real_from..n {
            let v = draw!(k);
            if v.is_nan() {
                naflag = true;
            }
            real_data.push(v);
        }
    }
    if naflag {
        i.warning("NAs produced")?;
    }
    let ans = if ty == RandType::Int && real_data.is_empty() { Value::ints(int_data) } else { Value::reals(real_data) };
    put_rng_state(i)?;
    Ok(ans)
}

// main/random.c returns REALSXP; stats/random.c selects each distribution's type.
macro_rules! random_generators {
    ($(($fname:ident, $rname:literal, $arity:literal, $ty:expr, $f:expr)),* $(,)?) => {
        $(fn $fname(i: &mut Interp, c: &Value, a: Vec<Arg>, _e: &Env) -> RResult<Value> {
            random_dispatch(i, c, a, $rname, $arity, RandType::Real, $f)
        })*

        /// R: library/stats/src/random.c:random1/random2/random3.
        pub(crate) fn stats_random(i: &mut Interp, call: &Value, args: Vec<Arg>, name: &str) -> RResult<Value> {
            match name {
                $($rname => random_dispatch(i, call, args, name, $arity, $ty, $f),)*
                _ => unreachable!("registered random alias"),
            }
        }
    };
}

random_generators! {
    (do_rchisq, "rchisq", 2, RandType::Real, |r, a| nr::rchisq(r, a[0])),
    (do_rexp, "rexp", 2, RandType::Real, |r, a| nr::rexp(r, a[0])),
    (do_rgeom, "rgeom", 2, RandType::Int, |r, a| nr::rgeom(r, a[0])),
    (do_rpois, "rpois", 2, RandType::Int, |r, a| nr::rpois(r, a[0])),
    (do_rt, "rt", 2, RandType::Real, |r, a| nr::rt(r, a[0])),
    (do_rsignrank, "rsignrank", 2, RandType::Int, |r, a| nr::rsignrank(r, a[0])),
    (do_rbeta, "rbeta", 3, RandType::Real, |r, a| nr::rbeta(r, a[0], a[1])),
    (do_rbinom, "rbinom", 3, RandType::Int, |r, a| nr::rbinom(r, a[0], a[1])),
    (do_rcauchy, "rcauchy", 3, RandType::Real, |r, a| nr::rcauchy(r, a[0], a[1])),
    (do_rf, "rf", 3, RandType::Real, |r, a| nr::rf(r, a[0], a[1])),
    (do_rgamma, "rgamma", 3, RandType::Real, |r, a| nr::rgamma(r, a[0], a[1])),
    (do_rlnorm, "rlnorm", 3, RandType::Real, |r, a| nr::rlnorm(r, a[0], a[1])),
    (do_rlogis, "rlogis", 3, RandType::Real, |r, a| nr::rlogis(r, a[0], a[1])),
    (do_rnbinom, "rnbinom", 3, RandType::Int, |r, a| nr::rnbinom(r, a[0], a[1])),
    (do_rnbinom_mu, "rnbinom_mu", 3, RandType::Real, |r, a| nr::rnbinom_mu(r, a[0], a[1])),
    (do_rnchisq, "rnchisq", 3, RandType::Real, |r, a| nr::rnchisq(r, a[0], a[1])),
    (do_rnorm, "rnorm", 3, RandType::Real, |r, a| nr::rnorm(r, a[0], a[1])),
    (do_runif, "runif", 3, RandType::Real, |r, a| nr::runif(r, a[0], a[1])),
    (do_rweibull, "rweibull", 3, RandType::Real, |r, a| nr::rweibull(r, a[0], a[1])),
    (do_rwilcox, "rwilcox", 3, RandType::Int, |r, a| nr::rwilcox(r, a[0], a[1])),
    (do_rhyper, "rhyper", 4, RandType::Int, |r, a| crate::nmath::hyper::rhyper(r, a[0], a[1], a[2])),
}
