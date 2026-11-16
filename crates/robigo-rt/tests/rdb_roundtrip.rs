//! L1's bar: every record of GNU R 4.6.2's own lazy-load databases
//! (`base.rdb`, `compiler.rdb`, `tools.rdb` of the oracle image's
//! library) unserializes and serializes back to the same bytes, the
//! records holding byte code excepted until robigo-rt has BCODESXP
//! (C1-BC).
//!
//! The databases are not in the repository: `scripts/rt-rdb-roundtrip.sh`
//! copies them out of the oracle image, with GNU R's own round trip of each
//! record (`PKG.roundtrip`), and runs this test with `ROBIGO_ORACLE_LIBRARY`
//! set to that copy. Without it the test says so and passes.
//!
//! The comparison is with GNU R's round trip: GNU R's reader changes a few
//! records (a forced promise's PRENV, R_NilValue, becomes the base
//! environment: serialize.c's `ReadItem_Iterative`), so GNU R's own
//! re-serialization of those is not their bytes; the test counts how many
//! records round-trip to their stored bytes as well.
//!
//! The persistent names (`env::N`, the environments makeLazyLoadDB writes
//! as their own records) are restored to an environment per name and
//! written back as that name, so their records round-trip as bytes too.

mod common;

use robigo_rt::serialize::{Format, serialize_bytes, unserialize_bytes};
use robigo_rtcore::{Args, Cx, Global, Locale, Out, RResult, Ref, Root, View};
use std::cell::RefCell;
use std::io::Read;

thread_local! {
    static ENVS: RefCell<Vec<(Vec<u8>, Global)>> = const { RefCell::new(Vec::new()) };
}

/// The restore hook: an environment per persistent name.
fn in_hook(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let name = match args.get(cx, 0).view() {
        View::Str(v) => v.get(0).bytes().to_vec(),
        _ => panic!("a persistent name that is not a string"),
    };
    let found = ENVS.with(|e| e.borrow().iter().find(|(n, _)| *n == name).map(|(_, g)| g.bind(cx).sexp()));
    if let Some(x) = found {
        // SAFETY: rooted by its global.
        out.set(cx, unsafe { Ref::from_sexp(x, cx) });
        return Ok(());
    }
    let o = out.root();
    cx.build(out, |cx| cx.new_env(cx.empty_env(), true, 29))?;
    let g = cx.global(o.bind(cx));
    ENVS.with(|e| e.borrow_mut().push((name, g)));
    Ok(())
}

/// The serialize hook: an environment's persistent name, if it has one.
fn out_hook(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let x = args.get(cx, 0).sexp();
    let name = ENVS.with(|e| e.borrow().iter().find(|(_, g)| g.bind(cx).sexp() == x).map(|(n, _)| n.clone()));
    match name {
        Some(n) => cx.build(out, |cx| cx.mk_string(&n, robigo_rtcore::Encoding::Native)),
        None => {
            out.set(cx, cx.nil());
            Ok(())
        }
    }
}

/// `..getNamespace(info, where)`: a stand-in namespace per name, which
/// `R_IsNamespaceEnv` takes for one (its `.__NAMESPACE__.$spec` is the
/// name), so it is written back as the same NAMESPACESXP.
fn get_namespace(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let mut key = b"ns:".to_vec();
    match args.get(cx, 0).view() {
        View::Str(v) => {
            for i in 0..v.len() {
                key.extend_from_slice(v.get(i).bytes());
                key.push(0);
            }
        }
        _ => panic!("a namespace spec that is not a string"),
    }
    let found = ENVS.with(|e| e.borrow().iter().find(|(n, _)| *n == key).map(|(_, g)| g.bind(cx).sexp()));
    if let Some(x) = found {
        // SAFETY: rooted by its global.
        out.set(cx, unsafe { Ref::from_sexp(x, cx) });
        return Ok(());
    }
    cx.scope(|cx, s| {
        let ns = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.empty_env(), true, 29)))?;
        let info = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.empty_env(), true, 29)))?;
        let spec = args.root(0);
        let spec_sym = cx.install_rooted(s, b"spec")?;
        cx.define_var(spec_sym, spec, info)?;
        let ns_sym = cx.install_rooted(s, b".__NAMESPACE__.")?;
        cx.define_var(ns_sym, info, ns)?;
        let g = cx.global(ns.bind(cx));
        ENVS.with(|e| e.borrow_mut().push((key, g)));
        out.set(cx, ns.bind(cx));
        Ok(())
    })
}

fn gunzip(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut v).expect("a gzip file");
    v
}

fn inflate_record(b: &[u8]) -> Vec<u8> {
    let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut v = Vec::with_capacity(n);
    flate2::read::ZlibDecoder::new(&b[4..]).read_to_end(&mut v).expect("a zlib record");
    assert_eq!(v.len(), n);
    v
}

/// `PKG.roundtrip`: each record's name and GNU R's re-serialization.
fn gnur_roundtrips(b: &[u8]) -> std::collections::HashMap<String, Vec<u8>> {
    let mut m = std::collections::HashMap::new();
    let mut i = 0;
    let line = |i: &mut usize| {
        let e = b[*i..].iter().position(|&c| c == b'\n').unwrap() + *i;
        let s = String::from_utf8_lossy(&b[*i..e]).into_owned();
        *i = e + 1;
        s
    };
    while i < b.len() {
        let name = line(&mut i);
        let n: usize = line(&mut i).parse().unwrap();
        m.insert(name, b[i..i + n].to_vec());
        i += n;
    }
    m
}

