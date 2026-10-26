//! Builtin functions: the `.Primitive` and `.Internal` layer of GNU R
//! (names.c). Modules are named after the C files they port.

use std::collections::HashMap;
use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub mod apply;
pub mod arith;
pub mod array;
pub mod attrib;
pub mod bind;
pub mod character;
pub mod coerce;
pub mod conditions;
pub mod datetime;
pub mod dotcode;
pub mod envir;
pub mod funs;
pub mod grep;
pub mod identical;
pub mod io;
pub mod lang;
pub mod lapack;
pub mod math;
pub mod misc;
pub mod namespace;
pub mod parsing;
pub mod options;
pub mod paste;
pub mod printint;
pub mod random;
pub mod scan;
pub mod seq;
pub mod serialize;
pub mod sort;
pub mod sprintf;
pub mod subassign;
pub mod subscript;
pub mod subset;
pub mod summary;
pub mod sysutils;
pub mod unique;
pub mod util;

pub type BuiltinFn = fn(&mut Interp, &Value, Vec<Arg>, &Env) -> RResult<Value>;

impl Builtin {
    pub const fn new(name: &'static str, func: BuiltinFn, arity: i16) -> Builtin {
        Builtin { name, kind: BuiltinKind::Builtin, func, internal: false, visibility: Visibility::On, arity }
    }
    pub const fn special(mut self) -> Builtin {
        self.kind = BuiltinKind::Special;
        self
    }
    pub const fn internal(mut self) -> Builtin {
        self.internal = true;
        self
    }
    pub const fn invisible(mut self) -> Builtin {
        self.visibility = Visibility::Off;
        self
    }
    pub const fn custom_vis(mut self) -> Builtin {
        self.visibility = Visibility::Custom;
        self
    }
}

/// A primitive with evaluated arguments.
pub const fn prim(name: &'static str, func: BuiltinFn, arity: i16) -> Builtin {
    Builtin::new(name, func, arity)
}

/// A primitive receiving unevaluated arguments.
pub const fn special(name: &'static str, func: BuiltinFn, arity: i16) -> Builtin {
    Builtin::new(name, func, arity).special()
}

/// An `.Internal` function with evaluated arguments.
pub const fn internal(name: &'static str, func: BuiltinFn, arity: i16) -> Builtin {
    Builtin::new(name, func, arity).internal()
}

thread_local! {
    static INTERNALS: std::cell::RefCell<HashMap<&'static str, &'static Builtin>> = Default::default();
}

pub fn lookup_internal(name: &str) -> Option<&'static Builtin> {
    INTERNALS.with(|m| m.borrow().get(name).copied())
}

/// All registered builtins (for coverage reports).
pub fn all_builtins() -> Vec<&'static Builtin> {
    TABLES.iter().flat_map(|t| t.iter()).collect()
}

thread_local! {
    static PRIMITIVES: HashMap<&'static str, &'static Builtin> = {
        let mut m = HashMap::new();
        for b in TABLES.iter().flat_map(|t| t.iter()).filter(|b| !b.internal) {
            // The first definition wins, as in a linear search.
            m.entry(b.name).or_insert(b);
        }
        m
    };
}

pub fn lookup_primitive(name: &str) -> Option<&'static Builtin> {
    PRIMITIVES.with(|m| m.get(name).copied())
}

static TABLES: &[&[Builtin]] = &[
    arith::DEFS,
    attrib::DEFS,
    bind::DEFS,
    coerce::DEFS,
    lang::DEFS,
    envir::DEFS,
    funs::DEFS,
    misc::DEFS,
    namespace::DEFS,
    parsing::DEFS,
    serialize::DEFS,
    conditions::DEFS,
    identical::DEFS,
    random::DEFS,
    subset::DEFS,
    subassign::DEFS,
    subscript::DEFS,
    character::DEFS,
    paste::DEFS,
    sprintf::DEFS,
    grep::DEFS,
    seq::DEFS,
    summary::DEFS,
    sort::DEFS,
    unique::DEFS,
    math::DEFS,
    io::DEFS,
    printint::DEFS,
    apply::DEFS,
    array::DEFS,
    datetime::DEFS,
    options::DEFS,
    sysutils::DEFS,
    lapack::DEFS,
    util::DEFS,
    dotcode::DEFS,
    scan::DEFS,
];

