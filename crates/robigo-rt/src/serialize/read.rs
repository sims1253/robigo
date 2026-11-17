//! `R_Unserialize`, `ReadItem` and `R_SerializeInfo`, ported.
//!
//! Every object under construction is rooted (GNU R's `PROTECT`s, here
//! one scope per level of the read), and every raw `Sexp` is re-read from
//! its root after anything that may collect: an allocation the heap
//! refuses collects and is retried, as GNU R's allocation collects, and
//! the objects built so far survive it (moved, under `ROBIGO_GC_MOVE=1`).
//! So an injected allocation failure (E9, `ROBIGO_FAIL_ALLOC_AT`) changes
//! nothing, and no object is visible to R code before it is complete: the
//! hooks (`PERSISTSXP`'s restore, `findPackageEnv`, `..getNamespace`) see
//! only the strings read for them, and an error leaves the partial objects
//! to the collector.
//!
//! An environment is registered in the reference table before its
//! contents are read (its own closures refer to it), and robigo's hashed
//! and unhashed environments have different shapes, so the reader looks
//! ahead in the stream ([`Reader::peek_env_hashed`]: the enclosure is
//! skipped syntactically, [`Reader::skip_item`]) to make the right shell,
//! and robigo-rtcore's `restore_env` fills it (`env/restore.rs`).

#![allow(unsafe_code)]

use super::stream::InStream;
use super::*;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Out, RResult, Ref, Root, Scope};

/// `INITIAL_REFREAD_TABLE_SIZE`.
const INITIAL_REFREAD_TABLE_SIZE: usize = 128;
/// `R_CODESET_MAX`.
const R_CODESET_MAX: i32 = 63;
const MAXIDSIZE: usize = 10000;

/// A codeset the reader can convert from and to (GNU R's `Riconv` on
/// glibc converts between any; robigo has these).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Codeset {
    Utf8,
    Latin1,
    Ascii,
}

fn codeset(name: &[u8]) -> Option<Codeset> {
    let n = String::from_utf8_lossy(name).to_ascii_uppercase();
    match n.as_str() {
        "UTF-8" | "UTF8" => Some(Codeset::Utf8),
        "ISO-8859-1" | "ISO8859-1" | "LATIN1" | "ISO_8859-1" => Some(Codeset::Latin1),
        "ANSI_X3.4-1968" | "ASCII" | "US-ASCII" | "646" => Some(Codeset::Ascii),
        _ => None,
    }
}

/// `Riconv` of `b` from `from` to `to`: None where it fails (an invalid or
/// unrepresentable byte sequence).
fn convert(from: Codeset, to: Codeset, b: &[u8]) -> Option<Vec<u8>> {
    let chars: Vec<char> = match from {
        Codeset::Utf8 => std::str::from_utf8(b).ok()?.chars().collect(),
        Codeset::Latin1 => b.iter().map(|&c| c as char).collect(),
        Codeset::Ascii => {
            if !b.is_ascii() {
                return None;
            }
            b.iter().map(|&c| c as char).collect()
        }
    };
    match to {
        Codeset::Utf8 => Some(chars.iter().collect::<String>().into_bytes()),
        Codeset::Latin1 => chars.iter().map(|&c| u8::try_from(c as u32).ok()).collect(),
        Codeset::Ascii => chars.iter().map(|&c| if c.is_ascii() { Some(c as u8) } else { None }).collect(),
    }
}

/// `reEnc3(buf, from, to, 1)`: converted, with `<xx>` for each byte that
/// does not convert.
fn re_enc3(b: &[u8], from: Codeset, to: Codeset) -> Vec<u8> {
    if let Some(v) = convert(from, to, b) {
        return v;
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let n = if from == Codeset::Utf8 { utf8_len(&b[i..]) } else { Some(1) };
        match n.and_then(|n| convert(from, to, &b[i..i + n])) {
            Some(v) => {
                out.extend_from_slice(&v);
                i += n.unwrap();
            }
            None => {
                out.extend_from_slice(format!("<{:02x}>", b[i]).as_bytes());
                i += 1;
            }
        }
    }
    out
}

