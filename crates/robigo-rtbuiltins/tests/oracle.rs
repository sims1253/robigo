//! The builtins against GNU R 4.6.2: every case in `tests/oracle/*.out`
//! (recorded by `tests/oracle/record.sh` with the 4.6.2 oracle image, never
//! the host's R) is replayed through the registry: its inputs decoded, its
//! call built, the builtin called, and the value, warnings (in order, with
//! the call each reports) and error compared with GNU R's, the doubles bit
//! for bit (NA against NaN) and the strings byte for byte with their
//! encoding mark. The format is described in `tests/oracle/helpers.R`.

// Tests make functions in the raw layer (their constructors are C1's and
// rt's).
#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtbuiltins::registry;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Char, Cx, Dispatched, Encoding, Generic, Hooks, Lgl, Locale, NeedGc, Options, Out, OwnedIsolate, RResult,
    Rcomplex, Ref, Root, View,
};

// ---- the encoding -----------------------------------------------------------

/// A decoded value.
#[derive(Clone, Debug)]
struct V {
    core: Core,
    attrs: Vec<(Vec<u8>, V)>,
}

#[derive(Clone, Debug)]
enum Core {
    Null,
    Lgl(Vec<i32>),
    Int(Vec<i32>),
    Dbl(Vec<u64>),
    Cplx(Vec<(u64, u64)>),
    Chr(Vec<Option<(Vec<u8>, u8)>>),
    Raw(Vec<u8>),
    List(Vec<V>),
    Sym(Vec<u8>),
    Lang(Vec<V>),
    /// a function or environment (only its type matters to the cases)
    Other(u8),
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn eat(&mut self, t: &str) -> bool {
        if self.s[self.i..].starts_with(t.as_bytes()) {
            self.i += t.len();
            true
        } else {
            false
        }
    }
    fn until(&mut self, stops: &[u8]) -> &str {
        let st = self.i;
        while self.i < self.s.len() && !stops.contains(&self.s[self.i]) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[st..self.i]).unwrap()
    }
    /// comma-separated atoms up to `]`
    fn atoms(&mut self) -> Vec<String> {
        let mut v = Vec::new();
        if self.eat("]") {
            return v;
        }
        loop {
            v.push(self.until(b",]").to_owned());
            if self.eat("]") {
                return v;
            }
            assert!(self.eat(","));
        }
    }
    fn items(&mut self, close: &str) -> Vec<V> {
        let mut v = Vec::new();
        if self.eat(close) {
            return v;
        }
        loop {
            v.push(self.value());
            if self.eat(close) {
                return v;
            }
            assert!(self.eat(";"), "at {}", self.i);
        }
    }
    fn value(&mut self) -> V {
        let core = if self.eat("NULL") {
            Core::Null
        } else if self.eat("lgl[") {
            Core::Lgl(
                self.atoms()
                    .iter()
                    .map(|a| match a.as_str() {
                        "T" => 1,
                        "F" => 0,
                        _ => i32::MIN,
                    })
                    .collect(),
            )
        } else if self.eat("int[") {
            Core::Int(self.atoms().iter().map(|a| if a == "NA" { i32::MIN } else { a.parse().unwrap() }).collect())
        } else if self.eat("dbl[") {
            Core::Dbl(self.atoms().iter().map(|a| u64::from_str_radix(a, 16).unwrap()).collect())
        } else if self.eat("cplx[") {
            Core::Cplx(
                self.atoms()
                    .iter()
                    .map(|a| {
                        let (r, i) = a.split_once('/').unwrap();
                        (u64::from_str_radix(r, 16).unwrap(), u64::from_str_radix(i, 16).unwrap())
                    })
                    .collect(),
            )
        } else if self.eat("chr[") {
            Core::Chr(
                self.atoms()
                    .iter()
                    .map(|a| {
                        if a == "NA" {
                            None
                        } else {
                            let (h, e) = a[1..].split_once('.').unwrap();
                            Some((unhex(h), e.as_bytes()[0]))
                        }
                    })
                    .collect(),
            )
        } else if self.eat("raw[") {
            Core::Raw(self.atoms().iter().map(|a| u8::from_str_radix(a, 16).unwrap()).collect())
        } else if self.eat("list[") {
            Core::List(self.items("]"))
        } else if self.eat("sym(") {
            let h = self.until(b")").to_owned();
            assert!(self.eat(")"));
            Core::Sym(unhex(&h))
        } else if self.eat("lang(") {
            Core::Lang(self.items(")"))
        } else if self.eat("<builtin>") {
            Core::Other(BUILTINSXP)
        } else if self.eat("<closure>") {
            Core::Other(CLOSXP)
        } else if self.eat("<environment>") {
            Core::Other(ENVSXP)
        } else {
            panic!("cannot decode at {}: {}", self.i, String::from_utf8_lossy(&self.s[self.i..]));
        };
        let mut attrs = Vec::new();
        if self.eat("{") {
            loop {
                let n = unhex(self.until(b"="));
                assert!(self.eat("="));
                attrs.push((n, self.value()));
                if self.eat("}") {
                    break;
                }
                assert!(self.eat(";"));
            }
        }
        V { core, attrs }
    }
}

