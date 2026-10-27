//! Workloads for the memory-manager bake-off, written once and generic
//! over `Mem` so both variants run identical code (only the manager
//! differs). Each returns a deterministic checksum; the harness compares
//! the two variants' checksums and refuses to time a mismatch.
//!
//! The shapes come from what R does (see docs/workstreams.md § memory):
//! binarytrees (recursive list allocation), one environment per closure
//! call (fib), lapply (list traffic + env per call), list growth and
//! in-place updates guarded by the sharing count, string vectors over
//! interned CHARSXPs, environment/closure cycles (memory must stay
//! bounded), and a C-API-shaped workload: `extern "C"` routines called
//! through a volatile function pointer with PROTECT/UNPROTECT traffic and
//! an interior `REAL(x)` pointer held across an allocation.

#![allow(unsafe_op_in_unsafe_fn)]

use mm_core::Mem;
use mm_core::sexp::*;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

pub const WORKLOADS: &[&str] = &["binarytrees", "fib", "lapply", "listgrowth", "strings", "cycles", "capi"];

// ---- binarytrees (benchmarks/games shape) ---------------------------------

fn build<M: Mem>(m: &mut M, depth: i64, item: f64) -> M::Handle {
    if depth == 0 {
        let v = m.alloc_real(2);
        let d = m.real_data(&v);
        unsafe {
            *d = item;
            *d.add(1) = item;
        }
        v
    } else {
        let l = build(m, depth - 1, item * 2.0);
        let node = m.alloc_generic(2);
        m.set_generic_elt(&node, 0, &l);
        m.discard(l);
        let r = build(m, depth - 1, item * 2.0 + 1.0);
        m.set_generic_elt(&node, 1, &r);
        m.discard(r);
        node
    }
}

fn check_tree<M: Mem>(m: &mut M, t: &M::Handle) -> u64 {
    if m.sexp_type(t) == REALSXP {
        let d = m.real_data(t);
        unsafe { ((*d + *d.add(1)) * 4096.0) as u64 }
    } else {
        let l = m.generic_elt(t, 0);
        let r = m.generic_elt(t, 1);
        let s = check_tree(m, &l).wrapping_add(check_tree(m, &r));
        m.discard(l);
        m.discard(r);
        s
    }
}

fn w_binarytrees<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let max_depth = (16.0 * scale.sqrt()) as i64;
    let min_depth = 4i64.min(max_depth);
    let mut check = 0u64;
    // stretch tree: die immediately
    let t = build(m, max_depth + 1, 0.0);
    check = check.wrapping_add(check_tree(m, &t));
    m.discard(t);
    // long-lived tree: retained across every iteration
    let long = build(m, max_depth, 0.0);
    for d in (min_depth..=max_depth).step_by(2) {
        let iters = 1usize << (max_depth - d + 1).min(20);
        let mut c = 0u64;
        for _ in 0..iters {
            let t = build(m, d, 0.0);
            c = c.wrapping_add(check_tree(m, &t));
            m.discard(t);
        }
        check = check.wrapping_add(c.wrapping_mul(iters as u64));
    }
    check = check.wrapping_add(check_tree(m, &long));
    m.discard(long);
    check
}

// ---- fib: one environment per closure call --------------------------------

fn fib_call<M: Mem>(m: &mut M, global: &M::Handle, syms: &Syms<M::Handle>, n: i32, calls: &mut u64) -> i64 {
    *calls += 1;
    // the closure's frame: parent is the closure environment (lexical),
    // not the caller's frame
    let e = m.env_new(global);
    let nv = m.int_scalar(n);
    m.env_define(&e, &syms.n, &nv);
    m.discard(nv);
    let r = if n < 2 {
        n as i64
    } else {
        let a = fib_call(m, global, syms, n - 1, calls);
        let av = m.int_scalar(a as i32);
        m.env_define(&e, &syms.a, &av);
        m.discard(av);
        let b = fib_call(m, global, syms, n - 2, calls);
        let bv = m.int_scalar(b as i32);
        m.env_define(&e, &syms.b, &bv);
        m.discard(bv);
        a + b
    };
    m.discard(e);
    r
}