fn utf8_len(b: &[u8]) -> Option<usize> {
    let n = match b[0] {
        0..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    (b.len() >= n && std::str::from_utf8(&b[..n]).is_ok()).then_some(n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Conv {
    /// not opened yet (`NULL`)
    Unopened,
    /// `(void *)-1`: impossible or not needed
    Off,
    Open(Codeset, Codeset),
}

pub(super) struct Reader<'h, 'r> {
    pub inp: InStream,
    refs: Root<'r>,
    nrefs: usize,
    hook: Option<Root<'h>>,
    /// `R_ReadItemDepth` (`R_InitReadItemDepth` is 0).
    depth: i32,
    lastname: Vec<u8>,
    native: Vec<u8>,
    nat2nat: Conv,
    nat2utf8: Conv,
}

fn unpack(flags: i32) -> (i32, u16, bool, bool, bool) {
    (
        flags & 255,
        (flags >> 12) as u16,
        flags & IS_OBJECT_BIT_MASK != 0,
        flags & HAS_ATTR_BIT_MASK != 0,
        flags & HAS_TAG_BIT_MASK != 0,
    )
}

/// The object `x` as a `Ref` of this interval.
///
/// # Safety
/// `x` is live: reachable from a root, or fresh with nothing collected
/// since it was made.
#[inline(always)]
unsafe fn r(cx: &Cx, x: Sexp) -> Ref<'_> {
    // SAFETY: the caller's contract.
    unsafe { Ref::from_sexp(x, cx) }
}

/// GNU R's `DecodeVersion`.
fn decode_version(packed: i32) -> (i32, i32, i32) {
    (packed / 65536, (packed % 65536) / 256, packed % 256)
}

/// `R_Unserialize(stream)`: the object, written to `out`.
// R: serialize.c:R_Unserialize
pub(super) fn unserialize(cx: &mut Cx, inp: InStream, hook: Option<Root<'_>>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let refs = s.out(cx);
        let refs_root = refs.root();
        cx.build(refs, |cx| Ok(cx.alloc_vector::<Ref<'_>>(INITIAL_REFREAD_TABLE_SIZE)?.finish().into()))?;
        let mut rd = Reader {
            inp,
            refs: refs_root,
            nrefs: 0,
            hook,
            depth: 0,
            lastname: b"<unknown>".to_vec(),
            native: Vec::new(),
            nat2nat: Conv::Unopened,
            nat2utf8: Conv::Unopened,
        };
        rd.in_format(cx)?;
        let version = rd.int(cx)?;
        let writer_version = rd.int(cx)?;
        let min_reader_version = rd.int(cx)?;
        match version {
            2 => {}
            3 => {
                let nelen = rd.int(cx)?;
                if !(0..=R_CODESET_MAX).contains(&nelen) {
                    return Err(cx.error("invalid length of encoding name"));
                }
                rd.native = rd.string(cx, nelen as usize)?;
            }
            _ => {
                let (vw, pw, sw) = decode_version(writer_version);
                if min_reader_version < 0 {
                    return Err(cx.error(&format!(
                        "cannot read unreleased workspace version {version} written by experimental R {vw}.{pw}.{sw}"
                    )));
                }
                let (vm, pm, sm) = decode_version(min_reader_version);
                return Err(cx.error(&format!(
                    "cannot read workspace version {version} written by R {vw}.{pw}.{sw}; need R {vm}.{pm}.{sm} or newer"
                )));
            }
        }
        let v = rd.read_item(cx)?;
        // SAFETY: fresh or reachable from the reference table, and nothing
        // has collected since it was returned.
        out.set(cx, unsafe { r(cx, v) });
        Ok(())
    })
}

/// `R_SerializeInfo(stream)`: the header as a named list.
// R: serialize.c:R_SerializeInfo
pub(super) fn serialize_info(cx: &mut Cx, mut inp: InStream, out: Out<'_>) -> RResult<()> {
    let e = |cx: &mut Cx, m: String| cx.error(&m);
    in_format(cx, &mut inp)?;
    let version = inp.integer(cx).map_err(|m| e(cx, m))?;
    let writer_version = inp.integer(cx).map_err(|m| e(cx, m))?;
    let min_reader_version = inp.integer(cx).map_err(|m| e(cx, m))?;
    let fmt = match inp.fmt {
        Format::Ascii | Format::AsciiHex => "ascii",
        Format::Binary => "binary",
        Format::Xdr => "xdr",
        Format::Any => return Err(cx.error("unknown input format")),
    };
    let native = if version == 3 {
        let nelen = inp.integer(cx).map_err(|m| e(cx, m))?;
        if !(0..=R_CODESET_MAX).contains(&nelen) {
            return Err(cx.error("invalid length of encoding name"));
        }
        Some(inp.string(cx, nelen as usize).map_err(|m| e(cx, m))?)
    } else {
        None
    };
    let (vv, vp, vs) = decode_version(writer_version);
    let wv = format!("{vv}.{vp}.{vs}");
    let mv = (min_reader_version >= 0).then(|| {
        let (a, b, c) = decode_version(min_reader_version);
        format!("{a}.{b}.{c}")
    });
    cx.build(out, |cx| {
        let n = if native.is_some() { 5 } else { 4 };
        let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
        let mut names = cx.alloc_vector::<robigo_rtcore::Char<'_>>(n)?;
        names.set(0, cx.mk_char(b"version", Encoding::Native)?);
        ans.set(0, cx.alloc_copy(&[version])?.finish().into());
        names.set(1, cx.mk_char(b"writer_version", Encoding::Native)?);
        ans.set(1, cx_string(cx, wv.as_bytes())?);
        names.set(2, cx.mk_char(b"min_reader_version", Encoding::Native)?);
        ans.set(
            2,
            match &mv {
                Some(m) => cx_string(cx, m.as_bytes())?,
                None => {
                    let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
                    v.set(0, cx.na_string());
                    v.finish().into()
                }
            },
        );
        names.set(3, cx.mk_char(b"format", Encoding::Native)?);
        ans.set(3, cx_string(cx, fmt.as_bytes())?);
        if let Some(nat) = &native {
            names.set(4, cx.mk_char(b"native_encoding", Encoding::Native)?);
            ans.set(4, cx_string(cx, nat)?);
        }
        let mut a = robigo_rtbuiltins::attrib::Attrs::new();
        a.install(robigo_rtbuiltins::attrib::Attr {
            tag: cx.sym(robigo_rtcore::RSym::Names),
            value: names.finish().into(),
        });
        a.apply(cx, &mut ans)?;
        Ok(ans.finish().into())
    })
}

fn cx_string<'g>(cx: &'g Cx, b: &[u8]) -> Result<Ref<'g>, robigo_heap::NeedGc> {
    cx.mk_string(b, Encoding::Native)
}

/// `InFormat(stream)`.
// R: serialize.c:InFormat
fn in_format(cx: &mut Cx, inp: &mut InStream) -> RResult<()> {
    let b = match inp.in_bytes(cx, 2) {
        Ok(b) => [b[0], b[1]],
        Err(m) => return Err(cx.error(&m)),
    };
    let ty = match b[0] {
        b'A' => Format::Ascii,
        b'B' => Format::Binary,
        b'X' => Format::Xdr,
        b'\n' if b[1] == b'A' => {
            if let Err(m) = inp.in_bytes(cx, 1) {
                return Err(cx.error(&m));
            }
            Format::Ascii
        }
        _ => return Err(cx.error("unknown input format")),
    };
    if inp.fmt == Format::Any {
        inp.fmt = ty;
    } else if !(ty == inp.fmt || (ty == Format::Ascii && inp.fmt == Format::AsciiHex)) {
        return Err(cx.error("input format does not match specified format"));
    }
    Ok(())
}