#[derive(Default, Debug)]
struct Tally {
    same: usize,
    stored: usize,
    bytecode: usize,
    other_gap: Vec<String>,
    differ: Vec<String>,
}

/// The keys of an `.rdx` (its `variables` and `references`): name, offset,
/// length.
fn keys(cx: &mut Cx, rdx: &[u8]) -> Vec<(String, usize, usize)> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let idx = o.root();
        unserialize_bytes(cx, gunzip(rdx), None, o).expect("the index unserializes");
        let mut v = Vec::new();
        let l = idx.bind(cx).as_vector::<Ref<'_>>().unwrap();
        for part in [l.get(0), l.get(1)] {
            let items = part.as_vector::<Ref<'_>>().unwrap();
            let names = robigo_rtbuiltins::attrib::get_attrib(cx, part, b"names").unwrap().unwrap();
            let names = names.as_vector::<robigo_rtcore::Char<'_>>().unwrap();
            for i in 0..items.len() {
                let k = items.get(i).as_ints().unwrap();
                v.push((String::from_utf8_lossy(names.get(i).bytes()).into_owned(), k[0] as usize, k[1] as usize));
            }
        }
        v
    })
}

fn roundtrip(
    cx: &mut Cx,
    rdb: &[u8],
    keys: &[(String, usize, usize)],
    gnur: &std::collections::HashMap<String, Vec<u8>>,
    hooks: (Root<'_>, Root<'_>),
    t: &mut Tally,
) {
    for (name, off, len) in keys {
        let data = inflate_record(&rdb[*off..off + len]);
        let want = &gnur[name];
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let v = o.root();
            if let Err(e) = unserialize_bytes(cx, data.clone(), Some(hooks.0), o) {
                let m = common::message(cx, e);
                if m.contains("byte code") {
                    t.bytecode += 1;
                } else {
                    t.other_gap.push(format!("{name}: {m}"));
                }
                return;
            }
            match serialize_bytes(cx, v, Format::Xdr, 3, Some(hooks.1)) {
                Ok(b) if b == *want => {
                    t.same += 1;
                    t.stored += (b == data) as usize;
                }
                Ok(b) => {
                    let at = b.iter().zip(want).position(|(x, y)| x != y).unwrap_or(b.len().min(want.len()));
                    t.differ.push(format!("{name}: {} vs {} bytes, first difference at {at}", b.len(), want.len()));
                }
                Err(e) => {
                    let m = common::message(cx, e);
                    t.other_gap.push(format!("{name}: re-serialize: {m}"));
                }
            }
        });
    }
}

#[test]
fn oracle_databases_round_trip() {
    let Some(lib) = std::env::var_os("ROBIGO_ORACLE_LIBRARY") else {
        eprintln!("ROBIGO_ORACLE_LIBRARY is not set: skipped (scripts/rt-rdb-roundtrip.sh sets it)");
        return;
    };
    let lib = std::path::PathBuf::from(lib);
    let mut session = common::Session::jit();
    // (the records with byte code are errors, and their messages noise)
    session.val(".Internal(options(show.error.messages = FALSE))");
    let cx = session.cx();
    // the databases were written in the build's C locale (their headers
    // say "ANSI_X3.4-1968"): the same native encoding writes the same bytes
    cx.set_locale(Locale { utf8: false, collate_c: true });
    robigo_rt::define_builtin(cx, "rdb_in_hook", in_hook, 1, 1);
    robigo_rt::define_builtin(cx, "rdb_out_hook", out_hook, 1, 1);
    robigo_rt::define_builtin(cx, "..getNamespace", get_namespace, 1, 2);
    let mut report = String::new();
    let mut failed = false;
    for pkg in ["base", "compiler", "tools"] {
        let dir = lib.join(pkg).join("R");
        let rdb = std::fs::read(dir.join(format!("{pkg}.rdb"))).expect("the .rdb");
        let rdx = std::fs::read(dir.join(format!("{pkg}.rdx"))).expect("the .rdx");
        let gnur = gnur_roundtrips(&std::fs::read(dir.join(format!("{pkg}.roundtrip"))).expect("GNU R's round trip"));
        let mut t = Tally::default();
        cx.scope(|cx, s| {
            let ks = keys(cx, &rdx);
            let hook = |cx: &mut Cx, name: &str| {
                let o = s.out(cx);
                let r = o.root();
                let src = format!("{name}");
                let ex = s.out(cx);
                let exr = ex.root();
                robigo_rt::lower::parse(cx, &src, ex).expect("parse");
                let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
                let g = s.root(cx, cx.global_env());
                robigo_rt::eval(cx, e, g, o).expect("the hook");
                r
            };
            let hin = hook(cx, "rdb_in_hook");
            let hout = hook(cx, "rdb_out_hook");
            roundtrip(cx, &rdb, &ks, &gnur, (hin, hout), &mut t);
        });
        report.push_str(&format!(
            "{pkg}: {} records as GNU R's round trip ({} of them their stored bytes), {} with byte code (C1-BC), {} other gaps, {} differ\n",
            t.same,
            t.stored,
            t.bytecode,
            t.other_gap.len(),
            t.differ.len()
        ));
        for m in t.other_gap.iter().chain(&t.differ).take(20) {
            report.push_str(&format!("  {m}\n"));
        }
        failed |= !t.differ.is_empty() || !t.other_gap.is_empty();
    }
    eprintln!("{report}");
    assert!(!failed, "{report}");
}
