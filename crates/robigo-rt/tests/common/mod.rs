//! A small R session for the evaluator's tests: source text parsed and
//! lowered (`robigo_rt::lower`), every expression evaluated in the global
//! environment, the last value described. A few test builtins stand in for
//! base functions other packages implement (`:`, `new.env`, `reg.finalizer`,
//! `gc`, `cat`-like recording).

#![allow(dead_code)]

use robigo_rt::{Profile, install_with};
use robigo_rtcore::{Args, Cx, Out, OwnedIsolate, RError, RResult, Ref, Root, View};
use std::sync::Mutex;

/// A described value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Real(Vec<f64>),
    Int(Vec<i32>),
    Lgl(Vec<i32>),
    Str(Vec<String>),
    List(Vec<Val>),
    Other(u8),
}

pub const NA: i32 = i32::MIN;

/// What the test builtin `record()` saw, per thread (each test runs on its
/// own thread).
static LOG: Mutex<Vec<(std::thread::ThreadId, String)>> = Mutex::new(Vec::new());

pub fn log_take() -> Vec<String> {
    let id = std::thread::current().id();
    let mut l = LOG.lock().unwrap();
    let (mine, rest): (Vec<_>, Vec<_>) = l.drain(..).partition(|(t, _)| *t == id);
    *l = rest;
    mine.into_iter().map(|(_, s)| s).collect()
}

fn log(s: String) {
    LOG.lock().unwrap().push((std::thread::current().id(), s));
}

pub struct Session {
    pub iso: OwnedIsolate,
}

impl Session {
    pub fn new(p: Profile) -> Session {
        let mut iso = OwnedIsolate::new();
        let cx = iso.cx();
        install_with(cx, Some(p));
        robigo_rt::define_builtin(cx, ":", colon, 1, 2);
        robigo_rt::define_builtin(cx, "record", record, 101, 1);
        robigo_rt::define_builtin(cx, "new.env", new_env, 1, 0);
        robigo_rt::define_builtin(cx, "reg.finalizer", reg_finalizer, 101, 2);
        robigo_rt::define_builtin(cx, "gc", gc, 101, 0);
        robigo_rt::define_builtin(cx, "keep", keep, 1, 1);
        robigo_rt::define_builtin(cx, "count", count, 1, -1);
        robigo_rt::define_builtin(cx, "list", list, 1, -1);
        robigo_rt::define_builtin(cx, "c", c, 1, -1);
        robigo_rt::define_builtin(cx, "length", length, 1, 1);
        robigo_rt::define_builtin(cx, "addr", addr, 1, 1);
        robigo_rt::define_builtin(cx, "pairlist", pairlist, 1, -1);
        robigo_rt::define_builtin(cx, "refcnt", refcnt, 1, 1);
        log_take();
        Session { iso }
    }

    pub fn cx(&mut self) -> &mut Cx {
        self.iso.cx()
    }

    /// Evaluate every expression of `src`; the last value and `R_Visible`,
    /// or the first error's message.
    pub fn run(&mut self, src: &str) -> Result<(Val, bool), String> {
        let cx = self.iso.cx();
        cx.scope(|cx, s| {
            let ex = s.out(cx);
            let exr = ex.root();
            if let Err(e) = robigo_rt::lower::parse(cx, src, ex) {
                return Err(message(cx, e));
            }
            let n = exr.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
            let g = s.root(cx, cx.global_env());
            let base = s.root(cx, cx.base_env());
            let mut last = None;
            let mut vis = true;
            for i in 0..n {
                let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                let o = s.out(cx);
                let or = o.root();
                // as GNU R's REPL: an error lands at the top level, which
                // restores the globals an exit's stop restores
                let mut msg = None;
                let done = cx
                    .toplevel(g, base, |cx| match robigo_rt::eval(cx, e, g, o) {
                        Ok(()) => {
                            vis = cx.visible();
                            Ok(())
                        }
                        Err(e) => {
                            msg = Some(peek_message(cx, &e));
                            Err(e)
                        }
                    })
                    .expect("an exit that passed the top level");
                if !done {
                    return Err(msg.unwrap_or_default());
                }
                last = Some(or);
            }
            let v = last.map_or(Val::Null, |r| describe(r.bind(cx)));
            Ok((v, vis))
        })
    }

    /// [`Session::run`]'s value; panics on an error.
    pub fn val(&mut self, src: &str) -> Val {
        match self.run(src) {
            Ok((v, _)) => v,
            Err(m) => panic!("R error: {m}\nin:\n{src}"),
        }
    }

