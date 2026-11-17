//! RNG.c: the generator's state and `.Random.seed` (`GetRNGstate`,
//! `PutRNGstate`), `.Internal(RNGkind(kind, normal.kind, sample.kind))`
//! and `.Internal(set.seed(seed, kind, normal.kind, sample.kind))`. The
//! generators are robigo-numerics' port (`robigo_numerics::rng`); the
//! state GNU R keeps in RNG.c's statics (`RNG_kind`, `N01_kind`,
//! `Sample_kind`, `BM_norm_keep`, `dummy`) is the isolate's [`RngState`].

use crate::util::not_yet;
use robigo_numerics::rng::{N01Kind, NeedTaocp1997Init, Rng, RngKind, SampleKind, time_to_seed};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, RSym, Root, Scope};
use std::cell::RefCell;

/// The isolate's generator: RNG.c's statics.
pub struct RngState(RefCell<Rng>);

const RNG_DEFAULT: RngKind = RngKind::MersenneTwister;
const N01_DEFAULT: N01Kind = N01Kind::Inversion;
const SAMPLE_DEFAULT: SampleKind = SampleKind::Rejection;

/// Run `f` on the isolate's generator (installed on first use, in GNU R's
/// initial state).
pub fn with_rng<R>(cx: &mut Cx, f: impl FnOnce(&mut Rng) -> R) -> R {
    if cx.ext::<RngState>().is_none() {
        cx.set_ext(RngState(RefCell::new(Rng::default())));
    }
    let st = cx.ext::<RngState>().unwrap();
    let mut r = st.0.borrow_mut();
    f(&mut r)
}

fn seeds_sym<'s>(cx: &mut Cx, s: Scope<'s>) -> Root<'s> {
    let sym: robigo_rtcore::Ref<'_> = cx.sym(RSym::Seeds).into();
    s.root(cx, sym)
}

/// `GetSeedsFromVar()`: `.Random.seed` in the global environment's frame
/// (a promise forced), R_UnboundValue if none.
// R: RNG.c:GetSeedsFromVar
fn get_seeds_from_var<'s>(cx: &mut Cx, s: Scope<'s>) -> RResult<Root<'s>> {
    let sym = seeds_sym(cx, s);
    let g = s.root(cx, cx.global_env());
    let seeds = s.capture(cx, |cx, o| cx.find_var_in_frame3(g, sym, true, o))?;
    if seeds.bind(cx).sexptype() == PROMSXP {
        return s.capture(cx, |cx, o| cx.eval(sym, g, o));
    }
    Ok(seeds)
}

/// `RNG_Init(kind, seed)`: the generator seeded; Knuth-TAOCP (1997) by the
/// R function `.TAOCP1997init(seed)` of base.
// R: RNG.c:RNG_Init, RNG_Init_R_KT
fn rng_init(cx: &mut Cx, kind: RngKind, seed: u32) -> RResult<()> {
    if kind == RngKind::UserUnif {
        if !cx.has_native_symbol(b"user_unif_rand") {
            return Err(cx.error("'user_unif_rand' not in load table"));
        }
        // calling a package's C generator
        return Err(not_yet(cx, None, "a user-supplied random number generator ('user_unif_rand')"));
    }
    let Err(NeedTaocp1997Init(s0)) = with_rng(cx, |r| r.init(kind, seed)) else { return Ok(()) };
    let words = cx.scope(|cx, s| {
        let name = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b".TAOCP1997init")?.into())))?;
        let base = s.root(cx, cx.base_env());
        // findVar1(sym, R_BaseEnv, CLOSXP, FALSE)
        let fun = s.capture(cx, |cx, o| cx.find_var_in_frame3(base, name, true, o))?;
        let fun = if fun.bind(cx).sexptype() == PROMSXP { s.capture(cx, |cx, o| cx.eval(fun, base, o))? } else { fun };
        if fun.bind(cx).sexptype() != CLOSXP {
            return Err(cx.error("function '.TAOCP1997init' is missing"));
        }
        let sseed = s.capture(cx, |cx, o| cx.build_scalar(o, s0))?;
        let call = s.capture(cx, |cx, o| cx.build(o, |cx| cx.lang2(fun.bind(cx), sseed.bind(cx))))?;
        let g = s.root(cx, cx.global_env());
        let ans = s.capture(cx, |cx, o| cx.eval(call, g, o))?;
        // memcpy(dummy, INTEGER(ans), 100*sizeof(int))
        let a = ans.bind(cx);
        match a.as_ints() {
            Some(v) if a.sexptype() == INTSXP && v.len() >= 100 => Ok(v[..100].to_vec()),
            _ => Err(not_yet(cx, None, "a '.TAOCP1997init' that does not return 100 integers")),
        }
    })?;
    with_rng(cx, |r| r.set_taocp1997(&words));
    Ok(())
}

