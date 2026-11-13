//! A small R session for the evaluator's tests: source text parsed and
//! lowered (`robigo_rt::lower`), every expression evaluated in the global
//! environment, the last value described. A few test builtins stand in for
//! base functions other packages implement (`:`, `new.env`, `reg.finalizer`,
//! `gc`, `cat`-like recording).

#![allow(dead_code)]

use robigo_rt::{Profile, install_with};
use robigo_rtcore::{Args, Cx, OwnedIsolate, Out, RError, RResult, Ref, Root, View};
use std::sync::Mutex;

/// A described value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Real(Vec<f64>),
    Int(Vec<i32>),
    Lgl(Vec<i32>),
    Str(Vec<String>),
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
    let head = call
        .as_cons()
        .and_then(|c| c.car().as_sym())
        .map(|s| String::from_utf8_lossy(s.name().bytes()).into_owned());
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
        View::Str(v) => Val::Str(v.as_slice().unwrap().iter().map(|c| String::from_utf8_lossy(c.bytes()).into_owned()).collect()),
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
