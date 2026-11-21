//! `.Internal(inspect(x, deep, pvec))`: GNU R's low-level dump of an
//! object's header and tree (inspect.c), line for line.
//!
//! Each line is `@<address> <type> <type name> g<gen>c<class> [<flags>]`,
//! then what the type has (lengths, a string, a symbol's name, the first
//! elements of a vector), and then, as deep as asked, its children.
//! - **The address** is robigo's: it is never compared, only relations on
//!   it (L1's "inherently variable values").
//! - **`g`, the generation bit,** is 0: robigo-rt's collector has no
//!   generations, and every collection is full (the collection schedule is
//!   robigo's own under L7).
//! - **`c`, the node class,** is the one GNU R's allocator would give the
//!   object (`Ref::gnu_node_class`).
//! - **The flags** are GNU R's from the header bits robigo keeps for them:
//!   OBJ, REF(n), DBG/TR/STP, S4, a binding's AB/LCK, an environment's
//!   LCK/GL, a promise's VAL, `gp=` (LEVELS), ATT. MARK is never set
//!   outside a collection, as in GNU R.
//! - **A slot frame's bindings** print as GNU R's FRAME, a pairlist of
//!   binding cells (`Cx::frame_snapshot`), so FRAME's shape is GNU R's.
//!   robigo has no immediate binding cells (GNU R's `BNDCELL_TAG`).

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, RResult, Ref, Root, Scope, View};

/// `do_inspect(call, op, args, env)`: positional `x`, then `deep` (-1:
/// unlimited) and `pvec` (5), each `asInteger`. The value is `x`, invisible.
// R: inspect.c:do_inspect
pub fn do_inspect(
    cx: &mut Cx,
    _call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: robigo_rtcore::Out<'_>,
) -> RResult<()> {
    let x = args.root(0);
    let mut deep = -1;
    let mut pvec = 5;
    if args.len() > 1 {
        deep = robigo_rtbuiltins::coerce::vector::as_integer(cx, args.root(1).bind(cx)).0;
        if args.len() > 2 {
            pvec = robigo_rtbuiltins::coerce::vector::as_integer(cx, args.root(2).bind(cx)).0;
        }
    }
    cx.scope(|cx, s| inspect_tree(cx, s, 0, x, deep, pvec))?;
    out.set(cx, x.bind(cx));
    cx.set_visible(false);
    Ok(())
}

/// The prefix: a tab for every 8 columns, then spaces.
// R: inspect.c:pp
fn pp(b: &mut Vec<u8>, pre: i32) {
    let mut p = pre;
    while p >= 8 {
        b.push(b'\t');
        p -= 8;
    }
    for _ in 0..p.max(0) {
        b.push(b' ');
    }
}

fn emit(cx: &Cx, b: &mut Vec<u8>) {
    cx.rprintf(b);
    b.clear();
}

/// `%g` as C's printf writes it, `nan` and `inf` included.
fn c_g(d: f64) -> String {
    if d.is_nan() {
        return if d.is_sign_negative() { "-nan".into() } else { "nan".into() };
    }
    if d.is_infinite() {
        return if d < 0.0 { "-inf".into() } else { "inf".into() };
    }
    crate::serialize::stream::fmt_g(d, 6)
}

/// GNU R's `PrintEnvironment`: `EncodeEnvironment`'s text without its
/// `environment: ` (`<R_GlobalEnv>`, `<base>`, `<package:stats>`,
/// `<namespace:x>`, `<0x...>`).
// R: inspect.c:PrintEnvironment
fn env_label(cx: &mut Cx, e: Root<'_>) -> RResult<Vec<u8>> {
    let t = crate::print::encode_environment(cx, e)?;
    Ok(match t.strip_prefix(b"<environment: ") {
        Some(rest) => [b"<" as &[u8], rest].concat(),
        None => t,
    })
}

