//! `R_Serialize` and `WriteItem`, ported.
//!
//! The walk reads the object graph raw, as GNU R's does, under `&mut Cx`
//! but without collecting: a raw `Sexp` it holds stays valid as long as
//! nothing collects. A few points of the walk run code that may collect
//! (GNU R's too): the persistent-name hook (R code), the warning for a
//! package environment, `FRAME()` of a slot frame (it allocates the
//! pairlist GNU R has), a promise's unboxed value (boxed once). Each is an
//! *effect point*: its result is kept, by its position in the walk's
//! sequence of effect points, in a global root. If a collection happened
//! while it ran, the walk's raw pointers may be stale (the moving
//! collector of `ROBIGO_GC_MOVE=1` moves objects, any collection may free
//! a frame snapshot), so the walk starts over from the root object, with
//! an empty output and reference table, and takes the kept results of the
//! effect points it passes again instead of running them again. The walk
//! is deterministic given those results, so every effect happens once, in
//! GNU R's order (the hook is called at every encounter of a hooked
//! object, as `GetPersistentName` is), and the output is what one walk
//! without collections writes.
//!
//! One case differs from GNU R: an effect point whose R code changes the
//! object being serialized (a refhook that assigns into an environment
//! written earlier in the same call) and collects. GNU R has written the
//! earlier parts already and keeps their old contents; the walk that
//! starts over writes them as they are now, and a change to the graph's
//! shape can move later effect points' places. Resuming from rooted walk
//! state instead of starting over would remove it.

#![allow(unsafe_code)]

use super::stream::OutStream;
use super::*;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Global, Out, RError, RResult, Ref, Root};
use std::collections::HashMap;

pub(super) enum WErr {
    R(RError),
    Restart,
}

impl From<RError> for WErr {
    fn from(e: RError) -> WErr {
        WErr::R(e)
    }
}

type W<T> = Result<T, WErr>;

use robigo_heap::gp::charsxp::{CACHED as CACHED_MASK, HASHASH as HASHASH_MASK};
use robigo_heap::gp::env::FRAME_LOCK as FRAME_LOCK_MASK;
use robigo_heap::gp::vector::GROWABLE as GROWABLE_MASK;

/// `PackFlags(type, levs, isobj, hasattr, hastag)`.
// R: serialize.c:PackFlags
pub(super) fn pack_flags(t: i32, mut levs: u16, isobj: bool, hasattr: bool, hastag: bool) -> i32 {
    if t == CHARSXP as i32 {
        levs &= !(CACHED_MASK | HASHASH_MASK);
    } else if matches!(t as u8, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP) {
        levs &= !GROWABLE_MASK;
    }
    let mut val = t | ((levs as i32) << 12);
    if isobj {
        val |= IS_OBJECT_BIT_MASK;
    }
    if hasattr {
        val |= HAS_ATTR_BIT_MASK;
    }
    if hastag {
        val |= HAS_TAG_BIT_MASK;
    }
    val
}

pub(super) struct Writer<'h> {
    pub out: OutStream,
    /// The reference table: an object's address and its index (1-based).
    refs: HashMap<usize, i32>,
    count: i32,
    hook: Option<Root<'h>>,
    /// The effect points' results so far, by their position in the walk.
    effects: Vec<Global>,
    seq: usize,
    depth: usize,
}

/// `R_Serialize(s, stream)`: the header and the object, into `out`.
// R: serialize.c:R_Serialize
pub(super) fn serialize(cx: &mut Cx, s: Root<'_>, out: OutStream, hook: Option<Root<'_>>) -> RResult<Vec<u8>> {
    let mut w = Writer { out, refs: HashMap::new(), count: 0, hook, effects: Vec::new(), seq: 0, depth: 0 };
    // (a walk that starts over starts with `R_compile_pkgs` as the first
    // did: it is off while a closure compiled on write is written)
    let compile_pkgs = crate::prims::state(cx).compile_pkgs.get();
    let r = loop {
        crate::prims::state(cx).compile_pkgs.set(compile_pkgs);
        w.out.buf.clear();
        w.refs.clear();
        w.count = 0;
        w.seq = 0;
        w.depth = 0;
        let mut go = || -> W<()> {
            w.header(cx)?;
            let x = s.bind(cx).sexp();
            w.item(cx, x)
        };
        match go() {
            Ok(()) => break Ok(()),
            Err(WErr::R(e)) => break Err(e),
            Err(WErr::Restart) => continue,
        }
    };
    for g in w.effects.drain(..) {
        cx.release_global(g);
    }
    r.map(|()| w.out.buf)
}