fn decode(s: &str) -> V {
    let mut p = P { s: s.as_bytes(), i: 0 };
    let v = p.value();
    assert_eq!(p.i, s.len(), "trailing input in {s}");
    v
}

fn enc_of(e: u8) -> Encoding {
    match e {
        b'u' => Encoding::Utf8,
        b'l' => Encoding::Latin1,
        b'b' => Encoding::Bytes,
        _ => Encoding::Native,
    }
}

/// Make `v` (fresh objects, under `&Cx`).
fn make<'g>(cx: &'g Cx, v: &V) -> Result<Ref<'g>, NeedGc> {
    let mut attrs = cx.nil();
    for (n, a) in v.attrs.iter().rev() {
        let val = make(cx, a)?;
        attrs = cx.cons_tagged(val, attrs, cx.install(n)?)?;
    }
    let object = v.attrs.iter().any(|(n, _)| n == b"class");
    macro_rules! fin {
        ($f:expr) => {{
            let mut f = $f;
            f.set_attrib(attrs);
            f.set_object(object);
            f.finish().into()
        }};
    }
    Ok(match &v.core {
        Core::Null => cx.nil(),
        Core::Lgl(x) => fin!(cx.alloc_with(x.len(), |i| Lgl(x[i]))?),
        Core::Int(x) => fin!(cx.alloc_copy(x)?),
        Core::Dbl(x) => fin!(cx.alloc_with(x.len(), |i| f64::from_bits(x[i]))?),
        Core::Cplx(x) => {
            fin!(cx.alloc_with(x.len(), |i| Rcomplex { r: f64::from_bits(x[i].0), i: f64::from_bits(x[i].1) })?)
        }
        Core::Raw(x) => fin!(cx.alloc_copy(x)?),
        Core::Chr(x) => {
            let mut f = cx.alloc_vector::<Char<'_>>(x.len())?;
            for (i, e) in x.iter().enumerate() {
                let c = match e {
                    None => cx.na_string(),
                    Some((b, e)) => cx.mk_char(b, enc_of(*e))?,
                };
                f.set(i, c);
            }
            fin!(f)
        }
        Core::List(x) => {
            let mut f = cx.alloc_vector::<Ref<'_>>(x.len())?;
            for (i, e) in x.iter().enumerate() {
                let r = make(cx, e)?;
                f.set(i, r);
            }
            fin!(f)
        }
        Core::Sym(n) => cx.install(n)?.into(),
        Core::Other(ENVSXP) => cx.new_env(cx.empty_env(), false, 0)?,
        Core::Other(t) => {
            // a node retyped in the raw layer (C1 makes closures; the
            // registry's builtin objects are rt's): its fields are nil
            let f = cx.lcons(cx.nil(), cx.nil())?;
            unsafe { f.sexp().set_typ(*t) };
            f
        }
        Core::Lang(x) => {
            let mut l = cx.nil();
            for (k, e) in x.iter().enumerate().rev() {
                let r = make(cx, e)?;
                l = if k == 0 { cx.lcons(r, l)? } else { cx.cons(r, l)? };
            }
            l
        }
    })
}