    /// [`Session::run`]'s error; panics if there is none.
    pub fn err(&mut self, src: &str) -> String {
        match self.run(src) {
            Ok((v, _)) => panic!("no error, value {v:?}\nin:\n{src}"),
            Err(m) => m,
        }
    }
}

/// The message of an exit that reached the top (its record resolved), with
/// its call when it has one: "Error in <call head>: msg" is S1's to format,
/// so the call is given as its function's name.
pub fn message(cx: &mut Cx, e: RError) -> String {
    let m = peek_message(cx, &e);
    cx.resolve_exit(e, None);
    m
}

/// [`message`] without resolving the exit.
pub fn peek_message(cx: &Cx, e: &RError) -> String {
    let (v, _, call) = cx.exit_objects(e);
    let msg = match v.as_vector::<robigo_rtcore::Char<'_>>() {
        Some(s) if s.len() == 1 => String::from_utf8_lossy(s.get(0).bytes()).into_owned(),
        _ => format!("<exit {:?}>", cx.exit_info(e)),
    };
    let head =
        call.as_cons().and_then(|c| c.car().as_sym()).map(|s| String::from_utf8_lossy(s.name().bytes()).into_owned());
    match head {
        Some(h) => format!("[{h}] {msg}"),
        None => msg,
    }
}

pub fn describe(x: Ref<'_>) -> Val {
    match x.view() {
        View::Null => Val::Null,
        View::Real(v) => Val::Real(v.as_slice().unwrap().to_vec()),
        View::Int(v) => Val::Int(v.as_slice().unwrap().to_vec()),
        View::Lgl(v) => Val::Lgl(v.as_slice().unwrap().iter().map(|l| l.0).collect()),
        View::Str(v) => {
            Val::Str(v.as_slice().unwrap().iter().map(|c| String::from_utf8_lossy(c.bytes()).into_owned()).collect())
        }
        View::List(v) => Val::List(v.as_slice().unwrap().iter().map(|&e| describe(e)).collect()),
        _ => Val::Other(x.sexptype()),
    }
}

pub fn real(x: f64) -> Val {
    Val::Real(vec![x])
}

pub fn int(x: i32) -> Val {
    Val::Int(vec![x])
}

pub fn lgl(x: bool) -> Val {
    Val::Lgl(vec![x as i32])
}

pub fn str1(x: &str) -> Val {
    Val::Str(vec![x.to_owned()])
}

fn number(x: Ref<'_>) -> Option<f64> {
    match x.view() {
        View::Real(v) if v.len() == 1 => Some(v.get(0)),
        View::Int(v) if v.len() == 1 => Some(v.get(0) as f64),
        _ => None,
    }
}

/// `from:to` for numbers that fit an integer.
fn colon(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let (Some(a), Some(b)) = (number(args.get(cx, 0)), number(args.get(cx, 1))) else {
        return Err(cx.error_call(Some(call), "test ':' takes two numbers"));
    };
    let (a, b) = (a as i32, b as i32);
    let v: Vec<i32> = if a <= b { (a..=b).collect() } else { (b..=a).rev().collect() };
    cx.build(out, |cx| Ok(cx.alloc_copy(&v)?.finish().into()))
}

/// `record(x)`: log a description of `x`; invisible NULL.
fn record(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let s = match describe(args.get(cx, 0)) {
        Val::Str(v) => v.join(" "),
        v => format!("{v:?}"),
    };
    log(s);
    let nil = cx.nil();
    out.set(cx, nil);
    Ok(())
}

/// `new.env()`: a hashed environment enclosed by the caller's.
fn new_env(cx: &mut Cx, _call: Root<'_>, _args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| cx.new_env(env.bind(cx), true, 29))
}

/// `reg.finalizer(e, f)`.
fn reg_finalizer(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    robigo_rt::fin::register_finalizer(cx, args.root(0), args.root(1), false)?;
    let nil = cx.nil();
    out.set(cx, nil);
    Ok(())
}

/// `gc()`: a full collection, then the pending finalizers.
fn gc(cx: &mut Cx, _call: Root<'_>, _args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    robigo_rt::fin::gc(cx);
    let nil = cx.nil();
    out.set(cx, nil);
    Ok(())
}

/// `keep(x)`: `x` (an identity builtin).
fn keep(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let x = args.get(cx, 0);
    out.set(cx, x);
    Ok(())
}

/// `count(...)`: how many arguments it got (an integer).
fn count(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len() as i32;
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| n)?.finish().into()))
}

