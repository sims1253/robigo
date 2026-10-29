//! What GNU R does at startup after loading base: environment files, the
//! system, site and user profiles, locking the base namespace, `.First`
//! and `.First.sys` (GNU R's src/unix/system.c, src/main/startup.c,
//! src/main/Renviron.c and `setup_Rmainloop` in src/main/main.c).
//!
//! robigo runs GNU R's own R code, so it uses an installed GNU R for what lives
//! outside base: the package library and `etc/` files under `R_HOME`.

use std::path::Path;

use crate::env::Env;
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::{Arg, Value};

/// The system profile (`library/base/R/Rprofile` in an R installation),
/// vendored as `rlib/profile`.
static PROFILE_SOURCES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/profile_sources.rs"));

/// R's `--no-environ`, `--no-site-file` and `--no-init-file`
/// (`--vanilla` sets all three).
#[derive(Default, Clone, Copy)]
pub struct StartupOptions {
    pub no_environ: bool,
    pub no_site_file: bool,
    pub no_init_file: bool,
}

fn setenv(name: &str, value: &str) {
    // SAFETY: called during single-threaded startup or from the R
    // evaluator, which is single-threaded.
    unsafe { std::env::set_var(name, value) }
}

/// robigo's own `R_HOME` (`scripts/robigo-home.sh`): `home/` beside the
/// directory of the executable (`.robigo/bin/robigo`, `.robigo/home`),
/// when it is there.
pub fn robigo_home() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let home = exe.parent()?.parent()?.join("home");
    home.join("etc/Makeconf").exists().then(|| home.to_string_lossy().into_owned())
}

/// Locate the GNU R installation: `R_HOME`, or a common install location.
/// Sets `R_HOME` and the directories R's front-end script (`bin/R`)
/// exports, unless they are already set.
pub fn init_r_home() -> Option<String> {
    let home = std::env::var("R_HOME").ok().filter(|h| !h.is_empty()).or_else(robigo_home).or_else(|| {
        [
            "/usr/lib/R",
            "/usr/lib64/R",
            "/usr/local/lib/R",
            "/usr/local/lib64/R",
            "/opt/R/4.5.0/lib/R",
            "/Library/Frameworks/R.framework/Resources",
        ]
        .iter()
        .find(|d| Path::new(d).join("library/base/DESCRIPTION").exists())
        .map(|d| d.to_string())
    })?;
    setenv("R_HOME", &home);
    let script = std::fs::read_to_string(Path::new(&home).join("bin/R")).unwrap_or_default();
    for var in ["R_SHARE_DIR", "R_INCLUDE_DIR", "R_DOC_DIR"] {
        if std::env::var_os(var).is_some() {
            continue;
        }
        let set = script.lines().find_map(|l| l.strip_prefix(var)?.strip_prefix('='));
        let value = match set {
            Some(v) if !v.contains('$') => v.trim().to_string(),
            _ => format!("{}/{}", home, &var[2..var.len() - 4].to_ascii_lowercase()),
        };
        setenv(var, &value);
    }
    Some(home)
}

// ----------------------------------------------------------------------
// Renviron files (R: Renviron.c)

fn is_space(c: char) -> bool {
    c.is_ascii_whitespace()
}

/// `${VAR-default}` / `${VAR:-default}`, recursively.
// R: Renviron.c:subterm
fn subterm(s: &str) -> String {
    if !s.starts_with("${") || !s.ends_with('}') {
        return s.to_string();
    }
    let s = s[2..s.len() - 1].trim_matches(is_space);
    if s.is_empty() {
        return String::new();
    }
    let (name, default, colon) = match s.find('-') {
        Some(p) if p > 1 && s.as_bytes()[p - 1] == b':' => (&s[..p - 1], Some(&s[p + 1..]), true),
        Some(p) => (&s[..p], Some(&s[p + 1..]), false),
        None => (s, None, false),
    };
    match std::env::var(name) {
        Ok(v) if !colon || !v.is_empty() => v,
        _ => default.map(subterm).unwrap_or_default(),
    }
}

