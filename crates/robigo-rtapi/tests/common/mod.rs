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
        // robigo_layout.h, which robigo-heap's build writes (Defn.h reads it)
        format!("-I{}", env!("ROBIGO_LAYOUT_INCLUDE")),
        format!("-I{}", rsrc().join("src/include").display()),
    ]
}

/// `tests/support/capi_test.c` built into a shared object, named by what
/// goes into it (the source, robigo's headers, the flags): the test
/// binaries' processes and later runs reuse one file, and a change builds
/// a new one.
pub fn support_so() -> &'static Path {
    static SO: OnceLock<PathBuf> = OnceLock::new();
    SO.get_or_init(|| {
        let src = root().join("crates/robigo-rtapi/tests/support/capi_test.c");
        let key = support_key(&src, &[]);
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("capi_test-{key:016x}.so"));
        build_support(&src, &[], &out);
        out
    })
}

/// `tests/support/<src>` built with `defines` (`-D...` flags) into
/// `<name>.so`, in a directory named by what goes into it: `dyn.load` names
/// an object by its file (`R_init_<name>`), so the name is the file's.
pub fn support_object(src: &str, name: &str, defines: &[&str]) -> PathBuf {
    let src = root().join("crates/robigo-rtapi/tests/support").join(src);
    let defines: Vec<String> = defines.iter().map(|d| d.to_string()).collect();
    let key = support_key(&src, &defines);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("capi_test-{name}-{key:016x}"));
    // (made in build_support, under the lock: another test's cleanup, made
    // under it too, would otherwise see the new directory empty and remove it)
    let out = dir.join(format!("{name}.so"));
    build_support(&src, &defines, &out);
    out
}

/// The compiler's own arguments, before the output's name.
const CC_ARGS: [&str; 4] = ["-fPIC", "-shared", "-O1", "-g"];

/// What a support object is built from, hashed: the source, robigo's
/// headers, the flags, the compiler (as named and as it reports itself) and
/// the pinned GNU R tree whose headers the flags name (its stamp).
fn support_key(src: &Path, extra: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let flags = include_flags();
    let mut h = std::hash::DefaultHasher::new();
    let mut inputs = vec![src.to_path_buf()];
    let layout = PathBuf::from(env!("ROBIGO_LAYOUT_INCLUDE"));
    for dir in [root().join("crates/robigo-rtapi/include"), root().join("crates/robigo-rtcore/include"), layout] {
        let mut files: Vec<PathBuf> =
            std::fs::read_dir(&dir).expect("reading robigo's headers").map(|e| e.unwrap().path()).collect();
        files.sort();
        inputs.extend(files);
    }
    for f in &inputs {
        std::fs::read(f).unwrap_or_else(|e| panic!("reading {}: {e}", f.display())).hash(&mut h);
    }
    flags.hash(&mut h);
    extra.hash(&mut h);
    CC_ARGS.hash(&mut h);
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    cc.hash(&mut h);
    let version = std::process::Command::new(&cc).arg("--version").output().expect("running cc --version");
    version.stdout.hash(&mut h);
    std::fs::read(rsrc().join(".rsrc-pinned")).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Build `src` with `extra` flags into `out` unless it is there.
fn build_support(src: &Path, extra: &[String], out: &Path) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    // the lookup, the refresh, a build and the cleanup under one lock of
    // the directory's, which other processes take for theirs: a cleanup
    // never removes an object between another run's lookup and refresh
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("capi_test.lock"))
        .expect("opening the support objects' lock");
    lock.lock().expect("locking the support objects");
    std::fs::create_dir_all(out.parent().expect("a support object's directory"))
        .expect("making a support object's directory");
    if out.exists() {
        // each use dates the object (before it is loaded): the cleanup
        // below removes only what no run has used for a day
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(out) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
    } else {
        // (built under a name of this process's, then renamed: another
        // process may build the same object at once)
        let tmp = PathBuf::from(format!("{}.{}", out.display(), std::process::id()));
        let st = std::process::Command::new(&cc)
            .args(CC_ARGS)
            .arg("-o")
            .arg(&tmp)
            .args(include_flags())
            .args(extra)
            .arg(src)
            .status()
            .expect("running cc");
        assert!(st.success(), "building {}", src.display());
        // (a rename that fails while another process's identical object
        // is in place leaves a usable one)
        if let Err(e) = std::fs::rename(&tmp, out) {
            let _ = std::fs::remove_file(&tmp);
            assert!(out.exists(), "renaming {}: {e}", tmp.display());
        }
        remove_stale_objects(dir, out);
    }
    drop(lock);
}

/// The support objects of other keys that no run has used for a day
/// removed, once a new one is built (under the directory's lock): the
/// directory is the target directory's, which checkouts share, and a run
/// dates the object it uses, under the lock, before loading it, so an
/// object another checkout uses stays. A directory of [`support_object`]'s
/// goes with its object.
pub fn remove_stale_objects(dir: &Path, keep: &Path) {
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let old = |p: &Path| {
        std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > day)
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p == keep || keep.starts_with(&p) || !name.starts_with("capi_test-") {
            continue;
        }
        if p.is_dir() {
            let objects: Vec<PathBuf> =
                std::fs::read_dir(&p).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            // (an empty directory is dated by itself: `all` of none is true,
            // and a directory made a moment ago must stay)
            let stale = if objects.is_empty() { old(&p) } else { objects.iter().all(|o| old(o)) };
            if stale {
                let _ = std::fs::remove_dir_all(&p);
            }
        } else if old(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
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
        // (the path as an R string literal)
        let so = support_so().display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
        s.val(&format!("invisible(dyn.load(\"{so}\"))"));
        // (the library's log is per thread: a test that failed before it
        // read the log must not leave entries for the next one)
        s.val("invisible(.Call(\"rt_log_take\"))");
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