/// `Randomize(kind)`: seeded from the time and the process id.
// R: RNG.c:Randomize
fn randomize(cx: &mut Cx, kind: RngKind) -> RResult<()> {
    rng_init(cx, kind, time_to_seed())
}

/// `FixupSeeds(kind, initial)`.
// R: RNG.c:FixupSeeds
fn fixup_seeds(cx: &mut Cx, kind: RngKind, initial: bool) -> RResult<()> {
    if with_rng(cx, |r| r.fixup_seeds(kind, initial)) {
        randomize(cx, kind)?;
    }
    Ok(())
}

/// `GetRNGkind(seeds)`: the kinds from `.Random.seed` (`seeds`, or the
/// variable for `None`); true, after a warning, a fresh seed and
/// `PutRNGstate`, when it is not valid; true without anything done when
/// there is none.
// R: RNG.c:GetRNGkind
fn get_rng_kind(cx: &mut Cx, seeds: Option<Root<'_>>) -> RResult<bool> {
    let tmp = cx.scope(|cx, s| {
        let seeds = match seeds {
            Some(x) => x,
            None => get_seeds_from_var(cx, s)?,
        };
        let v = seeds.bind(cx);
        if v == cx.unbound() {
            return Ok(None);
        }
        if v.sexptype() != INTSXP {
            if v == cx.missing_arg() {
                // R_MissingArgError_c(".Random.seed", ..., "getRNGError")
                return Err(cx.error("argument \".Random.seed\" is missing, with no default"));
            }
            let msg = format!(
                "'.Random.seed' is not an integer vector but of type '{}', so ignored",
                crate::util::type_to_char(v)
            );
            cx.warning(&msg)?;
            return Ok(Some(None));
        }
        // INTEGER(seeds)[0] (a zero-length vector reads past its end in C)
        let Some(&tmp) = v.as_ints().unwrap().first() else {
            return Err(not_yet(cx, None, "a zero-length '.Random.seed'"));
        };
        Ok(Some(Some(tmp)))
    })?;
    let tmp = match tmp {
        None => return Ok(true),
        Some(None) => return invalid(cx),
        Some(Some(t)) => t,
    };
    // avoid overflow here: max current value is 10705
    if tmp == i32::MIN || !(0..=11000).contains(&tmp) {
        cx.warning("'.Random.seed[1]' is not a valid integer, so ignored")?;
        return invalid(cx);
    }
    let (new_rng, new_n01, new_sample) = (tmp % 100, tmp % 10000 / 100, tmp / 10000);
    let (Some(n01), Some(sample)) = (N01Kind::from_code(new_n01), sample_kind(new_sample)) else {
        cx.warning("'.Random.seed[1]' is not a valid Normal type, so ignored")?;
        return invalid(cx);
    };
    let kind = match RngKind::from_code(new_rng) {
        Some(RngKind::UserUnif) => {
            // no user-supplied generator is ever loaded (User_unif_fun)
            cx.warning("'.Random.seed[1] = 5' but no user-supplied generator, so ignored")?;
            return invalid(cx);
        }
        Some(k) => k,
        None => {
            cx.warning("'.Random.seed[1]' is not a valid RNG kind so ignored")?;
            return invalid(cx);
        }
    };
    with_rng(cx, |r| {
        r.kind = kind;
        r.n01_kind = n01;
        r.sample_kind = sample;
    });
    Ok(false)
}

/// GetRNGkind's `invalid:` label: the defaults, a fresh seed, written out.
fn invalid(cx: &mut Cx) -> RResult<bool> {
    with_rng(cx, |r| {
        r.kind = RNG_DEFAULT;
        r.n01_kind = N01_DEFAULT;
        r.sample_kind = SAMPLE_DEFAULT;
    });
    randomize(cx, RNG_DEFAULT)?;
    put_rng_state(cx)?;
    Ok(true)
}

fn sample_kind(code: i32) -> Option<SampleKind> {
    match code {
        0 => Some(SampleKind::Rounding),
        1 => Some(SampleKind::Rejection),
        _ => None,
    }
}