impl<'r> Reader<'_, 'r> {
    fn in_format(&mut self, cx: &mut Cx) -> RResult<()> {
        in_format(cx, &mut self.inp)
    }

    #[inline]
    fn int(&mut self, cx: &mut Cx) -> RResult<i32> {
        match self.inp.integer(cx) {
            Ok(v) => Ok(v),
            Err(m) => Err(cx.error(&m)),
        }
    }

    fn string(&mut self, cx: &mut Cx, n: usize) -> RResult<Vec<u8>> {
        match self.inp.string(cx, n) {
            Ok(v) => Ok(v),
            Err(m) => Err(cx.error(&m)),
        }
    }

    /// `ReadItem`: the next object, raw (rooted by nothing: the caller
    /// stores or roots it before anything else may collect).
    pub(super) fn read_item(&mut self, cx: &mut Cx) -> RResult<Sexp> {
        let flags = self.int(cx)?;
        self.read_item_recursive(cx, flags)
    }

    /// `ReadItem` one level down (`R_ReadItemDepth++` around it).
    fn read_deeper(&mut self, cx: &mut Cx) -> RResult<Sexp> {
        self.depth += 1;
        let v = self.read_item(cx);
        self.depth -= 1;
        v
    }

    // R: serialize.c:GetReadRef
    fn get_read_ref(&mut self, cx: &mut Cx, index: i32) -> RResult<Sexp> {
        let i = index as i64 - 1;
        // SAFETY: the rooted table (a VECSXP).
        let len = unsafe { self.refs.bind(cx).sexp().std_len() } as i64;
        if i < 0 || i >= len {
            return Err(cx.error("reference index out of range"));
        }
        // SAFETY: as above, `i` in range.
        Ok(unsafe { *(self.refs.bind(cx).sexp().std_data() as *const Sexp).add(i as usize) })
    }

    /// `AddReadRef(table, value)`, for `value` rooted by the caller.
    // R: serialize.c:AddReadRef
    fn add_read_ref(&mut self, cx: &mut Cx, value: Root<'_>) -> RResult<()> {
        let count = self.nrefs + 1;
        // SAFETY: the rooted table.
        let len = unsafe { self.refs.bind(cx).sexp().std_len() } as usize;
        if count >= len {
            let refs = self.refs;
            refs.replace_with(cx, |cx| {
                let old = refs.bind(cx);
                let mut v = cx.alloc_vector::<Ref<'_>>(2 * count)?;
                let xs = old.as_vector::<Ref<'_>>().unwrap();
                for i in 0..len {
                    v.set(i, xs.get(i));
                }
                Ok(v.finish().into())
            })?;
        }
        self.nrefs = count;
        let t = self.refs.bind(cx).sexp();
        let v = value.bind(cx).sexp();
        // SAFETY: the rooted table, `count - 1` within it; the store goes
        // through the barrier.
        unsafe { t.set_elt(count - 1, v) };
        Ok(())
    }

    // R: serialize.c:InRefIndex
    fn in_ref_index(&mut self, cx: &mut Cx, flags: i32) -> RResult<i32> {
        let i = flags >> 8;
        if i == 0 { self.int(cx) } else { Ok(i) }
    }

    // R: serialize.c:ReadLENGTH
    fn read_length(&mut self, cx: &mut Cx) -> RResult<usize> {
        let len = self.int(cx)?;
        if len < -1 {
            return Err(cx.error("negative serialized length for vector"));
        }
        if len == -1 {
            let len1 = self.int(cx)? as u32;
            let len2 = self.int(cx)? as u32;
            if len1 > 65536 {
                return Err(cx.error("invalid upper part of serialized vector length"));
            }
            return Ok(((len1 as usize) << 32) + len2 as usize);
        }
        Ok(len as usize)
    }

    /// `InStringVec(stream, ref_table)`, rooted in `s`.
    // R: serialize.c:InStringVec
    fn in_string_vec<'s>(&mut self, cx: &mut Cx, s: Scope<'s>) -> RResult<Root<'s>> {
        if self.int(cx)? != 0 {
            return Err(cx.error("names in persistent strings are not supported yet"));
        }
        let len = self.int(cx)?;
        let len = usize::try_from(len).map_err(|_| cx.error("negative length vectors are not allowed"))?;
        let o = s.out(cx);
        let v = o.root();
        cx.build(o, |cx| Ok(cx.alloc_vector::<robigo_rtcore::Char<'_>>(len)?.finish().into()))?;
        self.depth += 1;
        for i in 0..len {
            let c = self.read_item(cx)?;
            self.set_string_elt(cx, v, i, c)?;
        }
        self.depth -= 1;
        Ok(v)
    }

    /// `SET_STRING_ELT(x, i, c)` with GNU R's check on `c`.
    fn set_string_elt(&mut self, cx: &mut Cx, x: Root<'_>, i: usize, c: Sexp) -> RResult<()> {
        // SAFETY: `c` was just read (nothing collected since); `x` rooted.
        unsafe {
            if c.typ() != CHARSXP {
                let t = robigo_rtbuiltins::util::type2char(c.typ());
                return Err(cx.error(&format!("Value of SET_STRING_ELT() must be a 'CHARSXP' not a '{t}'")));
            }
            x.bind(cx).sexp().set_elt(i, c);
        }
        Ok(())
    }

    /// `CallHook(x, fun)` with `x` rooted: `fun(x)` evaluated in the global
    /// environment.
    // R: serialize.c:CallHook
    fn call_fn<'s>(&mut self, cx: &mut Cx, s: Scope<'s>, fun: Root<'_>, x: Root<'_>) -> RResult<Root<'s>> {
        let c = s.out(cx);
        let call = c.root();
        cx.build(c, |cx| cx.lang2(fun.bind(cx), x.bind(cx)))?;
        let o = s.out(cx);
        let v = o.root();
        let g = s.root(cx, cx.global_env());
        cx.eval(call, g, o)?;
        Ok(v)
    }

    /// `name(args...)` evaluated in `env`.
    fn call_named<'s>(
        &mut self,
        cx: &mut Cx,
        s: Scope<'s>,
        name: &[u8],
        args: &[Root<'_>],
        base: bool,
    ) -> RResult<Root<'s>> {
        let f = cx.install_rooted(s, name)?;
        let c = s.out(cx);
        let call = c.root();
        cx.build(c, |cx| {
            let mut l = cx.nil();
            for a in args.iter().rev() {
                l = cx.cons(a.bind(cx), l)?;
            }
            cx.lcons(f.bind(cx), l)
        })?;
        let o = s.out(cx);
        let v = o.root();
        let env = s.root(cx, if base { cx.base_env() } else { cx.global_env() });
        cx.eval(call, env, o)?;
        Ok(v)
    }

    // R: serialize.c:ReadItem_Recursive
    fn read_item_recursive(&mut self, cx: &mut Cx, flags: i32) -> RResult<Sexp> {
        let (t, levs, objf, hasattr, _) = unpack(flags);
        match t {
            NILVALUE_SXP => return Ok(cx.nil().sexp()),
            EMPTYENV_SXP => return Ok(cx.empty_env().sexp()),
            BASEENV_SXP => return Ok(cx.base_env().sexp()),
            GLOBALENV_SXP => return Ok(cx.global_env().sexp()),
            UNBOUNDVALUE_SXP => return Ok(cx.unbound().sexp()),
            MISSINGARG_SXP => return Ok(cx.missing_arg().sexp()),
            BASENAMESPACE_SXP => return Ok(cx.base_namespace().sexp()),
            REFSXP => {
                let i = self.in_ref_index(cx, flags)?;
                return self.get_read_ref(cx, i);
            }
            PERSISTSXP => {
                return cx.scope(|cx, s| {
                    let v = self.in_string_vec(cx, s)?;
                    let Some(hook) = self.hook else {
                        return Err(cx.error("no restore method available"));
                    };
                    let x = self.call_fn(cx, s, hook, v)?;
                    self.add_read_ref(cx, x)?;
                    Ok(x.bind(cx).sexp())
                });
            }
            ALTREP_SXP => return self.altrep(cx, levs, objf),
            PACKAGESXP | NAMESPACESXP => {
                return cx.scope(|cx, s| {
                    let v = self.in_string_vec(cx, s)?;
                    let x = if t == PACKAGESXP {
                        // R_FindPackageEnv: findPackageEnv(info) in base
                        self.call_named(cx, s, b"findPackageEnv", &[v], true)?
                    } else {
                        // R_FindNamespace1: ..getNamespace(info, lastname) in base
                        let name = self.lastname.clone();
                        let o = s.out(cx);
                        let w = o.root();
                        cx.build(o, |cx| cx.mk_string(&name, Encoding::Native))?;
                        self.call_named(cx, s, b"..getNamespace", &[v, w], true)?
                    };
                    self.add_read_ref(cx, x)?;
                    Ok(x.bind(cx).sexp())
                });
            }
            _ => {}
        }
        let t = match u8::try_from(t) {
            Ok(t) if t <= OBJSXP => t,
            _ => {
                return match t {
                    CLASSREFSXP => Err(cx.error("this version of R cannot read class references")),
                    GENERICREFSXP => Err(cx.error("this version of R cannot read generic function references")),
                    _ => Err(cx.error(&format!("ReadItem: unknown type {t}, perhaps written by later version of R"))),
                };
            }
        };
        match t {
            SYMSXP => cx.scope(|cx, s| {
                let pn = self.read_deeper(cx)?;
                // SAFETY: just read.
                let pn = s.root(cx, unsafe { r(cx, pn) });
                let sym = self.install_tr_char(cx, s, pn)?;
                self.add_read_ref(cx, sym)?;
                Ok(sym.bind(cx).sexp())
            }),
            ENVSXP => self.env(cx),
            LISTSXP | LANGSXP | CLOSXP | PROMSXP | DOTSXP => self.read_iterative(cx, flags),
            _ => self.read_other(cx, t, levs, objf, hasattr),
        }
    }

    /// `installTrChar(pn)` with GNU R's name checks.
    // R: sysutils.c:installTrChar, names.c:installNoTrChar
    fn install_tr_char<'s>(&mut self, cx: &mut Cx, s: Scope<'s>, pn: Root<'_>) -> RResult<Root<'s>> {
        let c = pn.bind(cx);
        let Some(ch) = c.as_char() else {
            let t = robigo_rtbuiltins::util::type_to_char(c);
            return Err(cx.error(&format!("'installTrChar' must be called on a CHARSXP, but got '{t}'")));
        };
        let name = match robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(ch), cx.locale()) {
            Ok(b) => b.into_owned(),
            Err(_) => {
                return Err(
                    cx.error("robigo: a symbol name not representable in the native encoding is not implemented yet")
                );
            }
        };
        if name.is_empty() {
            return Err(cx.error("attempt to use zero-length variable name"));
        }
        if name.len() > MAXIDSIZE {
            return Err(cx.error(&format!("variable names are limited to {MAXIDSIZE} bytes")));
        }
        cx.install_rooted(s, &name)
    }

    /// `ReadItem_Iterative`: a run of pairlist-like cells (LISTSXP,
    /// LANGSXP, CLOSXP, PROMSXP, DOTSXP), each the CDR of the one before.
    // R: serialize.c:ReadItem_Iterative
    fn read_iterative(&mut self, cx: &mut Cx, mut flags: i32) -> RResult<Sexp> {
        cx.scope(|cx, s| {
            let first = s.out(cx).root();
            let last = s.out(cx).root();
            let mut have_first = false;
            let nil = cx.nil().sexp();
            loop {
                let (t, levs, objf, hasattr, hastag) = unpack(flags);
                let t = t as u8;
                // SAFETY: nil is immortal; the cell is fresh and rooted
                // before anything else allocates.
                let cell = cx.retry_alloc(|cx| unsafe { cx.alloc_node_raw(t, nil, nil, nil) })?;
                // SAFETY: fresh; nothing collected since.
                unsafe {
                    cell.set_gp(levs);
                    cell.set_object(objf);
                }
                if !have_first {
                    // SAFETY: fresh.
                    first.replace_with(cx, |cx| Ok(unsafe { r(cx, cell) }))?;
                    last.replace_from(cx, first);
                    have_first = true;
                } else {
                    let prev = last.bind(cx).sexp();
                    // SAFETY: `prev` rooted, `cell` fresh: the list's next
                    // cell, now reachable from `first`.
                    unsafe { prev.set_cdr(cell) };
                    // SAFETY: `cell` is reachable from `first` now.
                    last.replace_with(cx, |cx| Ok(unsafe { r(cx, cell) }))?;
                }
                self.depth += 1;
                let mut set_lastname = false;
                if hasattr {
                    let a = self.read_item(cx)?;
                    // SAFETY: rooted cell, just-read value.
                    unsafe { last.bind(cx).sexp().set_attrib(a) };
                }
                if hastag {
                    let tag = self.read_item(cx)?;
                    // SAFETY: as above.
                    unsafe {
                        last.bind(cx).sexp().set_tag(tag);
                        if self.depth == 1 && tag.typ() == SYMSXP {
                            self.lastname = tag.car().char_bytes().to_vec();
                            set_lastname = true;
                        }
                    }
                }
                let car = self.read_item(cx)?;
                // SAFETY: as above.
                unsafe { last.bind(cx).sexp().set_car(car) };
                self.depth -= 1;
                cx.check_c_stack()?;
                // SAFETY: the rooted cell.
                unsafe {
                    let c = last.bind(cx).sexp();
                    if t == CLOSXP && c.tag() == nil {
                        c.set_tag(cx.base_env().sexp());
                    } else if t == PROMSXP && c.tag() == nil {
                        // (a forced promise's PRENV is R_NilValue, which
                        // GNU R's reader also replaces)
                        c.set_tag(cx.base_env().sexp());
                    }
                }
                if set_lastname {
                    self.lastname = b"<unknown>".to_vec();
                }
                flags = self.int(cx)?;
                if !matches!((flags & 255) as u8, LISTSXP | LANGSXP | CLOSXP | PROMSXP | DOTSXP) || flags & 255 > 25 {
                    break;
                }
            }
            self.depth += 1;
            let v = self.read_item_recursive(cx, flags);
            self.depth -= 1;
            let v = v?;
            // SAFETY: rooted cell, just-read value.
            unsafe { last.bind(cx).sexp().set_cdr(v) };
            Ok(first.bind(cx).sexp())
        })
    }

    /// An environment (`ENVSXP`): registered before its contents are read.
    // R: serialize.c:ReadItem_Recursive (ENVSXP)
    fn env(&mut self, cx: &mut Cx) -> RResult<Sexp> {
        let locked = self.int(cx)?;
        let hashed = self.peek_env_hashed(cx);
        cx.scope(|cx, s| {
            let o = s.out(cx);
            let env = o.root();
            cx.build(o, |cx| cx.new_env(cx.nil(), hashed, 0))?;
            self.add_read_ref(cx, env)?;
            self.depth += 1;
            let mut parts = Vec::with_capacity(4);
            for _ in 0..4 {
                let v = self.read_item(cx)?;
                // SAFETY: just read.
                parts.push(s.root(cx, unsafe { r(cx, v) }));
            }
            self.depth -= 1;
            let (enclos, frame, table, attrib) = (parts[0], parts[1], parts[2], parts[3]);
            cx.restore_env(env, frame, table)?;
            let a = attrib.bind(cx).sexp();
            // SAFETY: the rooted environment and attributes.
            unsafe {
                let e = env.bind(cx).sexp();
                e.set_attrib(a);
                if a != cx.nil().sexp() && has_class(cx, a) {
                    e.set_object(true);
                }
            }
            if locked != 0 {
                cx.lock_environment(env, false)?;
            }
            let enclos = if enclos.bind(cx).sexp() == cx.nil().sexp() { s.root(cx, cx.base_env()) } else { enclos };
            if enclos.bind(cx).sexptype() != ENVSXP {
                return Err(cx.error(
                    "robigo: an environment's serialized enclosure that is not an environment is not implemented yet",
                ));
            }
            cx.set_enclos(env, enclos);
            Ok(env.bind(cx).sexp())
        })
    }

    /// Whether the environment whose contents follow is hashed: its
    /// enclosure skipped, its FRAME R_NilValue and its HASHTAB not.
    fn peek_env_hashed(&mut self, cx: &Cx) -> bool {
        let m = self.inp.mark();
        let r = (|| -> Result<bool, String> {
            self.skip_item(cx)?;
            let f = self.inp.integer(cx)?;
            if f & 255 != NILVALUE_SXP {
                return Ok(false);
            }
            let h = self.inp.integer(cx)?;
            Ok(h & 255 != NILVALUE_SXP)
        })();
        self.inp.reset(m);
        r.unwrap_or(false)
    }

    /// Skip an item: the stream's grammar, building nothing.
    pub(super) fn skip_item(&mut self, cx: &Cx) -> Result<(), String> {
        let flags = self.inp.integer(cx)?;
        self.skip_flags(cx, flags)
    }

    fn skip_flags(&mut self, cx: &Cx, mut flags: i32) -> Result<(), String> {
        loop {
            let (t, _, _, hasattr, hastag) = unpack(flags);
            match t {
                NILVALUE_SXP | EMPTYENV_SXP | BASEENV_SXP | GLOBALENV_SXP | UNBOUNDVALUE_SXP | MISSINGARG_SXP
                | BASENAMESPACE_SXP => return Ok(()),
                REFSXP => {
                    if flags >> 8 == 0 {
                        self.inp.integer(cx)?;
                    }
                    return Ok(());
                }
                PERSISTSXP | PACKAGESXP | NAMESPACESXP => return self.skip_string_vec(cx),
                ALTREP_SXP => {
                    for _ in 0..3 {
                        self.skip_item(cx)?;
                    }
                    return Ok(());
                }
                _ => {}
            }
            let Ok(t) = u8::try_from(t) else { return Err("unknown type".into()) };
            match t {
                SYMSXP => return self.skip_item(cx),
                ENVSXP => {
                    self.inp.integer(cx)?;
                    for _ in 0..4 {
                        self.skip_item(cx)?;
                    }
                    return Ok(());
                }
                LISTSXP | LANGSXP | CLOSXP | PROMSXP | DOTSXP => {
                    if hasattr {
                        self.skip_item(cx)?;
                    }
                    if hastag {
                        self.skip_item(cx)?;
                    }
                    self.skip_item(cx)?;
                    flags = self.inp.integer(cx)?;
                    continue;
                }
                EXTPTRSXP => {
                    self.skip_item(cx)?;
                    self.skip_item(cx)?;
                }
                WEAKREFSXP | OBJSXP => {}
                SPECIALSXP | BUILTINSXP => {
                    let n = self.inp.integer(cx)?;
                    self.inp.string(cx, n.max(0) as usize)?;
                }
                CHARSXP => {
                    let n = self.inp.integer(cx)?;
                    if n >= 0 {
                        self.inp.string(cx, n as usize)?;
                    }
                }
                LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP | EXPRSXP => {
                    let n = self.skip_length(cx)?;
                    match (t, self.inp.fmt) {
                        (LGLSXP | INTSXP, Format::Xdr | Format::Binary) => {
                            self.inp.in_bytes(cx, 4 * n)?;
                        }
                        (REALSXP, Format::Xdr | Format::Binary) => {
                            self.inp.in_bytes(cx, 8 * n)?;
                        }
                        (CPLXSXP, Format::Xdr | Format::Binary) => {
                            self.inp.in_bytes(cx, 16 * n)?;
                        }
                        (RAWSXP, Format::Xdr | Format::Binary) => {
                            self.inp.in_bytes(cx, n)?;
                        }
                        (LGLSXP | INTSXP, _) => {
                            for _ in 0..n {
                                self.inp.integer(cx)?;
                            }
                        }
                        (REALSXP, _) => {
                            for _ in 0..n {
                                self.inp.real(cx)?;
                            }
                        }
                        (CPLXSXP, _) => {
                            for _ in 0..2 * n {
                                self.inp.real(cx)?;
                            }
                        }
                        (RAWSXP, _) => {
                            for _ in 0..n {
                                self.inp.hex_byte(cx)?;
                            }
                        }
                        _ => {
                            for _ in 0..n {
                                self.skip_item(cx)?;
                            }
                        }
                    }
                }
                BCODESXP => {
                    self.inp.integer(cx)?;
                    self.skip_bc1(cx)?;
                }
                _ => return Err("unknown type".into()),
            }
            if hasattr {
                self.skip_item(cx)?;
            }
            return Ok(());
        }
    }

    fn skip_length(&mut self, cx: &Cx) -> Result<usize, String> {
        let len = self.inp.integer(cx)?;
        if len == -1 {
            let a = self.inp.integer(cx)? as u32 as usize;
            let b = self.inp.integer(cx)? as u32 as usize;
            return Ok((a << 32) + b);
        }
        usize::try_from(len).map_err(|_| "negative length".to_string())
    }

    fn skip_string_vec(&mut self, cx: &Cx) -> Result<(), String> {
        self.inp.integer(cx)?;
        let n = self.inp.integer(cx)?;
        for _ in 0..n.max(0) {
            self.skip_item(cx)?;
        }
        Ok(())
    }

    // R: serialize.c:ReadBC1, ReadBCConsts (the grammar)
    fn skip_bc1(&mut self, cx: &Cx) -> Result<(), String> {
        self.skip_item(cx)?;
        let n = self.inp.integer(cx)?;
        for _ in 0..n.max(0) {
            let t = self.inp.integer(cx)?;
            match t {
                x if x == BCODESXP as i32 => self.skip_bc1(cx)?,
                x if x == LANGSXP as i32 || x == LISTSXP as i32 => self.skip_bc_lang(cx, t)?,
                BCREPDEF | BCREPREF | ATTRLANGSXP | ATTRLISTSXP => self.skip_bc_lang(cx, t)?,
                _ => self.skip_item(cx)?,
            }
        }
        Ok(())
    }

    // R: serialize.c:ReadBCLang (the grammar)
    fn skip_bc_lang(&mut self, cx: &Cx, mut t: i32) -> Result<(), String> {
        if t == BCREPREF {
            self.inp.integer(cx)?;
            return Ok(());
        }
        if t == BCREPDEF || t == LANGSXP as i32 || t == LISTSXP as i32 || t == ATTRLANGSXP || t == ATTRLISTSXP {
            if t == BCREPDEF {
                self.inp.integer(cx)?;
                t = self.inp.integer(cx)?;
            }
            if t == ATTRLANGSXP || t == ATTRLISTSXP {
                self.skip_item(cx)?;
            }
            self.skip_item(cx)?;
            let a = self.inp.integer(cx)?;
            self.skip_bc_lang(cx, a)?;
            let d = self.inp.integer(cx)?;
            return self.skip_bc_lang(cx, d);
        }
        self.skip_item(cx)
    }

    /// `ALTREP_SXP`: the class info, state and attributes, then the class's
    /// `UnserializeEX`: base's compact sequences are robigo-rt's own classes
    /// (`robigo_rtcore::altrep::own`); any other class is not implemented
    /// yet. A compact sequence read inside another object is materialized
    /// (its serialized state is its info all the same), so code that reads
    /// that object's elements finds their data.
    // R: serialize.c:ReadItem_Recursive (ALTREP_SXP), altrep.c:ALTREP_UNSERIALIZE_EX,
    // altrep_UnserializeEX_default, altclasses.c:compact_intseq_Unserialize,
    // compact_realseq_Unserialize
    fn altrep(&mut self, cx: &mut Cx, levs: u16, objf: bool) -> RResult<Sexp> {
        cx.scope(|cx, s| {
            self.depth += 1;
            let mut parts = Vec::with_capacity(3);
            for _ in 0..3 {
                let v = self.read_item(cx)?;
                // SAFETY: just read.
                parts.push(s.root(cx, unsafe { r(cx, v) }));
            }
            self.depth -= 1;
            let (info, state, attr) = (parts[0], parts[1], parts[2]);
            let names = |cx: &Cx| -> Option<(Vec<u8>, Vec<u8>, u8)> {
                let i = info.bind(cx).as_cons()?;
                let c = i.car().as_sym()?.name_bytes().to_vec();
                let rest = i.cdr().as_cons()?;
                let p = rest.car().as_sym()?.name_bytes().to_vec();
                let t = rest.cdr().as_cons()?.car().as_scalar::<i32>()?;
                Some((c, p, t as u8))
            };
            let Some((cls, pkg, t)) = names(cx) else {
                return Err(cx.error("cannot unserialize this ALTREP object"));
            };
            let Some(class) = robigo_rtcore::altrep::own::OwnClass::from_names(&cls, &pkg, t) else {
                let msg = format!(
                    "robigo: unserializing an ALTREP object of class '{}' from package '{}' is not implemented yet",
                    String::from_utf8_lossy(&cls),
                    String::from_utf8_lossy(&pkg)
                );
                return Err(cx.error(&msg));
            };
            // COMPACT_INTSEQ_SERIALIZED_STATE_*: an INTSXP or REALSXP of three
            let seq = |cx: &Cx| -> Option<(f64, f64, f64)> {
                match state.bind(cx).view() {
                    View::Real(v) if v.len() >= 3 => Some((v.get(0), v.get(1), v.get(2))),
                    View::Int(v) if v.len() >= 3 => Some((v.get(0) as f64, v.get(1) as f64, v.get(2) as f64)),
                    _ => None,
                }
            };
            let Some((n, n1, inc)) = seq(cx) else {
                return Err(cx.error("robigo: a compact sequence's serialized state is not understood"));
            };
            let o = s.out(cx);
            let v = o.root();
            cx.compact_unserialize(class, n as usize, n1, inc, o)?;
            // altrep_UnserializeEX_default: SET_ATTRIB, SET_OBJECT, SETLEVELS
            // SAFETY: the rooted object and attributes.
            unsafe {
                let x = v.bind(cx).sexp();
                x.set_attrib(attr.bind(cx).sexp());
                x.set_object(objf);
                x.set_gp(levs);
            }
            if self.depth > 0 {
                cx.materialize(v)?;
            }
            Ok(v.bind(cx).sexp())
        })
    }

    /// The types `ReadItem_Recursive`'s inner switch reads, then their
    /// LEVELS, OBJECT and ATTRIB.
    // R: serialize.c:ReadItem_Recursive (default)
    fn read_other(&mut self, cx: &mut Cx, t: u8, levs: u16, objf: bool, hasattr: bool) -> RResult<Sexp> {
        cx.scope(|cx, s| {
            let nil = cx.nil().sexp();
            let x: Root<'_> = match t {
                EXTPTRSXP => {
                    // SAFETY: an EXTPTRSXP's car is its address, cleared below.
                    let p = cx.retry_alloc(|cx| unsafe { cx.alloc_node_raw(EXTPTRSXP, nil, nil, nil) })?;
                    // SAFETY: fresh.
                    unsafe { p.set_extptr_addr(std::ptr::null_mut()) };
                    // SAFETY: fresh, nothing collected since.
                    let x = s.root(cx, unsafe { r(cx, p) });
                    self.add_read_ref(cx, x)?;
                    self.depth += 1;
                    let prot = self.read_item(cx)?;
                    // SAFETY: rooted, just read.
                    unsafe { x.bind(cx).sexp().set_cdr(prot) };
                    let tag = self.read_item(cx)?;
                    // SAFETY: as above.
                    unsafe { x.bind(cx).sexp().set_tag(tag) };
                    self.depth -= 1;
                    x
                }
                WEAKREFSXP => {
                    let mut retried = false;
                    let w = loop {
                        // SAFETY: nil is immortal; the reference is rooted by
                        // the collector's list.
                        match unsafe { cx.new_weak_ref_raw(nil, nil, nil, false) } {
                            Ok(w) => break w,
                            Err(e) if !retried => {
                                cx.need_gc(e)?;
                                retried = true;
                            }
                            Err(e) => {
                                cx.need_gc(e)?;
                                return Err(cx.error("robigo: no room for a weak reference"));
                            }
                        }
                    };
                    // SAFETY: fresh.
                    let x = s.root(cx, unsafe { r(cx, w) });
                    self.add_read_ref(cx, x)?;
                    x
                }
                SPECIALSXP | BUILTINSXP => {
                    let length = self.int(cx)?;
                    if length < 0 {
                        return Err(cx.error("invalid length"));
                    }
                    let name = self.string(cx, length as usize)?;
                    let st = crate::prims::state(cx);
                    let found = st.defs.iter().position(|d| d.name.as_bytes() == &name[..]);
                    match found {
                        Some(i) => {
                            let g = st.objs[i].bind(cx);
                            s.root(cx, g)
                        }
                        None => {
                            let msg = format!(
                                "robigo: the primitive '{}' is not implemented yet",
                                String::from_utf8_lossy(&name)
                            );
                            return Err(cx.error(&msg));
                        }
                    }
                }
                CHARSXP => {
                    let length = self.int(cx)?;
                    if length < -1 {
                        return Err(cx.error("invalid length"));
                    }
                    let c = if length == -1 {
                        cx.na_string().as_ref().sexp()
                    } else {
                        let buf = self.string(cx, length as usize)?;
                        self.read_char(cx, buf, levs)?
                    };
                    // SAFETY: a CHARSXP just made or found (the cache keeps it
                    // until a collection, and none has run).
                    let x = s.root(cx, unsafe { r(cx, c) });
                    if hasattr {
                        self.read_deeper(cx)?;
                    }
                    let _ = objf;
                    return Ok(x.bind(cx).sexp());
                }
                LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP => {
                    let len = self.read_length(cx)?;
                    let v = cx.retry_alloc(|cx| cx.alloc_vector_raw(t, len))?;
                    // SAFETY: fresh, of `len` elements of `t`; filled before
                    // anything else allocates.
                    let res = unsafe {
                        let data = v.std_data();
                        match t {
                            LGLSXP | INTSXP => {
                                self.inp.ints_into(cx, std::slice::from_raw_parts_mut(data as *mut i32, len))
                            }
                            REALSXP => self.inp.reals_into(cx, std::slice::from_raw_parts_mut(data as *mut f64, len)),
                            CPLXSXP => {
                                self.inp.reals_into(cx, std::slice::from_raw_parts_mut(data as *mut f64, 2 * len))
                            }
                            _ => {
                                let out = std::slice::from_raw_parts_mut(data, len);
                                match self.inp.fmt {
                                    Format::Ascii | Format::AsciiHex => {
                                        let mut r = Ok(());
                                        for o in out.iter_mut() {
                                            match self.inp.hex_byte(cx) {
                                                Ok(b) => *o = b,
                                                Err(e) => {
                                                    r = Err(e);
                                                    break;
                                                }
                                            }
                                        }
                                        r
                                    }
                                    _ => self.inp.in_bytes(cx, len).map(|b| out.copy_from_slice(b)),
                                }
                            }
                        }
                    };
                    if let Err(m) = res {
                        return Err(cx.error(&m));
                    }
                    // SAFETY: fresh, now filled.
                    s.root(cx, unsafe { r(cx, v) })
                }
                STRSXP | VECSXP | EXPRSXP => {
                    let len = self.read_length(cx)?;
                    let v = cx.retry_alloc(|cx| cx.alloc_vector_raw(t, len))?;
                    // SAFETY: fresh (its elements R_NilValue or "").
                    let x = s.root(cx, unsafe { r(cx, v) });
                    self.depth += 1;
                    for i in 0..len {
                        let e = self.read_item(cx)?;
                        if t == STRSXP {
                            self.set_string_elt(cx, x, i, e)?;
                        } else {
                            // SAFETY: the rooted vector, `i` in range, `e` just read.
                            unsafe { x.bind(cx).sexp().set_elt(i, e) };
                        }
                    }
                    self.depth -= 1;
                    x
                }
                BCODESXP => {
                    let v = self.read_bc(cx)?;
                    // SAFETY: just read.
                    s.root(cx, unsafe { r(cx, v) })
                }
                OBJSXP => {
                    // SAFETY: nil is immortal.
                    let o = cx.retry_alloc(|cx| unsafe { cx.alloc_node_raw(OBJSXP, nil, nil, nil) })?;
                    // SAFETY: fresh.
                    s.root(cx, unsafe { r(cx, o) })
                }
                _ => {
                    return Err(cx.error(&format!("ReadItem: unknown type {t}, perhaps written by later version of R")));
                }
            };
            // SAFETY: the rooted object.
            unsafe {
                let o = x.bind(cx).sexp();
                if o.gp() != levs {
                    o.set_gp(levs);
                }
                if o.is_object() != objf {
                    o.set_object(objf);
                }
            }
            let a = if hasattr {
                self.depth += 1;
                let a = self.read_item(cx);
                self.depth -= 1;
                a?
            } else {
                nil
            };
            // SAFETY: the rooted object, `a` just read.
            unsafe {
                let o = x.bind(cx).sexp();
                if o.attrib_raw() != a {
                    o.set_attrib(a);
                }
            }
            // byte code this R cannot run reads as its expression
            let v = x.bind(cx);
            if t == BCODESXP && !robigo_rtbuiltins::bytecode::version_ok(v) {
                return Ok(robigo_rtbuiltins::bytecode::expr(cx, v).sexp());
            }
            Ok(v.sexp())
        })
    }

    /// `ReadBC`: byte code, after the number of its shared language cells
    /// (`reps`, filled as `BCREPDEF`s define them).
    // R: serialize.c:ReadBC
    fn read_bc(&mut self, cx: &mut Cx) -> RResult<Sexp> {
        let n = self.int(cx)?;
        let Ok(n) = usize::try_from(n) else {
            return Err(cx.error("negative length vectors are not allowed"));
        };
        cx.scope(|cx, s| {
            let v = cx.retry_alloc(|cx| cx.alloc_vector_raw(VECSXP, n))?;
            // SAFETY: fresh.
            let reps = s.root(cx, unsafe { r(cx, v) });
            self.read_bc1(cx, reps)
        })
    }

    /// `ReadBC1`: a BCODESXP, its code encoded as `R_bcEncode` encodes it
    /// before the constants are read; its expression (TAG) R_NilValue.
    /// (`R_registerBC` records the code for the constants check of an R
    /// built with it, which robigo-rt does not have.)
    // R: serialize.c:ReadBC1
    fn read_bc1(&mut self, cx: &mut Cx, reps: Root<'_>) -> RResult<Sexp> {
        cx.scope(|cx, s| {
            let nil = cx.nil().sexp();
            // SAFETY: nil is immortal.
            let b = cx.retry_alloc(|cx| unsafe { cx.alloc_node_raw(BCODESXP, nil, nil, nil) })?;
            // SAFETY: fresh.
            let bc = s.root(cx, unsafe { r(cx, b) });
            let bytes = self.read_deeper(cx)?;
            // SAFETY: just read.
            let bytes = s.root(cx, unsafe { r(cx, bytes) });
            let code = robigo_rtbuiltins::bytecode::bc_encode(cx, bytes)?;
            let code = match code {
                Some(pc) => s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&pc)?.finish().into())))?,
                None => s.root(cx, cx.nil()),
            };
            // SAFETY: the rooted cell and code.
            unsafe { bc.bind(cx).sexp().set_car(code.bind(cx).sexp()) };
            let consts = self.read_bc_consts(cx, reps)?;
            // SAFETY: the rooted cell, the constants just read.
            unsafe { bc.bind(cx).sexp().set_cdr(consts) };
            Ok(bc.bind(cx).sexp())
        })
    }

    /// `ReadBCConsts`: the constants, a list; byte code and language cells
    /// in the representation that shares cells.
    // R: serialize.c:ReadBCConsts
    fn read_bc_consts(&mut self, cx: &mut Cx, reps: Root<'_>) -> RResult<Sexp> {
        let n = self.int(cx)?;
        let Ok(n) = usize::try_from(n) else {
            return Err(cx.error("negative length vectors are not allowed"));
        };
        cx.scope(|cx, s| {
            let v = cx.retry_alloc(|cx| cx.alloc_vector_raw(VECSXP, n))?;
            // SAFETY: fresh.
            let ans = s.root(cx, unsafe { r(cx, v) });
            for i in 0..n {
                let t = self.int(cx)?;
                let c = match t {
                    x if x == BCODESXP as i32 => self.read_bc1(cx, reps)?,
                    x if x == LANGSXP as i32 || x == LISTSXP as i32 => self.read_bc_lang(cx, t, reps)?,
                    BCREPDEF | BCREPREF | ATTRLANGSXP | ATTRLISTSXP => self.read_bc_lang(cx, t, reps)?,
                    _ => self.read_deeper(cx)?,
                };
                // SAFETY: the rooted list, `i` in range, `c` just read.
                unsafe { ans.bind(cx).sexp().set_elt(i, c) };
            }
            Ok(ans.bind(cx).sexp())
        })
    }

    /// `ReadBCLang(type)`: a language or pairlist cell, its CAR and CDR in
    /// the same representation; a `BCREPDEF` cell is entered in `reps`
    /// before its contents are read, so a later `BCREPREF` (inside it too)
    /// finds it. Anything else is an item.
    // R: serialize.c:ReadBCLang
    fn read_bc_lang(&mut self, cx: &mut Cx, t: i32, reps: Root<'_>) -> RResult<Sexp> {
        cx.check_c_stack()?;
        match t {
            BCREPREF => {
                let i = self.int(cx)?;
                let i = rep_index(cx, reps, i, "attempt access index {i}/{n} in VECTOR_ELT")?;
                // SAFETY: the rooted list (a standard VECSXP), `i` in range.
                Ok(unsafe { *(reps.bind(cx).sexp().std_data() as *const Sexp).add(i) })
            }
            _ if t == BCREPDEF
                || t == LANGSXP as i32
                || t == LISTSXP as i32
                || t == ATTRLANGSXP
                || t == ATTRLISTSXP =>
            {
                let mut t = t;
                let mut pos = None;
                if t == BCREPDEF {
                    let p = self.int(cx)?;
                    pos = Some(rep_index(cx, reps, p, "attempt to set index {i}/{n} in SET_VECTOR_ELT")?);
                    t = self.int(cx)?;
                }
                let (t, hasattr) = match t {
                    ATTRLANGSXP => (LANGSXP, true),
                    ATTRLISTSXP => (LISTSXP, true),
                    x if x == LANGSXP as i32 || x == LISTSXP as i32 => (x as u8, false),
                    _ => {
                        return Err(cx.error(&format!(
                            "robigo: a shared cell of byte code of type {t} (not a language or pairlist cell) is not \
                             implemented yet"
                        )));
                    }
                };
                cx.scope(|cx, s| {
                    let nil = cx.nil().sexp();
                    // SAFETY: nil is immortal.
                    let c = cx.retry_alloc(|cx| unsafe { cx.alloc_node_raw(t, nil, nil, nil) })?;
                    // SAFETY: fresh.
                    let ans = s.root(cx, unsafe { r(cx, c) });
                    if let Some(i) = pos {
                        // SAFETY: the rooted list, `i` in range, the rooted cell.
                        unsafe { reps.bind(cx).sexp().set_elt(i, ans.bind(cx).sexp()) };
                    }
                    self.depth += 1;
                    let parts = (|| -> RResult<()> {
                        if hasattr {
                            let a = self.read_item(cx)?;
                            // SAFETY: the rooted cell, `a` just read.
                            unsafe { ans.bind(cx).sexp().set_attrib(a) };
                        }
                        let tag = self.read_item(cx)?;
                        // SAFETY: as above.
                        unsafe { ans.bind(cx).sexp().set_tag(tag) };
                        Ok(())
                    })();
                    self.depth -= 1;
                    parts?;
                    let a = self.int(cx)?;
                    let car = self.read_bc_lang(cx, a, reps)?;
                    // SAFETY: the rooted cell, `car` just read.
                    unsafe { ans.bind(cx).sexp().set_car(car) };
                    let d = self.int(cx)?;
                    let cdr = self.read_bc_lang(cx, d, reps)?;
                    // SAFETY: the rooted cell, `cdr` just read.
                    unsafe { ans.bind(cx).sexp().set_cdr(cdr) };
                    Ok(ans.bind(cx).sexp())
                })
            }
            _ => self.read_deeper(cx),
        }
    }

    /// `ReadChar(stream, buf, length, levs)`: the CHARSXP, converted from
    /// the writer's native encoding as GNU R's version 3 does.
    // R: serialize.c:ReadChar, invalid_utf8_warning
    fn read_char(&mut self, cx: &mut Cx, buf: Vec<u8>, levs: u16) -> RResult<Sexp> {
        let mk = |cx: &mut Cx, b: &[u8], e: Encoding| -> RResult<Sexp> {
            cx.retry_alloc(|cx| cx.mk_char(b, e).map(|c| c.as_ref().sexp()))
        };
        if let Some(e) = marked_encoding(levs) {
            return mk(cx, &buf, e);
        }
        if self.native.is_empty() || (self.nat2nat == Conv::Off && self.nat2utf8 == Conv::Off) {
            return mk(cx, &buf, Encoding::Native);
        }
        let ours = native_encoding(cx).as_bytes();
        if self.nat2nat == Conv::Unopened && self.native == ours {
            self.nat2nat = Conv::Off;
            self.nat2utf8 = Conv::Off;
            return mk(cx, &buf, Encoding::Native);
        }
        let from_name = String::from_utf8_lossy(&self.native).into_owned();
        let Some(from) = codeset(&self.native) else {
            return Err(cx.error(&format!(
                "robigo: strings serialized in the native encoding '{from_name}' are not implemented yet"
            )));
        };
        let to = if cx.locale().utf8 { Codeset::Utf8 } else { Codeset::Ascii };
        if self.nat2nat == Conv::Unopened {
            self.nat2nat = Conv::Open(from, to);
        }
        let utf8 = cx.locale().utf8;
        if let Conv::Open(f, t) = self.nat2nat {
            let enc = if utf8 { Encoding::Utf8 } else { Encoding::Native };
            if let Some(v) = convert(f, t, &buf) {
                return mk(cx, &v, enc);
            }
            if utf8 {
                self.nat2utf8 = Conv::Off;
                self.invalid_utf8_warning(cx, &buf, from, &from_name)?;
            }
        }
        if self.nat2utf8 == Conv::Unopened {
            self.nat2utf8 = Conv::Open(from, Codeset::Utf8);
            cx.warning("strings not representable in native encoding will be translated to UTF-8")?;
        }
        if let Conv::Open(f, t) = self.nat2utf8 {
            if let Some(v) = convert(f, t, &buf) {
                return mk(cx, &v, Encoding::Utf8);
            }
            self.invalid_utf8_warning(cx, &buf, from, &from_name)?;
        }
        mk(cx, &buf, Encoding::Native)
    }

    fn invalid_utf8_warning(&mut self, cx: &mut Cx, buf: &[u8], from: Codeset, from_name: &str) -> RResult<()> {
        let ours = if cx.locale().utf8 { Codeset::Utf8 } else { Codeset::Ascii };
        if std::str::from_utf8(buf).is_ok() {
            let shown = String::from_utf8_lossy(&re_enc3(buf, Codeset::Utf8, ours)).into_owned();
            cx.warning(&format!(
                "input string '{shown}' cannot be translated from '{from_name}' to UTF-8, but is valid UTF-8"
            ))
        } else {
            let shown =
                String::from_utf8_lossy(&re_enc3(&re_enc3(buf, from, Codeset::Utf8), Codeset::Utf8, ours)).into_owned();
            cx.warning(&format!("input string '{shown}' cannot be translated to UTF-8, is it valid in '{from_name}'?"))
        }
    }
}