/// The `}` matching an opening `{` just before `s`.
// R: Renviron.c:findRbrace
fn find_rbrace(s: &str) -> Option<usize> {
    let (mut nl, mut nr) = (0, 0);
    let mut p = 0;
    let mut pr = 0;
    while nr <= nl {
        let pl = s[p..].find('{').map(|i| i + p);
        pr = s[p..].find('}').map(|i| i + p)?;
        match pl {
            Some(pl) if pl < pr => {
                p = pl + 1;
                nl += 1;
            }
            _ => {
                p = pr + 1;
                nr += 1;
            }
        }
    }
    Some(pr)
}

/// Expand the `${...}` terms of a right-hand side.
// R: Renviron.c:findterm
fn findterm(s: &str) -> String {
    let mut ans = String::new();
    let mut s = s;
    while let Some(p) = s.find('$') {
        if s.as_bytes().get(p + 1) != Some(&b'{') {
            break;
        }
        let Some(q) = find_rbrace(&s[p + 2..]).map(|q| q + p + 2) else { break };
        ans.push_str(&s[..p]);
        ans.push_str(&subterm(&s[p..=q]));
        s = &s[q + 1..];
    }
    ans.push_str(s);
    ans
}

/// Set `name` to `value` with quotes removed and backslashes processed.
// R: Renviron.c:Putenv
fn putenv(name: &str, value: &str) {
    let b: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut quote = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let prev = if i > 0 { Some(b[i - 1]) } else { None };
        match quote {
            None if (c == '"' || c == '\'') && prev != Some('\\') => quote = Some(c),
            Some(q) if c == q && prev != Some('\\') => quote = None,
            None if c == '\\' => {
                if b.get(i + 1) == Some(&'\n') {
                    i += 1;
                } else if b.get(i + 1) == Some(&'\\') {
                    out.push(c);
                }
            }
            Some(q) if c == '\\' && b.get(i + 1) == Some(&q) => {}
            _ => out.push(c),
        }
        i += 1;
    }
    setenv(name, &out);
}

/// Read an `Renviron` file. Returns `None` if it cannot be read, else the
/// warning about invalid lines, if any.
// R: Renviron.c:process_Renviron
pub fn process_renviron(path: &str) -> Option<Option<String>> {
    let text = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&text);
    let mut msg: Option<String> = None;
    for line in text.lines() {
        let s = line.trim_matches(is_space);
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        let Some(eq) = s.find('=') else {
            let m = msg.get_or_insert_with(|| format!("\n   File {} contains invalid line(s)", path));
            m.push_str("\n      ");
            m.push_str(s);
            continue;
        };
        let lhs = s[..eq].trim_matches(is_space);
        let rhs = findterm(s[eq + 1..].trim_matches(is_space));
        if !lhs.is_empty() && !rhs.is_empty() {
            putenv(lhs, &rhs);
        }
    }
    Some(msg.map(|m| m + "\n   They were ignored\n"))
}

fn renviron_startup(path: &str) -> bool {
    match process_renviron(path) {
        None => false,
        Some(warning) => {
            if let Some(w) = warning {
                eprintln!("{}", w);
            }
            true
        }
    }
}

/// The system `Renviron`, read even with `--no-environ`/`--vanilla`.
// R: Renviron.c:process_system_Renviron, called from unix/system.c:Rf_initialize_R
fn process_system_environ(home: &str) {
    if !renviron_startup(&format!("{}/etc/Renviron", home)) {
        eprintln!("cannot find system Renviron");
    }
}

