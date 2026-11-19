//! L1's bar: every record of GNU R 4.6.2's own lazy-load databases
//! (`base.rdb`, `compiler.rdb`, `tools.rdb` of the oracle image's
//! library) unserializes and serializes back to the same bytes, byte code
//! and base's compact sequences (robigo-rt's own ALTREP classes) included.
//!
//! The databases are not in the repository: `scripts/rt-rdb-roundtrip.sh`
//! copies them out of the oracle image, with GNU R's own round trip of each
//! record (`PKG.roundtrip`), and runs this test with `ROBIGO_ORACLE_LIBRARY`
//! set to that copy; CI's rt job does the same with the data its rdb-data
//! job makes. Without the variable the test says so and passes; with it set
//! and a file missing, it fails.
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
use robigo_rtcore::{Args, Cx, Global, Locale, Out, OwnedIsolate, RResult, Ref, Root, View};
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

/// The packages whose databases the bar reads.
const PACKAGES: [&str; 3] = ["base", "compiler", "tools"];

fn gunzip(what: &str, b: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut v).unwrap_or_else(|e| panic!("{what}: not gzip: {e}"));
    v
}

/// A record of an `.rdb`: its uncompressed length (4 bytes, big-endian),
/// then the zlib stream.
fn inflate_record(what: &str, b: &[u8]) -> Vec<u8> {
    assert!(b.len() >= 4, "{what}: a record of {} bytes, shorter than its length header", b.len());
    let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut v = Vec::with_capacity(n);
    flate2::read::ZlibDecoder::new(&b[4..]).read_to_end(&mut v).unwrap_or_else(|e| panic!("{what}: not zlib: {e}"));
    assert_eq!(v.len(), n, "{what}: the record inflates to {} bytes, its header says {n}", v.len());
    v
}

/// `PKG.roundtrip`: each record's name and GNU R's re-serialization.
fn gnur_roundtrips(what: &str, b: &[u8]) -> std::collections::HashMap<String, Vec<u8>> {
    let mut m = std::collections::HashMap::new();
    let mut i = 0;
    let line = |i: &mut usize| {
        let e =
            b[*i..].iter().position(|&c| c == b'\n').unwrap_or_else(|| panic!("{what}: a line without its end")) + *i;
        let s = String::from_utf8_lossy(&b[*i..e]).into_owned();
        *i = e + 1;
        s
    };
    while i < b.len() {
        let name = line(&mut i);
        let n: usize = line(&mut i).parse().unwrap_or_else(|e| panic!("{what}: {name}'s length: {e}"));
        assert!(i + n <= b.len(), "{what}: {name}'s {n} bytes run past the end");
        m.insert(name, b[i..i + n].to_vec());
        i += n;
    }
    m
}

#[derive(Default, Debug)]
struct Tally {
    same: usize,
    stored: usize,
    /// Read or write errors.
    errors: Vec<String>,
    differ: Vec<String>,
}

/// The records an `.rdx` names (its `variables` and `references`): name,
/// offset and length, each checked to lie inside the `.rdb` of
/// `rdb_len` bytes.
fn rdx_records(cx: &mut Cx, pkg: &str, rdx: &[u8], rdb_len: usize) -> Vec<(String, usize, usize)> {
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let idx = o.root();
        let what = format!("{pkg}.rdx");
        if let Err(e) = unserialize_bytes(cx, gunzip(&what, rdx), None, o) {
            panic!("{what} does not unserialize: {}", common::message(cx, e));
        }
        let mut v = Vec::new();
        let l = idx.bind(cx).as_vector::<Ref<'_>>().unwrap_or_else(|| panic!("{what}: not a list"));
        assert!(l.len() >= 2, "{what}: no variables and references");
        for part in [l.get(0), l.get(1)] {
            let items = part.as_vector::<Ref<'_>>().unwrap_or_else(|| panic!("{what}: a part that is not a list"));
            let names = robigo_rtbuiltins::attrib::get_attrib(cx, part, b"names")
                .ok()
                .flatten()
                .and_then(|n| n.as_vector::<robigo_rtcore::Char<'_>>())
                .unwrap_or_else(|| panic!("{what}: a part without names"));
            for i in 0..items.len() {
                let name = String::from_utf8_lossy(names.get(i).bytes()).into_owned();
                let k = items.get(i).as_ints().unwrap_or_else(|| panic!("{what}: {name}'s key is not integers"));
                assert!(
                    k.len() == 2 && k[0] >= 0 && k[1] >= 0,
                    "{what}: {name}'s key {k:?} is not an offset and a length"
                );
                let (off, len) = (k[0] as usize, k[1] as usize);
                assert!(off + len <= rdb_len, "{what}: {name} at {off}+{len} runs past the .rdb's {rdb_len} bytes");
                v.push((name, off, len));
            }
        }
        v
    })
}