/// Whether the attribute list `a` has a `class` entry (GNU R's
/// `getAttrib(s, R_ClassSymbol) != R_NilValue` on an environment).
///
/// # Safety
/// `a` is a live pairlist.
unsafe fn has_class(cx: &Cx, a: Sexp) -> bool {
    let class = cx.sym(robigo_rtcore::RSym::Class).as_ref().sexp();
    let nil = cx.nil().sexp();
    let mut c = a;
    // SAFETY: the caller's contract.
    unsafe {
        while c != nil && c.typ() == LISTSXP {
            if c.tag() == class {
                return c.car() != nil;
            }
            c = c.cdr();
        }
    }
    false
}

/// `reps`' index `i` as `VECTOR_ELT` and `SET_VECTOR_ELT` check it, with
/// their message (`{i}` and `{n}` the index and the length).
// R: memory.c:VECTOR_ELT, SET_VECTOR_ELT (the index check)
fn rep_index(cx: &mut Cx, reps: Root<'_>, i: i32, msg: &str) -> RResult<usize> {
    // SAFETY: the rooted list.
    let n = unsafe { reps.bind(cx).sexp().std_len() } as i64;
    match usize::try_from(i) {
        Ok(k) if (k as i64) < n => Ok(k),
        _ => Err(cx.error(&msg.replace("{i}", &i.to_string()).replace("{n}", &n.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(convert(Codeset::Latin1, Codeset::Utf8, b"caf\xe9").unwrap(), "café".as_bytes());
        assert_eq!(convert(Codeset::Utf8, Codeset::Latin1, "café".as_bytes()).unwrap(), b"caf\xe9");
        assert!(convert(Codeset::Ascii, Codeset::Utf8, b"caf\xe9").is_none());
        assert_eq!(re_enc3(b"a\xffb", Codeset::Utf8, Codeset::Utf8), b"a<ff>b");
    }
}