impl Writer<'_> {
    /// `WriteBC(s)`: byte code, after the number of its language cells
    /// that are reached more than once (`ScanForCircles`) plus one; those
    /// are written once as `BCREPDEF` and then as `BCREPREF`.
    ///
    /// # Safety
    /// `s` is a live BCODESXP reachable from the rooted object, and nothing
    /// has collected since it was read.
    // R: serialize.c:WriteBC
    unsafe fn bc(&mut self, cx: &mut Cx, s: Sexp) -> W<()> {
        // SAFETY: the caller's contract.
        let mut reps = unsafe { scan_for_circles(cx, s) };
        self.out.integer(reps.len() as i32 + 1);
        let mut next = 0;
        // SAFETY: the caller's contract.
        unsafe { self.bc1(cx, s, &mut reps, &mut next) }
    }

    /// `WriteBC1(s)`: the code, decoded (`R_bcDecode`: robigo-rt keeps it
    /// decoded, so a plain integer vector of the same ints, written as one
    /// without allocating it), then the constants, byte code and language
    /// cells among them in their own representation.
    ///
    /// # Safety
    /// As for [`Writer::bc`].
    // R: serialize.c:WriteBC1, eval.c:R_bcDecode
    unsafe fn bc1(&mut self, cx: &mut Cx, s: Sexp, reps: &mut HashMap<usize, Option<i32>>, next: &mut i32) -> W<()> {
        // SAFETY: the caller's contract: a live BCODESXP, its CAR the code
        // (an INTSXP, or R_NilValue for empty code) and CDR the constants.
        unsafe {
            let code = s.car();
            let ints: &[i32] = if code.typ() == INTSXP {
                std::slice::from_raw_parts(code.std_data() as *const i32, code.std_len() as usize)
            } else {
                &[]
            };
            self.out.integer(pack_flags(INTSXP as i32, 0, false, false, false));
            self.write_length(ints.len());
            self.out.ints(ints);
            let consts = s.cdr();
            let n = if consts.typ() == VECSXP { consts.std_len() as usize } else { 0 };
            self.out.integer(n as i32);
            for i in 0..n {
                let c = *(consts.std_data() as *const Sexp).add(i);
                match c.typ() {
                    BCODESXP => {
                        self.out.integer(BCODESXP as i32);
                        self.bc1(cx, c, reps, next)?;
                    }
                    LANGSXP | LISTSXP => self.bc_lang(cx, c, reps, next)?,
                    t => {
                        self.out.integer(t as i32);
                        self.item(cx, c)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `WriteBCLang(s)`: a language or pairlist cell (its type, `ATTR...`
    /// with attributes, then the attributes, TAG, and CAR and CDR the same
    /// way), `BCREPDEF` the first time a shared one is written and
    /// `BCREPREF` after; anything else a pad and the item.
    ///
    /// # Safety
    /// As for [`Writer::bc`], `s` any object reachable from it.
    // R: serialize.c:WriteBCLang
    unsafe fn bc_lang(
        &mut self,
        cx: &mut Cx,
        s: Sexp,
        reps: &mut HashMap<usize, Option<i32>>,
        next: &mut i32,
    ) -> W<()> {
        cx.check_c_stack()?;
        // SAFETY: the caller's contract.
        unsafe {
            let t = s.typ();
            if !matches!(t, LANGSXP | LISTSXP) {
                self.out.integer(0);
                return self.item(cx, s);
            }
            if let Some(r) = reps.get_mut(&s.addr()) {
                match *r {
                    None => {
                        let i = *next;
                        *next += 1;
                        *r = Some(i);
                        self.out.integer(BCREPDEF);
                        self.out.integer(i);
                    }
                    Some(i) => {
                        self.out.integer(BCREPREF);
                        self.out.integer(i);
                        return Ok(());
                    }
                }
            }
            let nil = cx.nil().sexp();
            let attr = s.attrib_raw();
            let ty = match (t, attr != nil) {
                (LANGSXP, true) => ATTRLANGSXP,
                (LISTSXP, true) => ATTRLISTSXP,
                _ => t as i32,
            };
            self.out.integer(ty);
            if attr != nil {
                self.item(cx, attr)?;
            }
            self.item(cx, s.tag())?;
            self.bc_lang(cx, s.car(), reps, next)?;
            self.bc_lang(cx, s.cdr(), reps, next)
        }
    }

    // R: serialize.c:OutFormat, R_Serialize
    fn header(&mut self, cx: &mut Cx) -> W<()> {
        match self.out.fmt {
            Format::Ascii | Format::AsciiHex => self.out.bytes(b"A\n"),
            Format::Binary => self.out.bytes(b"B\n"),
            Format::Xdr => self.out.bytes(b"X\n"),
            Format::Any => return Err(cx.error("must specify ascii, binary, or xdr format").into()),
        }
        match self.out.version {
            2 => {
                self.out.integer(2);
                self.out.integer(R_VERSION);
                self.out.integer(r_version(2, 3, 0));
            }
            3 => {
                self.out.integer(3);
                self.out.integer(R_VERSION);
                self.out.integer(r_version(3, 5, 0));
                let enc = native_encoding(cx);
                self.out.integer(enc.len() as i32);
                self.out.string(enc.as_bytes());
            }
            v => return Err(cx.error(&format!("version {v} not supported")).into()),
        }
        Ok(())
    }

    /// Run the effect point `f` (or take its kept result): its result,
    /// raw, or a restart if it collected.
    fn effect(&mut self, cx: &mut Cx, f: impl FnOnce(&mut Cx, Out<'_>) -> RResult<()>) -> W<Sexp> {
        if self.seq < self.effects.len() {
            let x = self.effects[self.seq].bind(cx).sexp();
            self.seq += 1;
            return Ok(x);
        }
        let before = cx.gc_stats().collections;
        let g = cx.scope(|cx, s| {
            let r = s.capture(cx, f)?;
            Ok::<_, RError>(cx.global(r.bind(cx)))
        })?;
        self.effects.push(g);
        self.seq += 1;
        if cx.gc_stats().collections != before {
            return Err(WErr::Restart);
        }
        Ok(self.effects[self.seq - 1].bind(cx).sexp())
    }

    fn hash_add(&mut self, x: Sexp) {
        self.count += 1;
        self.refs.insert(x.addr(), self.count);
    }

    // R: serialize.c:OutRefIndex
    fn out_ref_index(&mut self, i: i32) {
        if i > MAX_PACKED_INDEX {
            self.out.integer(REFSXP);
            self.out.integer(i);
        } else {
            self.out.integer((i << 8) | REFSXP);
        }
    }

    // R: serialize.c:WriteLENGTH
    fn write_length(&mut self, n: usize) {
        if n > i32::MAX as usize {
            self.out.integer(-1);
            self.out.integer((n / 4294967296) as i32);
            self.out.integer((n % 4294967296) as u32 as i32);
        } else {
            self.out.integer(n as i32);
        }
    }

    // R: serialize.c:OutStringVec
    fn out_string_vec(&mut self, cx: &mut Cx, s: Sexp) -> W<()> {
        // SAFETY: a live STRSXP (checked by the caller); its elements are
        // CHARSXPs, written without collecting.
        unsafe {
            let n = s.std_len() as usize;
            self.out.integer(0);
            self.write_length(n);
            for i in 0..n {
                let c = *(s.std_data() as *const Sexp).add(i);
                self.item(cx, c)?;
            }
        }
        Ok(())
    }

    /// Whether `s` is a closure `R_compile_pkgs` compiles as it is written:
    /// its body not byte code, byte code not disabled, and not a reference
    /// class method or a default binding function (an S4 object of those
    /// classes), whose byte code would be dropped.
    ///
    /// # Safety
    /// `s` is live, reachable from the rooted object, and nothing has
    /// collected since it was read.
    // R: serialize.c:WriteItem
    unsafe fn compile_on_write(cx: &Cx, s: Sexp) -> bool {
        let st = crate::prims::state(cx);
        // SAFETY: the caller's contract; a closure's body is its CDR.
        unsafe {
            if st.compile_pkgs.get() == 0
                || s.typ() != CLOSXP
                || s.cdr().typ() == BCODESXP
                || st.jit.get().disable_bytecode
            {
                return false;
            }
            let x = Ref::from_sexp(s, cx);
            !x.is_s4()
                || !(robigo_rtbuiltins::attrib::inherits(x, b"refMethodDef")
                    || robigo_rtbuiltins::attrib::inherits(x, b"defaultBindingFunction"))
        }
    }

    /// Whether `s` is a closure robigo-rt's JIT has compiled (its own
    /// code, no BCODESXP for its body): GNU R's JIT has installed
    /// `R_cmpfun1`'s byte code as its BODY by now (eval.c:R_cmpfun), and
    /// serialize writes that.
    ///
    /// # Safety
    /// As for [`Self::compile_on_write`].
    unsafe fn jit_compiled(cx: &Cx, s: Sexp) -> bool {
        // SAFETY: the caller's contract; a closure's body is its CDR.
        unsafe {
            s.typ() == CLOSXP
                && crate::jit::compiled(s)
                && s.cdr().typ() != BCODESXP
                && !crate::prims::state(cx).jit.get().disable_bytecode
        }
    }

    /// `R_cmpfun1(s)` (an effect point: the compiler's R code runs): the
    /// compiled closure, raw, or `s` where the compile failed.
    fn cmpfun1(&mut self, cx: &mut Cx, s: Sexp) -> W<Sexp> {
        let f = cx.global(
            // SAFETY: live, reachable from the rooted object.
            unsafe { Ref::from_sexp(s, cx) },
        );
        let r = self.effect(cx, |cx, out| {
            cx.scope(|cx, sc| {
                let fr = sc.root(cx, f.bind(cx));
                crate::jit::cmpfun1(cx, fr, out)
            })
        });
        cx.release_global(f);
        r
    }

    /// `GetPersistentName(stream, s)`: the hook's STRSXP, or R_NilValue.
    // R: serialize.c:GetPersistentName, CallHook
    fn persistent_name(&mut self, cx: &mut Cx, s: Sexp) -> W<Sexp> {
        let Some(hook) = self.hook else { return Ok(cx.nil().sexp()) };
        // SAFETY: a live object.
        let t = unsafe { s.typ() };
        match t {
            WEAKREFSXP | EXTPTRSXP => {}
            ENVSXP => {
                if special_env(cx, s) || is_namespace_env(cx, s) || is_package_env(cx, s) {
                    return Ok(cx.nil().sexp());
                }
            }
            _ => return Ok(cx.nil().sexp()),
        }
        let obj = cx.global(
            // SAFETY: live, reachable from the rooted object.
            unsafe { Ref::from_sexp(s, cx) },
        );
        let r = self.effect(cx, |cx, out| {
            cx.scope(|cx, sc| {
                let call = sc.out(cx);
                let c = call.root();
                cx.build(call, |cx| cx.lang2(hook.bind(cx), obj.bind(cx)))?;
                let g = sc.root(cx, cx.global_env());
                cx.eval(c, g, out)
            })
        });
        cx.release_global(obj);
        r
    }

    // R: serialize.c:WriteItem
    pub(super) fn item(&mut self, cx: &mut Cx, mut s: Sexp) -> W<()> {
        self.depth += 1;
        if self.depth.is_multiple_of(64) {
            cx.check_c_stack()?;
        }
        let r = (|| -> W<()> {
            // SAFETY: `s` is reachable from the rooted object and nothing
            // has collected since it was read (an effect point that
            // collected restarts the walk).
            unsafe {
                // (a JIT-compiled closure the compiler declines: written as
                // its source)
                let mut source = None;
                loop {
                    if Self::compile_on_write(cx, s) {
                        // R_compile_pkgs off while the compiled closure is
                        // written (its environment's closures are written
                        // as they are), then TRUE; an error leaves it off
                        let st = crate::prims::state(cx);
                        st.compile_pkgs.set(0);
                        let new_s = self.cmpfun1(cx, s)?;
                        self.item(cx, new_s)?;
                        crate::prims::state(cx).compile_pkgs.set(1);
                        return Ok(());
                    }
                    // a closure robigo-rt's JIT compiled: GNU R's JIT has made
                    // its BODY R_cmpfun1's byte code, so the same compile now
                    // writes what GNU R writes (the closure cmpfun makes: the
                    // formals, the code, the environment and attributes; no
                    // JIT bits, as R_CheckJIT cleared MAYBEJIT)
                    if source != Some(s.addr()) && Self::jit_compiled(cx, s) {
                        let new_s = self.cmpfun1(cx, s)?;
                        if new_s.typ() == CLOSXP && new_s.cdr().typ() == BCODESXP {
                            return self.item(cx, new_s);
                        }
                        // R_cmpfun: the compiler declined, so GNU R's JIT left
                        // the source and set NOJIT
                        s.set_gp(s.gp() | crate::jit::NOJIT_MASK);
                        source = Some(s.addr());
                    }
                    if s.is_altrep() {
                        if robigo_rtcore::altrep::own::own_class(s).is_none() {
                            return Err(cx
                                .error("robigo: serializing an ALTREP object of a package class is not implemented yet")
                                .into());
                        }
                        // one of robigo's classes: ALTREP_SERIALIZED_CLASS
                        // (the class object's ATTRIB) and _STATE (a compact
                        // sequence's info, data1); before version 3 the
                        // standard form below, its elements read by element
                        if self.out.version >= 3 {
                            self.out.integer(pack_flags(ALTREP_SXP, s.gp(), s.is_object(), false, false));
                            self.item(cx, s.tag().attrib_raw())?;
                            self.item(cx, s.car())?;
                            return self.item(cx, s.attrib_raw());
                        }
                    }
                    let t = s.typ();
                    let pn = if matches!(t, WEAKREFSXP | EXTPTRSXP | ENVSXP) {
                        self.persistent_name(cx, s)?
                    } else {
                        cx.nil().sexp()
                    };
                    if pn != cx.nil().sexp() {
                        if pn.typ() != STRSXP || pn.std_len() <= 0 {
                            return Err(cx.error("robigo: a persistent name hook's value that is not a non-empty character vector is not implemented yet").into());
                        }
                        self.hash_add(s);
                        self.out.integer(PERSISTSXP);
                        return self.out_string_vec(cx, pn);
                    }
                    if let Some(i) = save_special_hook(cx, s) {
                        self.out.integer(i);
                        return Ok(());
                    }
                    if let Some(&i) = self.refs.get(&s.addr()) {
                        self.out_ref_index(i);
                        return Ok(());
                    }
                    if t == SYMSXP {
                        self.hash_add(s);
                        self.out.integer(SYMSXP as i32);
                        // the isolate's PRINTNAME (H5), with its encoding
                        let pname = cx.printname_raw(s);
                        return self.item(cx, pname);
                    }
                    if t == ENVSXP {
                        self.hash_add(s);
                        return self.env(cx, s);
                    }
                    let nil = cx.nil().sexp();
                    let hastag = match t {
                        LISTSXP | LANGSXP | PROMSXP | DOTSXP => s.tag() != nil,
                        CLOSXP => true,
                        _ => false,
                    };
                    let hasattr = t != CHARSXP && s.attrib_raw() != nil;
                    if t == PROMSXP
                        && s.extra() & robigo_heap::extra::promise::BYTECODE != 0
                        && s.cdr().typ() != BCODESXP
                    {
                        return Err(cx
                            .error("robigo: serializing a promise of byte code is not implemented yet")
                            .into());
                    }
                    let flags = pack_flags(t as i32, s.gp(), s.is_object(), hasattr, hastag);
                    self.out.integer(flags);
                    match t {
                        LISTSXP | LANGSXP | PROMSXP | DOTSXP => {
                            if hasattr {
                                self.item(cx, s.attrib_raw())?;
                            }
                            if s.tag() != nil {
                                self.item(cx, s.tag())?;
                            }
                            let car =
                                if t == PROMSXP && s.promise_tag() != 0 { self.boxed_value(cx, s)? } else { s.car() };
                            self.item(cx, car)?;
                            s = s.cdr();
                            continue;
                        }
                        CLOSXP => {
                            if hasattr {
                                self.item(cx, s.attrib_raw())?;
                            }
                            self.item(cx, s.tag())?;
                            self.item(cx, s.car())?;
                            s = s.cdr();
                            continue;
                        }
                        EXTPTRSXP => {
                            self.hash_add(s);
                            self.item(cx, s.cdr())?;
                            self.item(cx, s.tag())?;
                        }
                        WEAKREFSXP => self.hash_add(s),
                        SPECIALSXP | BUILTINSXP => {
                            let name = crate::prims::prim_name(cx, Ref::from_sexp(s, cx)).unwrap_or("");
                            self.out.integer(name.len() as i32);
                            self.out.string(name.as_bytes());
                        }
                        CHARSXP => {
                            if s == cx.na_string().as_ref().sexp() {
                                self.out.integer(-1);
                            } else {
                                let b = s.char_bytes();
                                self.out.integer(b.len() as i32);
                                self.out.string(b);
                            }
                        }
                        INTSXP if s.is_altrep() => {
                            let n = robigo_rtcore::Ref::from_sexp(s, cx).len().unwrap_or(0);
                            self.write_length(n);
                            let xs: Vec<i32> = (0..n).map(|i| robigo_rtcore::altrep::own::intseq_elt(s, i)).collect();
                            self.out.ints(&xs);
                        }
                        REALSXP if s.is_altrep() => {
                            let n = robigo_rtcore::Ref::from_sexp(s, cx).len().unwrap_or(0);
                            self.write_length(n);
                            let xs: Vec<f64> = (0..n).map(|i| robigo_rtcore::altrep::own::realseq_elt(s, i)).collect();
                            self.out.reals(&xs);
                        }
                        LGLSXP | INTSXP => {
                            let n = s.std_len() as usize;
                            self.write_length(n);
                            let xs = std::slice::from_raw_parts(s.std_data() as *const i32, n);
                            self.out.ints(xs);
                        }
                        REALSXP => {
                            let n = s.std_len() as usize;
                            self.write_length(n);
                            let xs = std::slice::from_raw_parts(s.std_data() as *const f64, n);
                            self.out.reals(xs);
                        }
                        CPLXSXP => {
                            let n = s.std_len() as usize;
                            self.write_length(n);
                            let xs = std::slice::from_raw_parts(s.std_data() as *const f64, 2 * n);
                            self.out.reals(xs);
                        }
                        STRSXP | VECSXP | EXPRSXP => {
                            let n = s.std_len() as usize;
                            self.write_length(n);
                            for i in 0..n {
                                let e = *(s.std_data() as *const Sexp).add(i);
                                self.item(cx, e)?;
                            }
                        }
                        BCODESXP => self.bc(cx, s)?,
                        RAWSXP => {
                            let n = s.std_len() as usize;
                            self.write_length(n);
                            let xs = std::slice::from_raw_parts(s.std_data() as *const u8, n);
                            match self.out.fmt {
                                Format::Xdr | Format::Binary => self.out.bytes(xs),
                                _ => {
                                    for &b in xs {
                                        self.out.byte(b);
                                    }
                                }
                            }
                        }
                        OBJSXP => {}
                        _ => return Err(cx.error(&format!("WriteItem: unknown type {t}")).into()),
                    }
                    if hasattr {
                        self.item(cx, s.attrib_raw())?;
                    }
                    return Ok(());
                }
            }
        })();
        self.depth -= 1;
        r
    }

    /// A promise's unboxed PRVALUE, boxed (an effect point: it allocates).
    fn boxed_value(&mut self, cx: &mut Cx, p: Sexp) -> W<Sexp> {
        // SAFETY: live, reachable from the rooted object.
        let g = cx.global(unsafe { Ref::from_sexp(p, cx) });
        let r = self.effect(cx, |cx, out| {
            cx.scope(|cx, s| {
                let p = s.root(cx, g.bind(cx));
                cx.promise_value(p, out).map(|_| ())
            })
        });
        cx.release_global(g);
        r
    }

    // R: serialize.c:WriteItem (ENVSXP)
    fn env(&mut self, cx: &mut Cx, s: Sexp) -> W<()> {
        if is_package_env(cx, s) {
            // SAFETY: live, reachable from the rooted object.
            let name = unsafe { package_env_name(cx, s) };
            let shown = String::from_utf8_lossy(&name_text(cx, name)).into_owned();
            self.effect(cx, |cx, out| {
                cx.warning(&format!("'{shown}' may not be available when loading"))?;
                out.set(cx, cx.nil());
                Ok(())
            })?;
            // SAFETY: as above (the effect did not collect).
            let name = unsafe { package_env_name(cx, s) };
            self.out.integer(PACKAGESXP);
            return self.out_string_vec(cx, name);
        }
        if is_namespace_env(cx, s) {
            // SAFETY: live, reachable from the rooted object.
            let spec = unsafe { namespace_env_spec(cx, s) };
            self.out.integer(NAMESPACESXP);
            return self.out_string_vec(cx, spec);
        }
        self.out.integer(ENVSXP as i32);
        // SAFETY: a live environment reachable from the rooted object; the
        // frame snapshot is kept by its effect point's global.
        unsafe {
            self.out.integer((s.gp() & FRAME_LOCK_MASK != 0) as i32);
            self.item(cx, robigo_heap::env::enclos(s))?;
            let r = Ref::from_sexp(s, cx);
            let frame = if cx.env_is_hashed(r) {
                robigo_heap::env::frame(s)
            } else {
                let g = cx.global(r);
                let f = self.effect(cx, |cx, out| {
                    cx.scope(|cx, sc| {
                        let e = sc.root(cx, g.bind(cx));
                        cx.frame_snapshot(e, out)
                    })
                });
                cx.release_global(g);
                f?
            };
            self.item(cx, frame)?;
            let table = cx.env_hashtab(Ref::from_sexp(s, cx)).sexp();
            self.item(cx, table)?;
            self.item(cx, s.attrib_raw())?;
        }
        Ok(())
    }
}

fn name_text(cx: &Cx, name: Sexp) -> Vec<u8> {
    // SAFETY: a live STRSXP of at least one element.
    unsafe {
        let c = *(name.std_data() as *const Sexp);
        let c = Ref::from_sexp(c, cx).as_char().unwrap();
        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
            .map(|b| b.into_owned())
            .unwrap_or_else(|_| c.bytes().to_vec())
    }
}

/// `ScanForCircles(s)`: the language and pairlist cells reached more than
/// once from byte code `s` (through CAR and CDR, and the constants of byte
/// code), each with no index yet. A cell is entered once and descended
/// into the first time it is reached, so which cells are found does not
/// depend on the order of the walk: an explicit stack walks long lists.
///
/// # Safety
/// `s` is live, and nothing collects while the walk runs (it allocates
/// nothing on the heap).
// R: serialize.c:ScanForCircles, ScanForCircles1, AddCircleHash
unsafe fn scan_for_circles(cx: &Cx, s: Sexp) -> HashMap<usize, Option<i32>> {
    let _ = cx;
    let mut seen: HashMap<usize, bool> = HashMap::new();
    let mut stack = vec![s];
    while let Some(x) = stack.pop() {
        // SAFETY: the caller's contract: everything reached is live.
        unsafe {
            match x.typ() {
                LANGSXP | LISTSXP => match seen.get_mut(&x.addr()) {
                    Some(twice) => *twice = true,
                    None => {
                        seen.insert(x.addr(), false);
                        stack.push(x.cdr());
                        stack.push(x.car());
                    }
                },
                BCODESXP => {
                    let consts = x.cdr();
                    if consts.typ() == VECSXP {
                        for i in (0..consts.std_len() as usize).rev() {
                            stack.push(*(consts.std_data() as *const Sexp).add(i));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    seen.into_iter().filter(|&(_, twice)| twice).map(|(a, _)| (a, None)).collect()
}