/// `inspect_tree(pre, v, deep, pvec)`.
// R: inspect.c:inspect_tree
fn inspect_tree(cx: &mut Cx, s: Scope<'_>, pre: i32, v: Root<'_>, deep: i32, pvec: i32) -> RResult<()> {
    let mut b = Vec::new();
    pp(&mut b, pre);
    let r = v.bind(cx);
    let t = r.sexptype();
    let name = if t == OBJSXP && r.is_s4() { "S4SXP" } else { robigo_rtbuiltins::util::sexptype2char(t) };
    b.extend_from_slice(format!("@{:x} {:02} {} g0c{} [", r.sexp().addr(), t, name, r.gnu_node_class()).as_bytes());
    let mut flags: Vec<String> = Vec::new();
    if r.is_object() {
        flags.push("OBJ".into());
    }
    if r.refcnt() != 0 {
        flags.push(format!("REF({})", r.refcnt()));
    }
    let (dbg, tr, stp) = r.debug_bits();
    if dbg {
        flags.push("DBG".into());
    }
    if tr {
        flags.push("TR".into());
    }
    if stp {
        flags.push("STP".into());
    }
    if r.is_s4() {
        flags.push("S4".into());
    }
    let mut lv = r.levels();
    match t {
        SYMSXP => {
            // the base binding's bits (GNU R keeps them on the symbol)
            let sym = r.as_sym().unwrap();
            if let robigo_rtcore::Lookup::Bound(bd) = cx.frame_binding(env_of(cx.base_env()), sym) {
                // (and in its gp: LEVELS shows them, as GNU R's does)
                if bd.is_active(cx) {
                    flags.push("AB".into());
                    lv |= robigo_heap::gp::cell::ACTIVE_BINDING;
                }
                if bd.is_locked(cx) {
                    flags.push("LCK".into());
                    lv |= robigo_heap::gp::cell::BINDING_LOCK;
                }
            }
        }
        LISTSXP => {
            if lv & robigo_heap::gp::cell::ACTIVE_BINDING != 0 {
                flags.push("AB".into());
            }
            if lv & robigo_heap::gp::cell::BINDING_LOCK != 0 {
                flags.push("LCK".into());
            }
        }
        ENVSXP => {
            if lv & robigo_heap::gp::env::FRAME_LOCK != 0 {
                flags.push("LCK".into());
            }
            if lv & robigo_heap::gp::env::GLOBAL_FRAME != 0 {
                flags.push("GL".into());
            }
        }
        PROMSXP => {
            if let View::Promise(p) = r.view()
                && p.value().sexp() != cx.unbound().sexp()
            {
                flags.push("VAL".into());
            }
        }
        _ => {}
    }
    if lv != 0 {
        flags.push(format!("gp=0x{lv:x}"));
    }
    let has_attrib = r.attrib().is_some_and(|a| a.sexptype() != NILSXP);
    if has_attrib {
        flags.push("ATT".into());
    }
    b.extend_from_slice(flags.join(",").as_bytes());
    b.extend_from_slice(b"] ");

    // ALTREP: the class's Inspect method (robigo's own classes are GNU R's
    // compact sequences)
    if r.is_altrep() {
        if r.is_package_altrep() {
            return Err(robigo_rtbuiltins::util::not_yet(cx, None, "inspect() of a package ALTREP vector"));
        }
        compact_seq(cx, &mut b, r);
        emit(cx, &mut b);
        if has_attrib {
            pp(&mut b, pre);
            b.extend_from_slice(b"ATTRIB:\n");
            emit(cx, &mut b);
            let a = s.root(cx, v.bind(cx).attrib().unwrap());
            inspect_tree(cx, s, pre + 2, a, deep, pvec)?;
        }
        return Ok(());
    }

    if matches!(t, VECSXP | STRSXP | LGLSXP | INTSXP | RAWSXP | REALSXP | CPLXSXP | EXPRSXP) {
        let len = r.len().unwrap_or(0);
        let tl = r.truelength();
        if r.is_growable() {
            b.extend_from_slice(format!("(len={len}, tl={tl}, gr)").as_bytes());
        } else {
            b.extend_from_slice(format!("(len={len}, tl={tl})").as_bytes());
        }
    }
    if t == ENVSXP {
        emit(cx, &mut b);
        let l = env_label(cx, v)?;
        b.extend_from_slice(&l);
    }
    let r = v.bind(cx);
    match r.view() {
        View::Char(c) => char_line(cx, &mut b, c),
        View::Sym(sym) => {
            let sx = sym.as_ref().sexp();
            if sx == cx.unbound().sexp() {
                b.extend_from_slice(b"[unbound value]");
            } else if sx == cx.missing_arg().sexp() {
                b.extend_from_slice(b"[missing argument]");
            } else if sx == cx.restart_token().sexp() {
                b.extend_from_slice(b"[restart token]");
            } else {
                let bound = matches!(cx.frame_binding(env_of(cx.base_env()), sym), robigo_rtcore::Lookup::Bound(_));
                let pn = s.root(cx, sym.name(cx).as_ref());
                emit(cx, &mut b);
                let name = crate::print::encode_char_now(cx, pn)?;
                b.push(b'"');
                b.extend_from_slice(&name);
                b.push(b'"');
                if bound {
                    b.extend_from_slice(b" (has value)");
                }
            }
        }
        View::ExtPtr(e) => b.extend_from_slice(format!("<{}>", crate::print::c_pointer(e.addr() as usize)).as_bytes()),
        View::Lgl(x) => first(&mut b, x.len(), pvec, |i| (x.get(i).0).to_string()),
        View::Int(x) => first(&mut b, x.len(), pvec, |i| x.get(i).to_string()),
        View::Raw(x) => first(&mut b, x.len(), pvec, |i| format!("{:02x}", x.get(i))),
        View::Real(x) => first(&mut b, x.len(), pvec, |i| c_g(x.get(i))),
        _ => {}
    }
    b.push(b'\n');
    emit(cx, &mut b);

    if deep != 0 {
        match t {
            VECSXP | EXPRSXP | STRSXP => {
                let n = v.bind(cx).len().unwrap_or(0);
                let shown = n.min(pvec.max(0) as usize);
                for i in 0..shown {
                    let e = s.root(cx, vector_elt(cx, v, i));
                    inspect_tree(cx, s, pre + 2, e, deep - 1, pvec)?;
                }
                if shown < n {
                    pp(&mut b, pre + 2);
                    b.extend_from_slice(b"...\n");
                    emit(cx, &mut b);
                }
            }
            LISTSXP | LANGSXP => {
                let mut lc = s.root(cx, v.bind(cx));
                loop {
                    let l = lc.bind(cx);
                    if l.sexptype() == NILSXP {
                        break;
                    }
                    let Some(cell) = l.as_cons().filter(|_| matches!(l.sexptype(), LISTSXP | LANGSXP)) else {
                        // a dotted pair
                        pp(&mut b, pre + 2);
                        b.extend_from_slice(b".\n");
                        emit(cx, &mut b);
                        inspect_tree(cx, s, pre + 2, lc, deep - 1, pvec)?;
                        break;
                    };
                    if cell.tag().sexptype() != NILSXP {
                        pp(&mut b, pre + 2);
                        b.extend_from_slice(b"TAG: ");
                        emit(cx, &mut b);
                        let tag = s.root(cx, cell.tag());
                        inspect_tree(cx, s, 0, tag, deep - 1, pvec)?;
                    }
                    let car = s.root(cx, lc.bind(cx).as_cons().unwrap().car());
                    inspect_tree(cx, s, pre + 2, car, deep - 1, pvec)?;
                    lc = s.root(cx, lc.bind(cx).as_cons().unwrap().cdr());
                }
            }
            ENVSXP => {
                let o = s.out(cx);
                let frame = o.root();
                cx.frame_snapshot(v, o)?;
                if frame.bind(cx).sexptype() != NILSXP {
                    pp(&mut b, pre);
                    b.extend_from_slice(b"FRAME:\n");
                    emit(cx, &mut b);
                    inspect_tree(cx, s, pre + 2, frame, deep - 1, pvec)?;
                }
                pp(&mut b, pre);
                b.extend_from_slice(b"ENCLOS:\n");
                emit(cx, &mut b);
                let enclos = s.root(cx, env_of(v.bind(cx)).enclos());
                inspect_tree(cx, s, pre + 2, enclos, 0, pvec)?;
                let ht = s.root(cx, cx.hash_table(env_of(v.bind(cx))));
                if ht.bind(cx).sexptype() != NILSXP {
                    pp(&mut b, pre);
                    b.extend_from_slice(b"HASHTAB:\n");
                    emit(cx, &mut b);
                    inspect_tree(cx, s, pre + 2, ht, deep - 1, pvec)?;
                }
            }
            CLOSXP => {
                for (label, field, d) in [("FORMALS", 0, deep - 1), ("BODY", 1, deep - 1), ("CLOENV", 2, 0)] {
                    pp(&mut b, pre);
                    b.extend_from_slice(label.as_bytes());
                    b.extend_from_slice(b":\n");
                    emit(cx, &mut b);
                    let View::Closure(c) = v.bind(cx).view() else { unreachable!() };
                    let f = match field {
                        0 => c.formals(),
                        1 => c.body(),
                        _ => c.env(),
                    };
                    let f = s.root(cx, f);
                    inspect_tree(cx, s, pre + 2, f, d, pvec)?;
                }
            }
            EXTPTRSXP => {
                let View::ExtPtr(e) = v.bind(cx).view() else { unreachable!() };
                let (prot, tag) = (s.root(cx, e.prot()), s.root(cx, e.tag()));
                if prot.bind(cx).sexptype() != NILSXP {
                    pp(&mut b, pre);
                    b.extend_from_slice(b"PROTECTED:\n");
                    emit(cx, &mut b);
                    inspect_tree(cx, s, pre + 2, prot, deep - 1, pvec)?;
                }
                if tag.bind(cx).sexptype() != NILSXP {
                    pp(&mut b, pre);
                    b.extend_from_slice(b"TAG:\n");
                    emit(cx, &mut b);
                    inspect_tree(cx, s, pre + 2, tag, deep - 1, pvec)?;
                }
            }
            _ => {}
        }
    }

    if has_attrib && t != CHARSXP {
        pp(&mut b, pre);
        b.extend_from_slice(b"ATTRIB:\n");
        emit(cx, &mut b);
        let a = s.root(cx, v.bind(cx).attrib().unwrap());
        inspect_tree(cx, s, pre + 2, a, deep, pvec)?;
    }
    Ok(())
}