fn roundtrip(
    cx: &mut Cx,
    pkg: &str,
    rdb: &[u8],
    records: &[(String, usize, usize)],
    gnur: &std::collections::HashMap<String, Vec<u8>>,
    hooks: (Root<'_>, Root<'_>),
    t: &mut Tally,
) {
    for (name, off, len) in records {
        let data = inflate_record(&format!("{pkg}.rdb: {name}"), &rdb[*off..off + len]);
        let want = gnur.get(name).unwrap_or_else(|| panic!("{pkg}.roundtrip has no {name}"));
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let v = o.root();
            if let Err(e) = unserialize_bytes(cx, data.clone(), Some(hooks.0), o) {
                let m = common::message(cx, e);
                t.errors.push(format!("{name}: {m}"));
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
                    t.errors.push(format!("{name}: re-serialize: {m}"));
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
    // set but incomplete is a broken setup, never a skip: CI sets it only
    // after restoring the data (ci.yml's rdb-data job)
    for pkg in PACKAGES {
        for ext in ["rdb", "rdx", "roundtrip"] {
            let f = lib.join(pkg).join("R").join(format!("{pkg}.{ext}"));
            assert!(
                f.is_file(),
                "ROBIGO_ORACLE_LIBRARY={} is set, but {} is missing: make the data with \
                 scripts/rt-rdb-roundtrip.sh --data-only {}",
                lib.display(),
                f.display(),
                lib.display()
            );
        }
    }
    let mut session = common::Session::jit();
    // (a record that fails to read is reported below: its message, printed
    // too, would be noise)
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
    for pkg in PACKAGES {
        let dir = lib.join(pkg).join("R");
        let read = |ext: &str| {
            let f = dir.join(format!("{pkg}.{ext}"));
            std::fs::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()))
        };
        let (rdb, rdx) = (read("rdb"), read("rdx"));
        let gnur = gnur_roundtrips(&format!("{pkg}.roundtrip"), &read("roundtrip"));
        let mut t = Tally::default();
        cx.scope(|cx, s| {
            let records = rdx_records(cx, pkg, &rdx, rdb.len());
            let hook = |cx: &mut Cx, name: &str| {
                let o = s.out(cx);
                let r = o.root();
                let ex = s.out(cx);
                let exr = ex.root();
                robigo_rt::lower::parse(cx, name, ex).unwrap_or_else(|_| panic!("parsing {name}"));
                let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().expect("parse() gives an expression").get(0));
                let g = s.root(cx, cx.global_env());
                robigo_rt::eval(cx, e, g, o).unwrap_or_else(|_| panic!("evaluating {name}"));
                r
            };
            let hin = hook(cx, "rdb_in_hook");
            let hout = hook(cx, "rdb_out_hook");
            roundtrip(cx, pkg, &rdb, &records, &gnur, (hin, hout), &mut t);
        });
        report.push_str(&format!(
            "{pkg}: {} records as GNU R's round trip ({} of them their stored bytes), {} differ, {} errors\n",
            t.same,
            t.stored,
            t.differ.len(),
            t.errors.len(),
        ));
        for m in t.differ.iter().chain(&t.errors) {
            report.push_str(&format!("  {m}\n"));
        }
        failed |= !t.differ.is_empty() || !t.errors.is_empty();
    }
    eprintln!("{report}");
    assert!(!failed, "{report}");
}