/// The names of a window's arguments as a character vector, if any.
fn arg_names<'g>(cx: &'g Cx, args: Args<'_>) -> Result<Option<Ref<'g>>, robigo_rtcore::NeedGc> {
    if (0..args.len()).all(|i| args.name(cx, i).is_none()) {
        return Ok(None);
    }
    let mut nm = cx.alloc_vector::<robigo_rtcore::Char<'g>>(args.len())?;
    for i in 0..args.len() {
        nm.set(i, args.name(cx, i).map_or(cx.blank_string(), |s| s.name()));
    }
    Ok(Some(nm.finish().into()))
}

/// `list(...)` (a test stand-in for base's): the arguments, named by their
/// tags.
fn list(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Ref<'_>>(args.len())?;
        for i in 0..args.len() {
            v.set(i, args.get(cx, i));
        }
        if let Some(nm) = arg_names(cx, args)? {
            let tag = cx.install(b"names")?;
            v.set_attrib(cx.cons_tagged(nm, cx.nil(), tag)?);
        }
        Ok(v.finish().into())
    })
}

/// `c(...)` (a test stand-in): scalars of one kind (integer, double,
/// character or logical) concatenated, named by their tags.
fn c(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let ty = (0..args.len()).map(|i| args.get(cx, i).sexptype()).max().unwrap_or(0);
    if (0..args.len()).any(|i| args.get(cx, i).len() != Some(1)) {
        return Err(cx.error_call(Some(call), "test 'c' takes scalars"));
    }
    // one kind: character alone, or logical, integer and double mixed
    let numeric = |t: u8| matches!(t, 10 | 13 | 14);
    let kinds_ok = match ty {
        16 => (0..args.len()).all(|i| args.get(cx, i).sexptype() == 16),
        10 | 13 | 14 => (0..args.len()).all(|i| numeric(args.get(cx, i).sexptype())),
        _ => args.is_empty(),
    };
    if !kinds_ok {
        return Err(cx.error_call(Some(call), "test 'c' takes scalars of one kind"));
    }
    cx.build(out, |cx| {
        let n = args.len();
        let mut attrs = cx.nil();
        if let Some(nm) = arg_names(cx, args)? {
            let tag = cx.install(b"names")?;
            attrs = cx.cons_tagged(nm, cx.nil(), tag)?;
        }
        let num = |i: usize| match args.get(cx, i).view() {
            View::Real(v) => v.get(0),
            View::Int(v) if v.get(0) == NA => f64::from_bits(0x7FF00000000007A2),
            View::Int(v) => v.get(0) as f64,
            View::Lgl(v) if v.get(0).0 == NA => f64::from_bits(0x7FF00000000007A2),
            View::Lgl(v) => v.get(0).0 as f64,
            _ => f64::NAN,
        };
        Ok(match ty {
            16 => {
                let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(n)?;
                for i in 0..n {
                    v.set(i, args.get(cx, i).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(0));
                }
                v.set_attrib(attrs);
                v.finish().into()
            }
            14 => {
                let mut v = cx.alloc_with(n, num)?;
                v.set_attrib(attrs);
                v.finish().into()
            }
            13 => {
                let mut v = cx.alloc_with(n, |i| args.get(cx, i).as_ints().unwrap()[0])?;
                v.set_attrib(attrs);
                v.finish().into()
            }
            _ => {
                let mut v = cx.alloc_with(n, |i| robigo_rtcore::Lgl(args.get(cx, i).as_ints().unwrap()[0]))?;
                v.set_attrib(attrs);
                v.finish().into()
            }
        })
    })
}

/// `length(x)` (a test stand-in) of a vector or NULL.
fn length(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.get(cx, 0).len().unwrap_or(1) as i32;
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| n)?.finish().into()))
}

/// `addr(x)` (a test stand-in for `.Internal(inspect(x))`'s address): the
/// object's address as a double.
fn addr(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let a = args.get(cx, 0).sexp().0 as usize as f64;
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| a)?.finish().into()))
}

/// `pairlist(...)` (a test stand-in): the arguments as a pairlist, tagged.
fn pairlist(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        let mut l = cx.nil();
        for i in (0..args.len()).rev() {
            l = match args.name(cx, i) {
                Some(t) => cx.cons_tagged(args.get(cx, i), l, t)?,
                None => cx.cons(args.get(cx, i), l)?,
            };
        }
        Ok(l)
    })
}

/// `refcnt(x)` (a test probe): GNU R's REFCNT of `x` (the argument window
/// counts none).
fn refcnt(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.get(cx, 0).refcnt() as i32;
    cx.build(out, |cx| Ok(cx.alloc_with(1, |_| n)?.finish().into()))
}