/// `enc(x)` of helpers.R.
fn enc(cx: &Cx, x: Ref<'_>) -> String {
    let join = |v: Vec<String>, sep: &str| v.join(sep);
    let bits = |d: f64| format!("{:016x}", d.to_bits());
    let mut s = match x.view() {
        View::Null => "NULL".to_owned(),
        View::Lgl(v) => format!(
            "lgl[{}]",
            join(
                v.as_slice()
                    .unwrap()
                    .iter()
                    .map(|l| match l.0 {
                        i32::MIN => "NA".into(),
                        0 => "F".into(),
                        _ => "T".into(),
                    })
                    .collect(),
                ","
            )
        ),
        View::Int(v) => format!(
            "int[{}]",
            join(
                v.as_slice()
                    .unwrap()
                    .iter()
                    .map(|&i| if i == i32::MIN { "NA".into() } else { i.to_string() })
                    .collect(),
                ","
            )
        ),
        View::Real(v) => format!("dbl[{}]", join(v.as_slice().unwrap().iter().map(|&d| bits(d)).collect(), ",")),
        View::Cplx(v) => format!(
            "cplx[{}]",
            join(v.as_slice().unwrap().iter().map(|z| format!("{}/{}", bits(z.r), bits(z.i))).collect(), ",")
        ),
        View::Raw(v) => {
            format!("raw[{}]", join(v.as_slice().unwrap().iter().map(|b| format!("{b:02x}")).collect(), ","))
        }
        View::Str(v) => format!(
            "chr[{}]",
            join(
                v.as_slice()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        if *c == cx.na_string() {
                            "NA".into()
                        } else {
                            let e = match c.enc() {
                                Encoding::Native => 'n',
                                Encoding::Utf8 => 'u',
                                Encoding::Latin1 => 'l',
                                Encoding::Bytes => 'b',
                            };
                            format!("x{}.{e}", hex(c.bytes()))
                        }
                    })
                    .collect(),
                ","
            )
        ),
        View::List(v) => format!("list[{}]", join(v.as_slice().unwrap().iter().map(|&e| enc(cx, e)).collect(), ";")),
        View::Sym(s) => format!("sym({})", hex(s.name().bytes())),
        View::Lang(c) => {
            let mut v = Vec::new();
            let mut at = Some(c);
            while let Some(n) = at {
                v.push(enc(cx, n.car()));
                at = n.cdr().as_cons();
            }
            format!("lang({})", v.join(";"))
        }
        _ => format!("<{}>", x.sexptype()),
    };
    if !matches!(x.sexptype(), SYMSXP | LANGSXP | NILSXP) {
        let a: Vec<String> = robigo_rtbuiltins::attrib::attrs(x)
            .map(|a| format!("{}={}", hex(a.tag.name().bytes()), enc(cx, a.value)))
            .collect();
        if !a.is_empty() {
            s.push_str(&format!("{{{}}}", a.join(";")));
        }
    }
    s
}

// ---- the cases --------------------------------------------------------------

struct Case {
    tag: String,
    call: String,
    inputs: Vec<(String, String)>,
    want: Vec<String>,
}

fn cases(text: &str) -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    for line in text.lines() {
        let (tag, rest) = line.split_once(' ').unwrap();
        if out.last().is_none_or(|c| c.tag != tag) {
            out.push(Case { tag: tag.to_owned(), call: String::new(), inputs: Vec::new(), want: Vec::new() });
        }
        let c = out.last_mut().unwrap();
        if let Some(v) = rest.strip_prefix("call : ") {
            c.call = v.to_owned();
        } else if let Some(v) = rest.strip_prefix("in ") {
            let (n, v) = v.split_once(" : ").unwrap();
            c.inputs.push((n.to_owned(), v.to_owned()));
        } else {
            c.want.push(rest.to_owned());
        }
    }
    out
}

/// A dispatcher with no methods: every internal generic runs its default.
fn no_methods<'o>(
    _: &mut Cx,
    _: Generic,
    _: Root<'_>,
    _: Args<'_>,
    _: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    Ok(Dispatched::Default(out))
}

