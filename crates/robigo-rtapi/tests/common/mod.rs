//! A session for the C API's tests: an isolate with the evaluator, the C
//! API and base (GNU R 4.6.2's own R code from the pinned tree), its console
//! captured, and `tests/support/capi_test.c` loaded: built once per test
//! binary with `cc -fPIC -shared` against GNU R's public headers, robigo's
//! `Rinternals.h` in front, as a package is.

#![allow(dead_code)]

use robigo_rt::Profile;
use robigo_rtcore::{Console, Cx, OwnedIsolate, RError, Ref, SharedBuf, View};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The pinned GNU R tree (`scripts/r-src.sh`; `ROBIGO_RSRC` overrides).
pub fn rsrc() -> PathBuf {
    robigo_rt::base::source_dir().unwrap_or_else(|e| panic!("{e} (run scripts/r-src.sh)"))
}

/// The include path a package is built with.
pub fn include_flags() -> Vec<String> {
    let r = root();
    vec![
        format!("-I{}", r.join("crates/robigo-rtapi/include").display()),
        format!("-I{}", r.join("crates/robigo-rtcore/include").display()),
        format!("-I{}", rsrc().join("src/include").display()),
    ]
}

/// `tests/support/<name>.c` built into a shared object (once per binary).
pub fn support_so() -> &'static Path {
    static SO: OnceLock<PathBuf> = OnceLock::new();
    SO.get_or_init(|| {
        let src = root().join("crates/robigo-rtapi/tests/support/capi_test.c");
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("capi_test-{}.so", std::process::id()));
        let st = std::process::Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
            .args(["-fPIC", "-shared", "-O1", "-g", "-o"])
            .arg(&out)
            .args(include_flags())
            .arg(&src)
            .status()
            .expect("running cc");
        assert!(st.success(), "building {}", src.display());
        out
    })
}

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

pub fn int(x: i32) -> Val {
    Val::Int(vec![x])
}

pub fn str1(x: &str) -> Val {
    Val::Str(vec![x.to_owned()])
}

/// The message of the pending error `e`.
pub fn peek_message(cx: &Cx, e: &RError) -> String {
    let (v, _, _) = cx.exit_objects(e);
    match v.as_vector::<robigo_rtcore::Char<'_>>() {
        Some(s) if s.len() == 1 => String::from_utf8_lossy(s.get(0).bytes()).into_owned(),
        _ => format!("<exit {:?}>", cx.exit_info(e)),
    }
}

pub struct Session {
    pub iso: OwnedIsolate,
    pub out: SharedBuf,
    pub err: SharedBuf,
}

impl Session {
    /// A session in profile `p` with the support library loaded.
    pub fn new(p: Profile) -> Session {
        let mut iso = OwnedIsolate::new();
        let (console, out, err) = Console::capture();
        let cx = iso.cx();
        cx.replace_console(console);
        robigo_rt::install_with(cx, Some(p));
        robigo_rtapi::install(cx);
        robigo_rt::base::load(cx, &rsrc()).expect("loading base");
        let mut s = Session { iso, out, err };
        let so = support_so().display().to_string();
        s.val(&format!("invisible(dyn.load(\"{so}\"))"));
        s
    }

    pub fn cx(&mut self) -> &mut Cx {
        self.iso.cx()
    }

    /// Evaluate every expression of `src` in the global environment, each
    /// in a top-level context as the REPL does: the last value, or the
    /// first error's message.
    pub fn run(&mut self, src: &str) -> Result<Val, String> {
        let cx = self.iso.cx();
        cx.scope(|cx, s| {
            let ex = s.out(cx);
            let exr = ex.root();
            if let Err(e) = robigo_rt::lower::parse(cx, src, ex) {
                let m = peek_message(cx, &e);
                cx.resolve_exit(e, None);
                return Err(m);
            }
            let n = exr.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
            let g = s.root(cx, cx.global_env());
            let base = s.root(cx, cx.base_env());
            let mut last = None;
            for i in 0..n {
                let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                let o = s.out(cx);
                let or = o.root();
                let mut msg = None;
                let done = cx
                    .toplevel(g, base, |cx| match robigo_rt::eval(cx, e, g, o) {
                        Ok(()) => Ok(()),
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
            Ok(last.map_or(Val::Null, |r| describe(r.bind(cx))))
        })
    }

    pub fn val(&mut self, src: &str) -> Val {
        match self.run(src) {
            Ok(v) => v,
            Err(m) => panic!("R error: {m}\nin:\n{src}"),
        }
    }

    pub fn err(&mut self, src: &str) -> String {
        match self.run(src) {
            Ok(v) => panic!("no error, value {v:?}\nin:\n{src}"),
            Err(m) => m,
        }
    }

    /// What the support library logged since the last call (`rt_log_take`).
    pub fn log(&mut self) -> Vec<String> {
        match self.val(".Call(\"rt_log_take\")") {
            Val::Str(v) => v[0].split(';').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            v => panic!("rt_log_take returned {v:?}"),
        }
    }

    /// The console's error stream since the last call.
    pub fn stderr(&mut self) -> String {
        self.iso.cx().flush_console();
        String::from_utf8_lossy(&self.err.take()).into_owned()
    }
}