pub fn install(interp: &mut Interp) {
    INTERNALS.with(|m| {
        let mut m = m.borrow_mut();
        for table in TABLES {
            for b in table.iter() {
                if b.internal {
                    m.insert(b.name, b);
                } else {
                    interp.base.set_local(Sym::new(b.name), Value::Builtin(b));
                }
            }
        }
    });
    interp.base.set_local(Sym::new(".BaseNamespaceEnv"), Value::Env(interp.base_ns.clone()));
    interp.base.set_local(Sym::new(".GlobalEnv"), Value::Env(interp.global.clone()));
    interp.base.set_local(Sym::new("T"), Value::lgl(true));
    interp.base.set_local(Sym::new("F"), Value::lgl(false));
    interp.base.set_local(Sym::new("pi"), Value::real(std::f64::consts::PI));
    interp.set_option("digits", Value::int(7));
    interp.set_option("width", Value::int(80));
    interp.set_option("scipen", Value::real(0.0));
    interp.set_option("warn", Value::real(0.0));
    interp.set_option("OutDec", Value::str("."));
    interp.set_option("keep.source", Value::lgl(false));
    interp.set_option("expressions", Value::int(5000));
    interp.set_option("useFancyQuotes", Value::lgl(false));
    options::init_options(interp);
    sysutils::install_vars(interp);
}

/// Settings applied after the base library is loaded (the parts of the
/// system Rprofile that robigo supports so far).
pub fn post_base_init(interp: &mut Interp) {
    interp.set_option("showErrorCalls", Value::lgl(true));
    interp.set_option("max.print", Value::int(99999));
    interp.set_option("deparse.max.lines", Value::Null);
}

// ----------------------------------------------------------------------
// Argument helpers

/// Match evaluated builtin arguments to formal names with R's rules
/// (exact, partial before `...`, positional). Unmatched arguments go to
/// `...` if present in `formals`, otherwise it is an error.
pub fn match_args(args: Vec<Arg>, formals: &[&str], call: &Value) -> RResult<Matched> {
    let nf = formals.len();
    let mut vals: Vec<Option<Value>> = vec![None; nf];
    let mut fused = vec![false; nf];
    let mut used = vec![false; args.len()];
    let dots = formals.iter().position(|f| *f == "...");
    // exact
    for (fi, f) in formals.iter().enumerate() {
        if *f == "..." {
            continue;
        }
        for (ai, a) in args.iter().enumerate() {
            if !used[ai] && a.tag.is_some_and(|t| t.with_name(|n| n == *f)) {
                if fused[fi] {
                    return Err(Flow::error_call(
                        call.clone(),
                        format!("formal argument \"{}\" matched by multiple actual arguments", f),
                    ));
                }
                vals[fi] = Some(a.value.clone());
                fused[fi] = true;
                used[ai] = true;
            }
        }
    }
    // partial (only formals before ...)
    for (fi, f) in formals.iter().enumerate() {
        if Some(fi) >= dots && dots.is_some() {
            break;
        }
        if fused[fi] || *f == "..." {
            continue;
        }
        for (ai, a) in args.iter().enumerate() {
            if used[ai] {
                continue;
            }
            if let Some(t) = a.tag {
                let tn = t.name();
                if !tn.is_empty() && f.starts_with(&*tn) {
                    if fused[fi] {
                        return Err(Flow::error_call(
                            call.clone(),
                            format!("formal argument \"{}\" matched by multiple actual arguments", f),
                        ));
                    }
                    vals[fi] = Some(a.value.clone());
                    fused[fi] = true;
                    used[ai] = true;
                }
            }
        }
    }
    // positional
    let mut ai = 0;
    for (fi, f) in formals.iter().enumerate() {
        if *f == "..." {
            break;
        }
        if fused[fi] {
            continue;
        }
        while ai < args.len() && (used[ai] || args[ai].tag.is_some()) {
            ai += 1;
        }
        if ai < args.len() {
            vals[fi] = Some(args[ai].value.clone());
            fused[fi] = true;
            used[ai] = true;
            ai += 1;
        }
    }
    let rest: Vec<Arg> = args.iter().zip(used.iter()).filter(|(_, u)| !**u).map(|(a, _)| a.clone()).collect();
    if dots.is_none() && !rest.is_empty() {
        let text: Vec<String> = rest
            .iter()
            .map(|a| match a.tag {
                Some(t) => format!("{} = {}", t, crate::deparse::deparse_one_line(&a.value)),
                None => crate::deparse::deparse_one_line(&a.value),
            })
            .collect();
        return Err(Flow::error_call(
            call.clone(),
            format!("unused argument{} ({})", if rest.len() == 1 { "" } else { "s" }, text.join(", ")),
        ));
    }
    // Treat explicitly empty arguments as missing.
    for v in vals.iter_mut() {
        if v.as_ref().is_some_and(|x| x.is_missing_arg()) {
            *v = None;
        }
    }
    Ok(Matched { vals, dots: rest })
}