/// V3's bar: every closure of GNU R 4.6.2's base.rdb (base's installed
/// byte code) translates into the IR, with the functions it makes, each
/// translation through the verifier ([`robigo_rt::compile::translation_gaps`]).
/// A translation that stops at a primitive robigo-rt does not have yet
/// ("... robigo-rt lacks", `log`) is counted by its reason: a gap of
/// robigo-rt's the translation waits on. Any other stop fails: code the
/// translation cannot follow, an opcode it does not know, the verifier's
/// refusal. The data and the variable are the round trip's.
#[test]
fn oracle_base_closures_translate() {
    let Some(lib) = std::env::var_os("ROBIGO_ORACLE_LIBRARY") else {
        eprintln!("ROBIGO_ORACLE_LIBRARY is not set: skipped (scripts/rt-rdb-roundtrip.sh sets it)");
        return;
    };
    let dir = std::path::PathBuf::from(lib).join("base").join("R");
    let read = |ext: &str| {
        let f = dir.join(format!("base.{ext}"));
        std::fs::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()))
    };
    let (rdb, rdx) = (read("rdb"), read("rdx"));
    let mut session = common::Session::jit();
    session.val(".Internal(options(show.error.messages = FALSE))");
    let cx = session.cx();
    cx.set_locale(Locale { utf8: false, collate_c: true });
    robigo_rt::define_builtin(cx, "rdb_in_hook", in_hook, 1, 1);
    robigo_rt::define_builtin(cx, "..getNamespace", get_namespace, 1, 2);
    let (mut closures, mut whole) = (0usize, 0usize);
    let mut waits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    cx.scope(|cx, s| {
        let records = rdx_records(cx, "base", &rdx, rdb.len());
        let o = s.out(cx);
        let hin = o.root();
        let ex = s.out(cx);
        let exr = ex.root();
        robigo_rt::lower::parse(cx, "rdb_in_hook", ex).expect("parsing the hook's name");
        let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().expect("parse() gives an expression").get(0));
        let g = s.root(cx, cx.global_env());
        robigo_rt::eval(cx, e, g, o).expect("the hook");
        for (name, off, len) in &records {
            let data = inflate_record(&format!("base.rdb: {name}"), &rdb[*off..off + len]);
            cx.scope(|cx, s| {
                let o = s.out(cx);
                let v = o.root();
                if let Err(e) = unserialize_bytes(cx, data, Some(hin), o) {
                    failures.push(format!("{name}: unserialize: {}", common::message(cx, e)));
                    return;
                }
                if !matches!(v.bind(cx).view(), View::Closure(_)) {
                    return;
                }
                closures += 1;
                match robigo_rt::compile::translation_gaps(cx, v) {
                    Ok(g) if g.is_empty() => whole += 1,
                    Ok(g) => {
                        for why in g {
                            if why.contains("robigo-rt lacks") || why.contains("calling log") {
                                *waits.entry(why).or_default() += 1;
                            } else {
                                failures.push(format!("{name}: {why}"));
                            }
                        }
                    }
                    Err(e) => failures.push(format!("{name}: {}", common::message(cx, e))),
                }
            });
        }
    });
    let mut report = format!(
        "base.rdb: {closures} closures, {whole} translated whole; {} stop(s) waiting on robigo-rt, {} failure(s)\n",
        waits.values().sum::<usize>(),
        failures.len()
    );
    for (why, n) in &waits {
        report.push_str(&format!("  waits ({n}): {why}\n"));
    }
    for m in &failures {
        report.push_str(&format!("  fails: {m}\n"));
    }
    eprintln!("{report}");
    assert!(closures > 1000, "{report}");
    assert!(failures.is_empty(), "{report}");
}

/// V3's crosscheck on base's installed code (`ROBIGO_BC_CROSSCHECK`, CI's
/// rdb step; docs/ledger/g3/2026-11-17-v3-bcodesxp-ir.md): every closure of
/// base.rdb compared with what the from-source compiler makes of its
/// source ([`robigo_rt::compile::crosscheck_installed`]), reported. No
/// closure may differ unexpectedly, and at least [`CROSSCHECK_COMPARED`]
/// are compared (a setup that skipped them would pass empty). The
/// session has base loaded with every binding forced: GNU R compiled
/// base's installed code with all of base defined (makebasedb.R), so the
/// compile decides on base's definitions as cmp.R did (a simple
/// `.Internal` wrapper's, a binding's kind). A definition robigo-rt cannot
/// evaluate yet stays a promise, and the compile that needs it is skipped.
/// (The translation test keeps its session without base: there a missing
/// primitive is a gap it counts, not a stub that hides it.)
const CROSSCHECK_COMPARED: usize = 1100;