/// `GetRNGstate()`: the generator from `.Random.seed`, or seeded from the
/// time when there is none.
// R: RNG.c:GetRNGstate
pub fn get_rng_state(cx: &mut Cx) -> RResult<()> {
    cx.scope(|cx, s| {
        let seeds = get_seeds_from_var(cx, s)?;
        if seeds.bind(cx) == cx.unbound() {
            let kind = with_rng(cx, |r| r.kind);
            return randomize(cx, kind);
        }
        // this might re-set the generator
        if get_rng_kind(cx, Some(seeds))? {
            return Ok(());
        }
        let kind = with_rng(cx, |r| r.kind);
        let len_seed = kind.n_seed();
        let len = seeds.bind(cx).len().unwrap_or(0);
        if len > 1 && len < len_seed + 1 {
            return Err(cx.error("'.Random.seed' has wrong length"));
        }
        if len == 1 && kind != RngKind::UserUnif {
            return randomize(cx, kind);
        }
        // copy_seeds_in: INTEGER(seeds)[1..=len_seed] (past a shorter
        // vector's end in C)
        let words: Vec<u32> = seeds.bind(cx).as_ints().unwrap()[1..].iter().take(len_seed).map(|&w| w as u32).collect();
        if words.len() < len_seed {
            return Err(not_yet(cx, None, "a '.Random.seed' shorter than its generator's seed"));
        }
        with_rng(cx, |r| r.seed[..len_seed].copy_from_slice(&words));
        fixup_seeds(cx, kind, false)
    })
}

/// `PutRNGstate()`: `.Random.seed` in the global environment, the kinds'
/// code then the seed; the existing vector rewritten when it is not shared,
/// has no attributes and has the length, else a fresh one bound.
// R: RNG.c:PutRNGstate
pub fn put_rng_state(cx: &mut Cx) -> RResult<()> {
    // (RNG_kind, N01_kind and Sample_kind are always valid here: the
    // "Internal .Random.seed is corrupt" warning cannot happen)
    let (kinds, words) = with_rng(cx, |r| {
        let kinds = r.kind as i32 + 100 * r.n01_kind as i32 + 10000 * r.sample_kind as i32;
        let words: Vec<i32> = r.seed[..r.kind.n_seed()].iter().map(|&w| w as i32).collect();
        (kinds, words)
    });
    cx.scope(|cx, s| {
        let sym = seeds_sym(cx, s);
        let g = s.root(cx, cx.global_env());
        let seeds = s.capture(cx, |cx, o| cx.find_var_in_frame3(g, sym, true, o))?;
        let v = seeds.bind(cx);
        let reuse = v.sexptype() == INTSXP
            && !v.maybe_shared()
            && !crate::attrib::has_attrib(v)
            && !v.is_altrep()
            && v.len() == Some(words.len() + 1);
        if reuse {
            // it is safe to reuse the existing .Random.seed vector
            let mut u = cx.owned_mut::<i32>(seeds)?;
            let p = u.as_mut_slice();
            p[0] = kinds;
            p[1..].copy_from_slice(&words);
            return Ok(());
        }
        // need to allocate a fresh .Random.seed vector
        let fresh = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<i32>(words.len() + 1)?;
                let p = v.as_mut_slice();
                p[0] = kinds;
                p[1..].copy_from_slice(&words);
                Ok(v.finish().into())
            })
        })?;
        // assign only in the workspace
        cx.define_var(sym, fresh, g)
    })
}

/// `RNGkind(newkind)`: a new generator, seeded from the old one's next
/// uniform.
// R: RNG.c:RNGkind
fn rng_kind(cx: &mut Cx, newkind: i32) -> RResult<()> {
    let newkind = if newkind == -1 { RNG_DEFAULT as i32 } else { newkind };
    let kind = match RngKind::from_code(newkind) {
        Some(k) => {
            if k == RngKind::MarsagliaMulticarry {
                cx.warning("RNGkind: Marsaglia-Multicarry has poor statistical properties")?;
            }
            k
        }
        None => return Err(cx.error(&format!("RNGkind: unimplemented RNG kind {newkind}"))),
    };
    get_rng_state(cx)?;
    // precaution against corruption as per package randtoolbox
    let u = unif_rand(cx)?;
    if !(0.0..=1.0).contains(&u) {
        cx.warning("someone corrupted the random-number generator: re-initializing")?;
        rng_init(cx, kind, time_to_seed())?;
    } else {
        // (Int32) (u * UINT_MAX)
        rng_init(cx, kind, (u * u32::MAX as f64) as u32)?;
    }
    with_rng(cx, |r| r.kind = kind);
    put_rng_state(cx)
}

/// `unif_rand()`.
// R: RNG.c:unif_rand
pub fn unif_rand(cx: &mut Cx) -> RResult<f64> {
    if with_rng(cx, |r| r.kind) == RngKind::UserUnif {
        return Err(not_yet(cx, None, "a user-supplied random number generator ('user_unif_rand')"));
    }
    Ok(with_rng(cx, |r| r.unif_rand()))
}

