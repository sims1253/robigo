//! envir.c's `.Internal`s (`assign`, `get`, `get0`, `exists`, `mget`)
//! against GNU R 4.6.2: the steps of `tests/oracle/envir.R`, replayed on
//! the isolate's environments and compared with its recorded output
//! (`tests/oracle/envir.out`, from the 4.6.2 oracle image).

mod common;

use common::{decode, enc, hex, make};
use robigo_rtbuiltins::registry;
use robigo_rtcore::{Char, Cx, Options, OwnedIsolate, Root, Scope};
use std::collections::HashMap;

/// The recorded lines per tag.
fn oracle() -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for l in include_str!("oracle/envir.out").lines() {
        let (tag, rest) = l.split_once(' ').unwrap();
        m.entry(tag.to_owned()).or_default().push(rest.to_owned());
    }
    m
}

/// An argument: an encoded value (helpers.R's format) or a root.
enum A<'r> {
    V(String),
    R(Root<'r>),
}

fn chr(s: &[&str]) -> A<'static> {
    A::V(format!("chr[{}]", s.iter().map(|x| format!("x{}.n", hex(x.as_bytes()))).collect::<Vec<_>>().join(",")))
}

fn chr_enc(bytes: &[u8], e: char) -> A<'static> {
    A::V(format!("chr[x{}.{e}]", hex(bytes)))
}

fn dbl(x: f64) -> A<'static> {
    A::V(format!("dbl[{:016x}]", x.to_bits()))
}

fn lgl(x: Option<bool>) -> A<'static> {
    A::V(format!(
        "lgl[{}]",
        match x {
            Some(true) => "T",
            Some(false) => "F",
            None => "NA",
        }
    ))
}

fn v(s: &str) -> A<'static> {
    A::V(s.to_owned())
}

/// `.Internal(name(args...))`: the outcome lines as `show()` prints them.
fn internal(cx: &mut Cx, name: &str, args: &[A<'_>]) -> Vec<String> {
    let b = registry::lookup(name, true).unwrap();
    cx.scope(|cx, s| {
        let call = s.root(cx, cx.nil());
        let mut roots = Vec::new();
        for a in args {
            roots.push(match a {
                A::R(r) => s.root(cx, r.bind(cx)),
                A::V(e) => {
                    let val = decode(e);
                    let o = s.out(cx);
                    let r = o.root();
                    cx.build(o, |cx| make(cx, &val)).unwrap();
                    r
                }
            });
        }
        let refs: Vec<_> = roots.iter().map(|r| r.bind(cx)).collect();
        let args = s.args(cx, &refs, None);
        let env = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let res = out.root();
        let r = (b.fun)(cx, call, args, env, out);
        let mut lines: Vec<String> =
            cx.collected_warnings().iter().map(|(_, m)| format!("W : {}", hex(m.bytes()))).collect();
        cx.clear_warnings();
        match r {
            Ok(()) => lines.push(format!("= {}", enc(cx, res.bind(cx)))),
            Err(e) => {
                let o = s.out(cx);
                let m = o.root();
                cx.resolve_exit(e, Some(o));
                let msg = m.bind(cx).as_vector::<Char<'_>>().map(|v| v.get(0).bytes().to_vec()).unwrap_or_default();
                lines.push(format!("E : {}", hex(&msg)));
            }
        }
        lines
    })
}

/// A new hashed environment (`new.env()`) whose enclosure is `parent`.
fn new_env<'s>(cx: &mut Cx, s: Scope<'s>, parent: Root<'_>) -> Root<'s> {
    let e = cx.new_env(parent.bind(cx), true, 29).unwrap();
    s.root(cx, e)
}

fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &str) -> Root<'s> {
    let x = cx.install(name.as_bytes()).unwrap();
    s.root(cx, x)
}