/// An environment's typed handle.
fn env_of(r: Ref<'_>) -> robigo_rtcore::EnvRef<'_> {
    match r.view() {
        View::Env(e) => e,
        _ => unreachable!("robigo: an environment"),
    }
}

/// The first `pvec` elements in line, `,...` when there are more.
fn first(b: &mut Vec<u8>, n: usize, pvec: i32, elt: impl Fn(usize) -> String) {
    let shown = n.min(pvec.max(0) as usize);
    for i in 0..shown {
        b.push(if i > 0 { b',' } else { b' ' });
        b.extend_from_slice(elt(i).as_bytes());
    }
    if shown < n {
        b.extend_from_slice(b",...");
    }
}

/// Element `i` of a VECSXP, EXPRSXP or STRSXP (a CHARSXP for a STRSXP).
fn vector_elt<'g>(cx: &'g Cx, v: Root<'_>, i: usize) -> Ref<'g> {
    match v.bind(cx).view() {
        View::Str(x) => x.get(i).as_ref(),
        View::List(x) | View::Expr(x) => x.get(i),
        _ => unreachable!("robigo: a vector of objects"),
    }
}

/// A CHARSXP's line: its encoding marks, the bytes, `[NA]`.
fn char_line(cx: &Cx, b: &mut Vec<u8>, c: Char<'_>) {
    let g = c.as_ref().levels();
    use robigo_heap::gp::charsxp::{ASCII, BYTES, CACHED, LATIN1, UTF8};
    for (bit, label) in
        [(BYTES, "[bytes] "), (LATIN1, "[latin1] "), (UTF8, "[UTF8] "), (ASCII, "[ASCII] "), (CACHED, "[cached] ")]
    {
        if g & bit != 0 {
            b.extend_from_slice(label.as_bytes());
        }
    }
    b.push(b'"');
    b.extend_from_slice(c.bytes());
    b.push(b'"');
    if c.as_ref().sexp() == cx.na_string().as_ref().sexp() {
        b.extend_from_slice(b" [NA]");
    }
}

/// GNU R's compact sequences' Inspect methods: ` n1 : n2 (compact)`, or
/// `(expanded)` once the sequence was expanded.
// R: altclasses.c:compact_intseq_Inspect, compact_realseq_Inspect
fn compact_seq(_cx: &Cx, b: &mut Vec<u8>, r: Ref<'_>) {
    // the class's own info (length, first, increment), so that inspecting
    // reads no element (which would expand the sequence)
    let expanded = if r.own_is_expanded() { "expanded" } else { "compact" };
    let (n, n1, inc) = r.compact_info();
    let (n, n1) = (n as i64, n1 as i64);
    let n2 = if inc == 1.0 { n1 + n - 1 } else { n1 - n + 1 };
    b.extend_from_slice(format!(" {n1} : {n2} ({expanded})\n").as_bytes());
}