/// The site and user `Renviron` files, skipped by `--no-environ`.
// R: Renviron.c:process_site_Renviron, process_user_Renviron
fn process_environ_files(home: &str) {
    match std::env::var("R_ENVIRON") {
        Ok(p) => {
            if !p.is_empty() {
                renviron_startup(&p);
            }
        }
        Err(_) => {
            renviron_startup(&format!("{}/etc/Renviron.site", home));
        }
    }
    match std::env::var("R_ENVIRON_USER") {
        Ok(p) => {
            if !p.is_empty() {
                renviron_startup(&crate::builtins::io::expand_file_name(&p));
            }
        }
        Err(_) => {
            if !renviron_startup(".Renviron") {
                if let Ok(h) = std::env::var("HOME") {
                    renviron_startup(&format!("{}/.Renviron", h));
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Profiles and .First

/// Evaluate a profile in `env` like R's `R_ReplFile`: print visible
/// values; an error stops the session (as `check_session_exit` does in a
/// non-interactive R).
fn load_profile(interp: &mut Interp, src: &str, name: &str, env: &Env) -> bool {
    let saved = std::mem::replace(&mut interp.source_name, name.to_string());
    let ok = interp.run_source_in(src, env);
    interp.source_name = saved;
    ok
}

fn read_profile(path: &str) -> Option<String> {
    std::fs::read_to_string(crate::builtins::io::expand_file_name(path)).ok()
}

/// Call `name()` in the global environment if `env` binds it to a closure.
fn call_first(interp: &mut Interp, name: &str, env: &Env) -> bool {
    match env.lookup(Sym::new(name)) {
        Some((Value::Closure(_), _)) => {
            let call = Value::call_sym(name, vec![]);
            let global = interp.global.clone();
            interp.eval_toplevel(&call, &global, true).is_some()
        }
        _ => true,
    }
}

/// Everything after loading base. Returns `false` if an error ended the
/// session ("Execution halted").
// R: main.c:setup_Rmainloop
pub fn run(interp: &mut Interp, opts: StartupOptions) -> bool {
    let home = init_r_home();
    if let Some(home) = &home {
        process_system_environ(home);
        if !opts.no_environ {
            process_environ_files(home);
        }
    }
    let base_ns = interp.base_ns.clone();
    let global = interp.global.clone();
    let system: String = PROFILE_SOURCES.iter().map(|(_, s)| *s).collect::<Vec<_>>().join("\n");
    if !load_profile(interp, &system, "Rprofile", &base_ns) {
        return false;
    }
    // R: devices.c:InitGraphics (run before the base package loads)
    // binds .Device and .Devices, the base variables the graphics engine
    // keeps current
    if !base_ns.has_local(Sym::new(".Device")) {
        base_ns.set_local(Sym::new(".Device"), Value::str("null device"));
        base_ns.set_local(Sym::new(".Devices"), Value::pairlist(vec![Arg::pos(Value::str("null device"))]));
    }
    // These are the same bindings, so only lock them once.
    base_ns.lock(true);
    interp.base.lock(false);
    for s in [".Device", ".Devices"] {
        interp.base.set_binding_locked(Sym::new(s), false);
    }
    if !call_first(interp, ".OptRequireMethods", &global) {
        return false;
    }
    if !opts.no_site_file {
        let site = match std::env::var("R_PROFILE") {
            Ok(p) => (!p.is_empty()).then(|| read_profile(&p)).flatten(),
            Err(_) => home.as_ref().and_then(|h| read_profile(&format!("{}/etc/Rprofile.site", h))),
        };
        if let Some(src) = site {
            if !load_profile(interp, &src, "Rprofile.site", &global) {
                return false;
            }
        }
    }
    // The system profile binds .Library.site actively in the global
    // environment to capture the site profile's assignments.
    global.remove(Sym::new(".Library.site"));
    if !opts.no_init_file {
        let user = match std::env::var("R_PROFILE_USER") {
            Ok(p) => (!p.is_empty()).then(|| read_profile(&p)).flatten(),
            Err(_) => read_profile(".Rprofile")
                .or_else(|| std::env::var("HOME").ok().and_then(|h| read_profile(&format!("{}/.Rprofile", h)))),
        };
        if let Some(src) = user {
            if !load_profile(interp, &src, ".Rprofile", &global) {
                return false;
            }
        }
    }
    if !call_first(interp, ".First", &global) {
        return false;
    }
    call_first(interp, ".First.sys", &base_ns)
}
