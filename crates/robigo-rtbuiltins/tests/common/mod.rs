//! The oracle encoding of `tests/oracle/helpers.R`: decoding a value into
//! fresh objects and encoding one, shared by the oracle test files.

// Tests make functions in the raw layer (their constructors are C1's and
// rt's).
#![allow(clippy::undocumented_unsafe_blocks, dead_code)]

use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, Encoding, Lgl, NeedGc, Rcomplex, Ref, View};

// ---- the encoding -----------------------------------------------------------

/// A decoded value.
#[derive(Clone, Debug)]
pub struct V {
    pub core: Core,
    pub attrs: Vec<(Vec<u8>, V)>,
}

#[derive(Clone, Debug)]
pub enum Core {
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

pub fn hex(b: &[u8]) -> String {
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

pub fn decode(s: &str) -> V {
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
pub fn make<'g>(cx: &'g Cx, v: &V) -> Result<Ref<'g>, NeedGc> {
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
pub fn enc(cx: &Cx, x: Ref<'_>) -> String {
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
        _ => format!("<{}>", robigo_rtbuiltins::util::type2char(x.sexptype())),
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