/// `Norm_kind(kind)`.
// R: RNG.c:Norm_kind
fn norm_kind(cx: &mut Cx, kind: i32) -> RResult<()> {
    let rng = with_rng(cx, |r| r.kind);
    if kind == N01Kind::KindermanRamage as i32 && rng == RngKind::MarsagliaMulticarry {
        cx.warning("RNGkind: severe deviations from normality for Kinderman-Ramage + Marsaglia-Multicarry")?;
    }
    if kind == N01Kind::AhrensDieter as i32 && rng == RngKind::MarsagliaMulticarry {
        cx.warning("RNGkind: deviations from normality for Ahrens-Dieter + Marsaglia-Multicarry")?;
    }
    let kind = if kind == -1 { N01_DEFAULT as i32 } else { kind };
    // (an unsigned enum: a negative kind is above KINDERMAN_RAMAGE)
    let Some(k) = N01Kind::from_code(kind) else {
        return Err(cx.error("invalid Normal type in 'RNGkind'"));
    };
    if k == N01Kind::UserNorm {
        if !cx.has_native_symbol(b"user_norm_rand") {
            return Err(cx.error("'user_norm_rand' not in load table"));
        }
        // calling a package's C generator
        return Err(not_yet(cx, None, "a user-supplied normal generator ('user_norm_rand')"));
    }
    // might not be initialized
    get_rng_state(cx)?;
    with_rng(cx, |r| {
        if k == N01Kind::BoxMuller {
            // zap Box-Muller history
            r.bm_norm_keep = 0.0;
        }
        r.n01_kind = k;
    });
    put_rng_state(cx)
}

/// `Samp_kind(kind)`.
// R: RNG.c:Samp_kind
fn samp_kind(cx: &mut Cx, kind: i32) -> RResult<()> {
    let kind = if kind == -1 { SAMPLE_DEFAULT as i32 } else { kind };
    let Some(k) = sample_kind(kind) else {
        return Err(cx.error("invalid sample type in 'RNGkind'"));
    };
    // might not be initialized
    get_rng_state(cx)?;
    with_rng(cx, |r| r.sample_kind = k);
    put_rng_state(cx)
}

/// `asInteger(x)` with its coercion warnings.
fn as_int(cx: &mut Cx, x: Root<'_>) -> RResult<i32> {
    crate::coerce::vector::as_integer_warn(cx, x)
}

/// `.Internal(RNGkind(kind, normal.kind, sample.kind))`: the kinds in use
/// (before any change), then each non-NULL one set.
// R: RNG.c:do_RNGkind
pub fn do_rngkind(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("RNGkind", true).check_arity(cx, call, args)?;
    // might not be initialized
    get_rng_state(cx)?;
    let ans = with_rng(cx, |r| [r.kind as i32, r.n01_kind as i32, r.sample_kind as i32]);
    cx.scope(|cx, s| {
        let ans = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&ans)?.finish().into())))?;
        // pull from .Random.seed if present
        get_rng_kind(cx, None)?;
        let (rng, norm, sample) = (args.root(0), args.root(1), args.root(2));
        if rng.bind(cx) != cx.nil() {
            let k = as_int(cx, rng)?;
            rng_kind(cx, k)?;
        }
        if norm.bind(cx) != cx.nil() {
            let k = as_int(cx, norm)?;
            norm_kind(cx, k)?;
        }
        if sample.bind(cx) != cx.nil() {
            let k = as_int(cx, sample)?;
            samp_kind(cx, k)?;
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

/// `.Internal(set.seed(seed, kind, normal.kind, sample.kind))`: the kinds
/// set as `RNGkind` sets them, then the generator seeded with `seed` (the
/// time for NULL).
// R: RNG.c:do_setseed
pub fn do_setseed(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("set.seed", true).check_arity(cx, call, args)?;
    let seed = if args.get(cx, 0) != cx.nil() {
        let seed = as_int(cx, args.root(0))?;
        if seed == i32::MIN {
            return Err(cx.error("supplied seed is not a valid integer"));
        }
        seed as u32
    } else {
        time_to_seed()
    };
    // pull RNG_kind, N01_kind from .Random.seed if present
    get_rng_kind(cx, None)?;
    let (skind, nkind, sampkind) = (args.root(1), args.root(2), args.root(3));
    if skind.bind(cx) != cx.nil() {
        let k = as_int(cx, skind)?;
        rng_kind(cx, k)?;
    }
    if nkind.bind(cx) != cx.nil() {
        let k = as_int(cx, nkind)?;
        norm_kind(cx, k)?;
    }
    if sampkind.bind(cx) != cx.nil() {
        let k = as_int(cx, sampkind)?;
        samp_kind(cx, k)?;
    }
    // zaps BM history
    let kind = with_rng(cx, |r| r.kind);
    rng_init(cx, kind, seed)?;
    put_rng_state(cx)?;
    out.set(cx, cx.nil());
    Ok(())
}