/// Run one case: the outcome lines in the recorded format.
fn run(cx: &mut Cx, case: &Case) -> Vec<String> {
    let call_v = decode(&case.call);
    let Core::Lang(parts) = &call_v.core else { panic!("{}: not a call", case.tag) };
    let Core::Sym(fname) = &parts[0].core else { panic!("{}: no function name", case.tag) };
    let fname = String::from_utf8(fname.clone()).unwrap();
    let b = registry::by_name(&fname).unwrap_or_else(|| panic!("{}: no builtin {fname}", case.tag));
    let inputs: Vec<(String, V)> = case.inputs.iter().map(|(n, v)| (n.clone(), decode(v))).collect();
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let call = o.root();
        cx.build(o, |cx| make(cx, &call_v)).unwrap();
        let mut roots = Vec::new();
        for a in &parts[1..] {
            let Core::Sym(n) = &a.core else { panic!("{}: an argument that is not a symbol", case.tag) };
            let v = &inputs.iter().find(|(m, _)| m.as_bytes() == n.as_slice()).unwrap().1;
            let o = s.out(cx);
            roots.push(o.root());
            cx.build(o, |cx| make(cx, v)).unwrap();
        }
        let refs: Vec<Ref<'_>> = roots.iter().map(|r| r.bind(cx)).collect();
        let args = s.args(cx, &refs, None);
        let env = s.root(cx, cx.global_env());
        let res = s.out(cx);
        let res_root = res.root();
        let r = (b.fun)(cx, call, args, env, res);
        let who = |cx: &Cx, c: Ref<'_>| {
            if c.sexp() == call.bind(cx).sexp() {
                "call"
            } else if c.sexptype() == NILSXP {
                "ctx"
            } else {
                "other"
            }
        };
        let mut lines: Vec<String> =
            cx.collected_warnings().iter().map(|(c, m)| format!("W : {} {}", hex(m.bytes()), who(cx, *c))).collect();
        cx.clear_warnings();
        match r {
            Ok(()) => lines.push(format!("= {}", enc(cx, res_root.bind(cx)))),
            Err(e) => {
                let w = who(cx, cx.exit_call(&e)).to_owned();
                let o = s.out(cx);
                let m = o.root();
                cx.resolve_exit(e, Some(o));
                let msg = m.bind(cx).as_vector::<Char<'_>>().map(|v| v.get(0).bytes().to_vec()).unwrap_or_default();
                lines.push(format!("E : {} {w}", hex(&msg)));
            }
        }
        lines
    })
}

/// Replay every case of `name.out`; the C-collation cases (tags starting
/// "C.") run with `LC_COLLATE=C`, as the script sets it. `not_yet` lists
/// the cases robigo answers with its "not implemented yet" error instead
/// (AGENTS rule 6): exactly those, so a new gap fails as surely as a wrong
/// value, and a closed one asks for its removal.
fn replay(name: &str, text: &str, not_yet: &[&str]) {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() });
    let cx = iso.cx();
    cx.install_hooks(Hooks { dispatch: no_methods, ..Hooks::default() });
    let all = cases(text);
    let mut bad = Vec::new();
    let mut gaps = Vec::new();
    // "robigo: " ... " is not implemented yet", hex-encoded in an E line
    let (head, tail) = (hex(b"robigo: "), hex(b" is not implemented yet"));
    for c in &all {
        let c_collate = c.tag.starts_with("C.");
        cx.set_locale(Locale { collate_c: c_collate, ..Locale::default() });
        let got = run(cx, c);
        let gap = got.last().is_some_and(|l| l.starts_with(&format!("E : {head}")) && l.contains(&tail));
        if gap {
            gaps.push(c.tag.clone());
        } else if got != c.want {
            bad.push(format!("{}\n  call {}\n  want {:?}\n  got  {:?}", c.tag, c.call, c.want, got));
        }
    }
    assert_eq!(gaps, not_yet, "{name}: the cases robigo does not implement yet");
    assert!(
        bad.is_empty(),
        "{name}: {} of {} cases differ from GNU R 4.6.2:\n{}",
        bad.len(),
        all.len(),
        bad.iter().take(400).cloned().collect::<Vec<_>>().join("\n")
    );
    eprintln!("{name}: {} cases match", all.len());
}

#[test]
fn arithmetic_matches_gnu_r() {
    replay("arith", include_str!("oracle/arith.out"), &[]);
}

#[test]
fn comparison_matches_gnu_r() {
    replay("relop", include_str!("oracle/relop.out"), &["bad.list"]);
}

#[test]
fn logic_matches_gnu_r() {
    replay("logic", include_str!("oracle/logic.out"), &[]);
}

#[test]
fn paste_matches_gnu_r() {
    replay("paste", include_str!("oracle/paste.out"), &[]);
}

#[test]
fn nchar_matches_gnu_r() {
    replay("nchar", include_str!("oracle/nchar.out"), &[]);
}