pub struct Matched {
    pub vals: Vec<Option<Value>>,
    pub dots: Vec<Arg>,
}

impl Matched {
    pub fn get(&self, i: usize) -> Option<&Value> {
        self.vals[i].as_ref()
    }
    pub fn take(&mut self, i: usize) -> Option<Value> {
        self.vals[i].take()
    }
    pub fn or_null(&mut self, i: usize) -> Value {
        self.vals[i].take().unwrap_or(Value::Null)
    }
}

/// Check the number of arguments for fixed-arity builtins.
pub fn check_arity(args: &[Arg], n: usize, name: &str, call: &Value) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires {}",
                args.len(),
                if args.len() == 1 { "" } else { "s" },
                name,
                n
            ),
        ));
    }
    Ok(())
}

pub fn arg(args: &[Arg], i: usize) -> Value {
    args.get(i).map(|a| a.value.clone()).unwrap_or(Value::Null)
}

pub fn str_arg(v: &Value, what: &str) -> RResult<Rc<str>> {
    match v {
        Value::Str(s) if !s.data.is_empty() => match &s.data[0] {
            Some(x) => Ok(x.clone()),
            None => Err(Flow::error(format!("invalid '{}' argument", what))),
        },
        Value::Sym(s) => Ok(s.name()),
        _ => Err(Flow::error(format!("invalid '{}' argument", what))),
    }
}

pub fn lgl_arg(v: &Value, what: &str) -> RResult<bool> {
    let l = crate::coerce::as_logical(v);
    if l == NA_LOGICAL {
        return Err(Flow::error(format!("invalid '{}' argument", what)));
    }
    Ok(l != 0)
}

pub fn int_arg(v: &Value, what: &str) -> RResult<i32> {
    let i = crate::coerce::as_integer(v);
    if i == NA_INTEGER {
        return Err(Flow::error(format!("invalid '{}' argument", what)));
    }
    Ok(i)
}

#[cfg(test)]
mod tests {
    /// Each primitive and each .Internal is defined by exactly one module.
    #[test]
    fn no_duplicate_registrations() {
        let mut seen = std::collections::HashSet::new();
        let dups: Vec<_> = super::all_builtins()
            .into_iter()
            .filter(|b| !seen.insert((b.internal, b.name)))
            .map(|b| b.name)
            .collect();
        assert!(dups.is_empty(), "builtins registered twice: {:?}", dups);
    }
}