struct Syms<H> {
    n: H,
    a: H,
    b: H,
    x: H,
    g: H,
    xs: H,
    ys: H,
}

fn intern_syms<M: Mem>(m: &mut M) -> Syms<M::Handle> {
    Syms {
        n: m.symbol_intern(b"n"),
        a: m.symbol_intern(b"a"),
        b: m.symbol_intern(b"b"),
        x: m.symbol_intern(b"x"),
        g: m.symbol_intern(b"g"),
        xs: m.symbol_intern(b"x"),
        ys: m.symbol_intern(b"y"),
    }
}

fn w_fib<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let budget = (1_000_000.0 * scale) as u64;
    let ge = m.global_env();
    let syms = intern_syms(m);
    let mut calls = 0u64;
    let mut check = 0u64;
    while calls < budget {
        check = check.wrapping_add(fib_call(m, &ge, &syms, 25, &mut calls) as u64);
    }
    check
}

// ---- lapply: list traffic plus an environment per call --------------------

fn w_lapply<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let n = (100_000.0 * scale) as usize;
    let ge = m.global_env();
    let syms = intern_syms(m);
    let xs = m.alloc_generic(n);
    for i in 0..n {
        let v = m.int_scalar(i as i32);
        m.set_generic_elt(&xs, i, &v);
        m.discard(v);
    }
    let res = m.alloc_generic(n);
    let mut check = 0u64;
    for i in 0..n {
        let elt = m.generic_elt(&xs, i);
        let e = m.env_new(&ge);
        m.env_define(&e, &syms.x, &elt);
        // FUN(x) = x * 2 + 1, computed like a forced closure body
        let xi = m.scalar_int_value(&elt) as i64;
        let r = m.int_scalar((xi * 2 + 1) as i32);
        m.set_generic_elt(&res, i, &r);
        check = check.wrapping_add(xi as u64 * 2 + 1);
        m.discard(r);
        m.discard(e);
        m.discard(elt);
    }
    m.discard(res);
    m.discard(xs);
    check
}

// ---- list growth + in-place updates guarded by the sharing count ----------

fn w_listgrowth<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let n = (100_000.0 * scale) as usize;
    let ge = m.global_env();
    let syms = intern_syms(m);

    // A: grow a list by appending (`r[[length(r)+1]] <- v`)
    let mut r = m.alloc_generic(0);
    for i in 0..n {
        let v = m.int_scalar(i as i32);
        let nr = m.vector_push(&r, &v);
        m.discard(r);
        r = nr;
        m.discard(v);
    }
    let len = m.generic_len(&r) as u64;

    // B: `x[i] <- x[i] + ...` in a loop, no alias: every iteration should
    // update in place (the evaluator moves the value out of the binding,
    // the builtin mutates, the binding is restored)
    let x = m.alloc_real(n);
    let d = m.real_data(&x);
    unsafe {
        for i in 0..n {
            *d.add(i) = i as f64;
        }
    }
    m.env_define(&ge, &syms.xs, &x);
    m.discard(x);
    for i in 0..n {
        let mut v = m.env_take(&ge, &syms.xs).unwrap();
        if m.external_refs(&v) > 0 {
            let dup = m.duplicate(&v);
            m.discard(v);
            v = dup;
        }
        let d = m.real_data(&v);
        unsafe { *d.add(i) += i as f64 * 0.5 };
        m.env_define(&ge, &syms.xs, &v);
        m.discard(v);
    }

    // C: same loop with an alias (`y <- x`): the first write must copy,
    // the rest run in place
    let yv = m.env_lookup(&ge, &syms.xs).unwrap();
    m.env_define(&ge, &syms.ys, &yv);
    m.discard(yv);
    let mut dups = 0u64;
    for i in 0..n {
        let mut v = m.env_take(&ge, &syms.xs).unwrap();
        if m.external_refs(&v) > 0 {
            let dup = m.duplicate(&v);
            m.discard(v);
            v = dup;
            dups += 1;
        }
        let d = m.real_data(&v);
        unsafe { *d.add(i) += 1.0 };
        m.env_define(&ge, &syms.xs, &v);
        m.discard(v);
    }
    assert_eq!(dups, 1, "with an alias, exactly one copy is expected");

    let fin = m.env_lookup(&ge, &syms.xs).unwrap();
    let mut check = len ^ dups;
    let d = m.real_data(&fin);
    unsafe {
        for i in 0..n {
            check = check.wrapping_add((*d.add(i) * 1024.0) as i64 as u64);
        }
    }
    m.discard(fin);
    m.discard(r);
    m.env_take(&ge, &syms.xs);
    m.env_take(&ge, &syms.ys);
    check
}