#[test]
fn oracle_base_closures_crosscheck() {
    let Some(lib) = std::env::var_os("ROBIGO_ORACLE_LIBRARY") else {
        eprintln!("ROBIGO_ORACLE_LIBRARY is not set: skipped (scripts/rt-rdb-roundtrip.sh sets it)");
        return;
    };
    if std::env::var_os("ROBIGO_BC_CROSSCHECK").is_none_or(|v| v.is_empty() || v == "0") {
        eprintln!("ROBIGO_BC_CROSSCHECK is not set: skipped");
        return;
    }
    let dir = std::path::PathBuf::from(lib).join("base").join("R");
    let read = |ext: &str| {
        let f = dir.join(format!("base.{ext}"));
        std::fs::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()))
    };
    let (rdb, rdx) = (read("rdb"), read("rdx"));
    let src = robigo_rt::base::source_dir().unwrap_or_else(|e| panic!("{e}"));
    let mut iso = OwnedIsolate::new();
    robigo_rt::install_with(iso.cx(), None);
    robigo_rt::base::load(iso.cx(), &src).unwrap_or_else(|e| panic!("{e}"));
    eval_src(
        iso.cx(),
        ".Internal(options(show.error.messages = FALSE))
         for (n in .Internal(ls(baseenv(), TRUE, FALSE)))
             tryCatch(.Internal(get(n, baseenv(), \"any\", FALSE)), error = function(e) NULL)",
    );
    let cx = iso.cx();
    cx.set_locale(Locale { utf8: false, collate_c: true });
    robigo_rt::define_builtin(cx, "rdb_in_hook", in_hook, 1, 1);
    robigo_rt::define_builtin(cx, "..getNamespace", get_namespace, 1, 2);
    let mut closures = 0usize;
    let mut failures = Vec::new();
    let mut verdicts: std::collections::BTreeMap<String, usize> = Default::default();
    cx.scope(|cx, s| {
        let records = rdx_records(cx, "base", &rdx, rdb.len());
        let o = s.out(cx);
        let hin = o.root();
        let ex = s.out(cx);
        let exr = ex.root();
        robigo_rt::lower::parse(cx, "rdb_in_hook", ex).expect("parsing the hook's name");
        let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().expect("parse() gives an expression").get(0));
        let g = s.root(cx, cx.global_env());
        robigo_rt::eval(cx, e, g, o).expect("the hook");
        for (name, off, len) in &records {
            let data = inflate_record(&format!("base.rdb: {name}"), &rdb[*off..off + len]);
            cx.scope(|cx, s| {
                let o = s.out(cx);
                let v = o.root();
                if let Err(e) = unserialize_bytes(cx, data, Some(hin), o) {
                    failures.push(format!("{name}: unserialize: {}", common::message(cx, e)));
                    return;
                }
                if !matches!(v.bind(cx).view(), View::Closure(_)) {
                    return;
                }
                closures += 1;
                match robigo_rt::compile::crosscheck_installed(cx, v) {
                    Ok(Some(c)) => {
                        let verdict = match c {
                            robigo_rt::compile::Verdict::Same => "Same",
                            robigo_rt::compile::Verdict::Expected => "Expected",
                            robigo_rt::compile::Verdict::Unexpected => "Unexpected",
                            robigo_rt::compile::Verdict::Skipped => "Skipped",
                            robigo_rt::compile::Verdict::Untranslated => "Untranslated",
                        };
                        *verdicts.entry(verdict.to_owned()).or_default() += 1;
                    },
                    Ok(None) => *verdicts.entry("no byte code".to_owned()).or_default() += 1,
                    Err(e) => failures.push(format!("{name}: {}", common::message(cx, e))),
                }
            });
        }
    });
    let n = |v: &str| verdicts.get(v).copied().unwrap_or(0);
    eprintln!("base.rdb crosscheck: {closures} closures, {} failure(s); {verdicts:?}", failures.len());
    assert!(closures > 1000, "{closures} closures");
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(n("Unexpected"), 0, "unexpected differences (the report has them): {verdicts:?}");
    let compared = n("Same") + n("Expected");
    assert!(compared >= CROSSCHECK_COMPARED, "{compared} compared, fewer than {CROSSCHECK_COMPARED}: {verdicts:?}");
}

/// Evaluate every expression of `src` in the global environment, each
/// error ignored.
fn eval_src(cx: &mut Cx, src: &str) {
    cx.scope(|cx, s| {
        let ex = s.out(cx);
        let exr = ex.root();
        robigo_rt::lower::parse(cx, src, ex).expect("parsing");
        let n = exr.bind(cx).as_vector::<Ref<'_>>().expect("parse() gives an expression").len();
        let g = s.root(cx, cx.global_env());
        for i in 0..n {
            let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().expect("parse() gives an expression").get(i));
            let o = s.out(cx);
            let _ = robigo_rt::eval(cx, e, g, o);
        }
    });
}