#[test]
fn assign_get_exists_mget_match_gnu_r() {
    let want = oracle();
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() });
    let cx = iso.cx();
    let mut checked = 0;
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let empty = s.root(cx, cx.empty_env());
        let e = new_env(cx, s, g);
        macro_rules! show {
            ($tag:expr, $name:expr, $($a:expr),*) => {{
                let got = internal(cx, $name, &[$($a),*]);
                assert_eq!(&got, want.get($tag).unwrap_or_else(|| panic!("no tag {}", $tag)), "{}", $tag);
                checked += 1;
            }};
        }
        let (any, f) = (chr(&["any"]), lgl(Some(false)));
        let _ = (&any, &f);
        show!("assign.1", "assign", chr(&["a"]), dbl(1.0), A::R(e), lgl(Some(false)));
        show!("get.1", "get", chr(&["a"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("exists.1", "exists", chr(&["a"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("exists.2", "exists", chr(&["b"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get0.miss", "get0", chr(&["b"]), A::R(e), chr(&["any"]), lgl(Some(false)), chr(&["dflt"]));
        show!("get0.hit", "get0", chr(&["a"]), A::R(e), chr(&["any"]), lgl(Some(false)), chr(&["dflt"]));
        show!("get.miss", "get", chr(&["b"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.mode.miss", "get", chr(&["a"]), A::R(e), chr(&["function"]), lgl(Some(false)));
        let e2 = new_env(cx, s, e);
        show!("get.inh", "get", chr(&["a"]), A::R(e2), chr(&["any"]), lgl(Some(true)));
        show!("get.noinh", "get", chr(&["a"]), A::R(e2), chr(&["any"]), lgl(Some(false)));
        show!("exists.inh", "exists", chr(&["a"]), A::R(e2), chr(&["any"]), lgl(Some(true)));
        show!("assign.inh", "assign", chr(&["a"]), dbl(2.0), A::R(e2), lgl(Some(true)));
        show!("get.after", "get", chr(&["a"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("exists.e2", "exists", chr(&["a"]), A::R(e2), chr(&["any"]), lgl(Some(false)));
        show!("assign.new.inh", "assign", chr(&["zz_new"]), dbl(5.0), A::R(e2), lgl(Some(true)));
        show!("exists.global", "exists", chr(&["zz_new"]), A::R(g), chr(&["any"]), lgl(Some(false)));
        show!("assign.multi", "assign", chr(&["m", "n"]), dbl(3.0), A::R(e), lgl(Some(false)));
        show!("get.m", "get", chr(&["m"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("exists.n", "exists", chr(&["n"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("assign.empty", "assign", chr(&[]), dbl(1.0), A::R(e), lgl(Some(false)));
        show!("assign.num", "assign", dbl(1.0), dbl(1.0), A::R(e), lgl(Some(false)));
        show!("assign.nullenv", "assign", chr(&["x"]), dbl(1.0), v("NULL"), lgl(Some(false)));
        show!("assign.badenv", "assign", chr(&["x"]), dbl(1.0), dbl(1.0), lgl(Some(false)));
        show!("assign.inhNA", "assign", chr(&["x"]), dbl(1.0), A::R(e), lgl(None));
        show!("assign.zero", "assign", chr(&[""]), dbl(1.0), A::R(e), lgl(Some(false)));
        show!("assign.na", "assign", v("chr[NA]"), dbl(7.0), A::R(e), lgl(Some(false)));
        show!("get.na", "get", chr(&["NA"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("assign.utf8", "assign", chr_enc("été".as_bytes(), 'u'), dbl(8.0), A::R(e), lgl(Some(false)));
        show!("get.latin1", "get", chr_enc(b"\xe9t\xe9", 'l'), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.bytes", "get", chr_enc(b"\xe9", 'b'), A::R(e), chr(&["any"]), lgl(Some(false)));
        let a = sym(cx, s, "a");
        show!("get.sym", "get", A::R(a), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.len2", "get", chr(&["a", "b"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.empty", "get", chr(&[""]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.num", "get", dbl(1.0), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get.badmode", "get", chr(&["a"]), A::R(e), chr(&["foo"]), lgl(Some(false)));
        show!("get.modenum", "get", chr(&["a"]), A::R(e), dbl(1.0), lgl(Some(false)));
        show!("get.nullenv", "get", chr(&["a"]), v("NULL"), chr(&["any"]), lgl(Some(false)));
        show!("get.badenv", "get", chr(&["a"]), chr(&["x"]), chr(&["any"]), lgl(Some(false)));
        show!("get.inhNA", "get", chr(&["a"]), A::R(e), chr(&["any"]), lgl(None));
        show!("exists.badmode", "exists", chr(&["a"]), A::R(e), chr(&["foo"]), lgl(Some(false)));
        // modes
        internal(cx, "assign", &[chr(&["i"]), v("int[1]"), A::R(e), lgl(Some(false))]);
        internal(cx, "assign", &[chr(&["s"]), chr(&["str"]), A::R(e), lgl(Some(false))]);
        show!("get.mode.numeric", "get", chr(&["i"]), A::R(e), chr(&["numeric"]), lgl(Some(false)));
        show!("get.mode.double", "get", chr(&["i"]), A::R(e), chr(&["double"]), lgl(Some(false)));
        show!("get.mode.integer", "get", chr(&["i"]), A::R(e), chr(&["integer"]), lgl(Some(false)));
        show!("get.mode.char", "get", chr(&["s"]), A::R(e), chr(&["character"]), lgl(Some(false)));
        show!("exists.mode.char", "exists", chr(&["i"]), A::R(e), chr(&["character"]), lgl(Some(false)));
        show!("exists.mode.any", "exists", chr(&["s"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        show!("get0.mode.miss", "get0", chr(&["s"]), A::R(e), chr(&["numeric"]), lgl(Some(false)), v("NULL"));
        internal(cx, "assign", &[chr(&["sh"]), chr(&["inner"]), A::R(e2), lgl(Some(false))]);
        internal(cx, "assign", &[chr(&["sh"]), dbl(99.0), A::R(e), lgl(Some(false))]);
        show!("get.mode.skip", "get", chr(&["sh"]), A::R(e2), chr(&["numeric"]), lgl(Some(true)));
        show!("get.mode.noskip", "get", chr(&["sh"]), A::R(e2), chr(&["any"]), lgl(Some(true)));
        show!("exists.mode.skip", "exists", chr(&["sh"]), A::R(e2), chr(&["logical"]), lgl(Some(true)));
        // mget
        internal(cx, "assign", &[chr(&["p"]), dbl(1.0), A::R(e), lgl(Some(false))]);
        internal(cx, "assign", &[chr(&["q"]), chr(&["x"]), A::R(e), lgl(Some(false))]);
        let nl = || v("list[NULL]");
        show!("mget.1", "mget", chr(&["p", "q"]), A::R(e), chr(&["any"]), nl(), lgl(Some(false)));
        show!(
            "mget.nf",
            "mget",
            chr(&["p", "zz"]),
            A::R(e),
            chr(&["any"]),
            v("list[chr[x6e6f6e65.n]]"),
            lgl(Some(false))
        );
        show!("mget.nfvec", "mget", chr(&["zz", "yy"]), A::R(e), chr(&["any"]), chr(&["u", "v"]), lgl(Some(false)));
        show!("mget.nfnum", "mget", chr(&["zz", "p"]), A::R(e), chr(&["any"]), v("int[1,2]"), lgl(Some(false)));
        show!("mget.mode", "mget", chr(&["p", "q"]), A::R(e), chr(&["numeric", "character"]), nl(), lgl(Some(false)));
        show!(
            "mget.mode.nf",
            "mget",
            chr(&["p", "q"]),
            A::R(e),
            chr(&["numeric"]),
            v("list[lgl[NA]]"),
            lgl(Some(false))
        );
        show!("mget.nflen", "mget", chr(&["p"]), A::R(e), chr(&["any"]), v("list[NULL;NULL]"), lgl(Some(false)));
        show!("mget.nfnull", "mget", chr(&["p"]), A::R(e), chr(&["any"]), v("NULL"), lgl(Some(false)));
        show!("mget.badfirst", "mget", dbl(1.0), A::R(e), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.empty", "mget", chr(&["", "p"]), A::R(e), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.empty2", "mget", chr(&["p", ""]), A::R(e), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.badenv", "mget", chr(&["p"]), dbl(1.0), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.nullenv", "mget", chr(&["p"]), v("NULL"), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.modelen", "mget", chr(&["p", "q"]), A::R(e), chr(&["any", "any", "any"]), nl(), lgl(Some(false)));
        show!("mget.modenum", "mget", chr(&["p"]), A::R(e), dbl(1.0), nl(), lgl(Some(false)));
        show!("mget.badmode", "mget", chr(&["p"]), A::R(e), chr(&["foo"]), nl(), lgl(Some(false)));
        show!("mget.inh", "mget", chr(&["a"]), A::R(e2), chr(&["any"]), nl(), lgl(Some(true)));
        show!("mget.noinh", "mget", chr(&["a"]), A::R(e2), chr(&["any"]), nl(), lgl(Some(false)));
        show!("mget.inhNA", "mget", chr(&["a"]), A::R(e2), chr(&["any"]), nl(), lgl(None));
        show!("mget.zero", "mget", chr(&[]), A::R(e), chr(&["any"]), v("list[]"), lgl(Some(false)));
        show!(
            "mget.named",
            "mget",
            v("chr[x70.n]{6e616d6573=chr[x75.n]}"),
            A::R(e),
            chr(&["any"]),
            nl(),
            lgl(Some(false))
        );
        // locks
        let p = sym(cx, s, "p");
        cx.lock_binding(p, e).unwrap();
        show!("assign.locked", "assign", chr(&["p"]), dbl(2.0), A::R(e), lgl(Some(false)));
        show!("get.locked", "get", chr(&["p"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        cx.lock_environment(e, false).unwrap();
        show!("assign.lockedenv", "assign", chr(&["brand_new"]), dbl(2.0), A::R(e), lgl(Some(false)));
        show!("assign.lockedenv.old", "assign", chr(&["q"]), chr(&["y"]), A::R(e), lgl(Some(false)));
        show!("get.q", "get", chr(&["q"]), A::R(e), chr(&["any"]), lgl(Some(false)));
        // the empty environment
        show!("assign.emptyenv", "assign", chr(&["x"]), dbl(1.0), A::R(empty), lgl(Some(false)));
        show!("get.emptyenv", "get", chr(&["x"]), A::R(empty), chr(&["any"]), lgl(Some(true)));
        show!("exists.emptyenv", "exists", chr(&["x"]), A::R(empty), chr(&["any"]), lgl(Some(true)));
    });
    assert_eq!(checked, want.len(), "every recorded step is replayed");
}