// ---- strings over interned CHARSXPs ---------------------------------------

fn w_strings<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let round_ops = (300_000.0 * scale) as usize;
    let distinct = (100_000.0 * scale) as usize;
    let mut name = String::new();
    let mut check = 0u64;

    // A: a hot cache of 500 names recycled into a 1000-slot string vector
    let sv = m.alloc_string(1000);
    for i in 0..round_ops {
        name.clear();
        write!(&mut name, "s{}", i % 500).unwrap();
        let c = m.char_intern(name.as_bytes());
        m.set_string_elt(&sv, i % 1000, &c);
        m.discard(c);
        check = check.wrapping_add(i as u64);
    }
    // B: distinct strings (paste0("t", i)): every lookup misses
    let out = m.alloc_string(distinct);
    for i in 0..distinct {
        name.clear();
        write!(&mut name, "t{}", i).unwrap();
        let c = m.char_intern(name.as_bytes());
        m.set_string_elt(&out, i, &c);
        m.discard(c);
    }
    // C: nchar()-style walk
    for i in 0..m.string_len(&out) {
        let c = m.string_elt(&out, i);
        check = check.wrapping_add(m.char_bytes(&c).len() as u64);
        m.discard(c);
    }
    m.discard(sv);
    m.discard(out);
    check
}

// ---- cycles: dead environment/closure graphs must be reclaimed -----------

fn w_cycles<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let n = (100_000.0 * scale) as usize;
    let ge = m.global_env();
    let syms = intern_syms(m);
    let nil = m.nil();
    // the "closure" whose environment binds it: e -> g -> e
    let c = m.closure_new(&nil, &nil, &ge);
    let mut keep = m.alloc_generic(0);
    let mut check = 0u64;
    for i in 0..n {
        let e = m.env_new(&ge);
        let cl = m.closure_new(&nil, &nil, &e);
        m.env_define(&e, &syms.g, &cl);
        if i % 97 == 0 {
            let k = m.vector_push(&keep, &cl);
            m.discard(keep);
            keep = k;
        }
        m.discard(cl);
        m.discard(e);
        check = check.wrapping_add(i as u64);
    }
    m.discard(c);
    // most cycles are garbage: a collection must reclaim them
    m.collect();
    let live_with_keep = m.stats().live_bytes;
    m.discard(keep);
    m.collect();
    let live_after = m.stats().live_bytes;
    assert!(live_after < 4 << 20, "cycles not reclaimed: {live_after} bytes live");
    let _ = live_with_keep;
    check
}

// ---- the C-API-shaped workload ---------------------------------------------

type CRoutine = unsafe extern "C" fn(*mut u8, i64) -> i64;

unsafe extern "C" fn c_inner<M: Mem>(ctx: *mut u8, seed: i64) -> i64 {
    let m = &mut *(ctx as *mut M);
    let tmp = m.alloc_real(64);
    let v = m.protect(&tmp);
    m.discard(tmp);
    let d = m.real_data(&v); // interior pointer, held across allocations
    for i in 0..64 {
        *d.add(i) = (seed + i as i64) as f64;
    }
    let mut k = String::new();
    write!(&mut k, "k{}", seed & 1023).unwrap();
    let kc = m.char_intern(k.as_bytes());
    let key = m.protect(&kc);
    m.discard(kc);
    let tmp = m.alloc_string(8);
    let sv = m.protect(&tmp);
    m.discard(tmp);
    for i in 0..8 {
        m.set_string_elt(&sv, i, &key);
    }
    let sval = m.symbol_intern(b"val");
    let nil = m.nil();
    let pc = m.cons(&v, &nil, &sval, false);
    let pl = m.protect(&pc);
    m.discard(pc);
    let mut sum = 0f64;
    for i in 0..64 {
        sum += *d.add(i);
    }
    let nb = m.char_bytes(&key).len() as i64 + m.string_len(&sv) as i64;
    m.unprotect(4);
    m.discard(v);
    m.discard(key);
    m.discard(sv);
    m.discard(pl);
    sum as i64 + nb
}

unsafe extern "C" fn c_outer<M: Mem>(ctx: *mut u8, it: i64) -> i64 {
    let m = &mut *(ctx as *mut M);
    let f: CRoutine = c_inner::<M>;
    let t = m.int_scalar((it & 0xff) as i32);
    let acc = m.protect(&t);
    m.discard(t);
    let mut r = 0i64;
    for k in 0..4 {
        r = r.wrapping_add(black_box(f)(ctx, it * 4 + k));
    }
    m.unprotect(1);
    m.discard(acc);
    r
}

fn w_capi<M: Mem>(m: &mut M, scale: f64) -> u64 {
    let n = (150_000.0 * scale) as usize;
    let f: CRoutine = c_outer::<M>;
    let ctx = m as *mut M as *mut u8;
    let mut check = 0u64;
    for i in 0..n {
        check = check.wrapping_add(unsafe { black_box(f)(ctx, i as i64) } as u64);
    }
    check
}

// ---- driver ---------------------------------------------------------------

fn run_one<M: Mem>(w: &str, scale: f64, top: usize) -> (u64, mm_core::Stats, usize, u128) {
    let mut m = M::create();
    m.set_stack_top(top);
    let t0 = Instant::now();
    let check = match w {
        "binarytrees" => w_binarytrees(&mut m, scale),
        "fib" => w_fib(&mut m, scale),
        "lapply" => w_lapply(&mut m, scale),
        "listgrowth" => w_listgrowth(&mut m, scale),
        "strings" => w_strings(&mut m, scale),
        "cycles" => w_cycles(&mut m, scale),
        "capi" => w_capi(&mut m, scale),
        _ => panic!("unknown workload {w}"),
    };
    let wall = t0.elapsed().as_millis();
    let s = m.stats();
    let heap = m.heap_reserved();
    drop(m); // after stats: handles are gone by here
    (check, s, heap, wall)
}

fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
        }
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: mmbench <rc|gc> <workload> [scale]");
        eprintln!("workloads: {}", WORKLOADS.join(", "));
        std::process::exit(2);
    }
    let variant = args[1].clone();
    let workload = args[2].clone();
    let scale: f64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(1.0);
    let top = black_box(&args) as *const _ as usize;
    let (check, s, heap, wall) = match variant.as_str() {
        "rc" => run_one::<mm_rc::RcMem>(&workload, scale, top),
        "gc" => run_one::<mm_gc::GcMem>(&workload, scale, top),
        _ => panic!("unknown variant {variant}"),
    };
    let rss = peak_rss_kb();
    println!(
        "{{\"variant\":\"{variant}\",\"workload\":\"{workload}\",\"scale\":{scale},\"check\":{check},\
         \"allocs\":{},{},\"alloc_mb\":{:.1},\"live_mb\":{:.1},\"live_objs\":{},\
         \"collections\":{},\"gc_ms\":{:.2},\"max_pause_us\":{:.1},\
         \"heap_mb\":{:.1},\"wall_ms\":{},\"rss_mb\":{:.1}}}",
        s.allocs,
        format_args!("\"frees\":{}", s.frees),
        s.bytes_allocated as f64 / 1048576.0,
        s.live_bytes as f64 / 1048576.0,
        s.live_objects,
        s.collections,
        s.collect_total_ns as f64 / 1e6,
        s.collect_max_pause_ns as f64 / 1e3,
        heap as f64 / 1048576.0,
        wall,
        rss as f64 / 1024.0,
    );
}
