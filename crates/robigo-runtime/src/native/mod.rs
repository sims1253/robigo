//! Packages' native code (GNU R's Rdynload.c).
//!
//! robigo does not load packages' shared libraries: their code is written
//! against GNU R's internals. Instead, `dyn.load()` registers a DLL with
//! the routines GNU R's own build registers for it (vendored from GNU R as
//! `rlib/native-routines.tsv` by `scripts/vendor-native.R`), so that
//! `loadNamespace()` creates the same `C_xxx` symbol objects, and the
//! `.Call`/`.External`/`.C`/`.Fortran` interfaces dispatch to Rust ports
//! of the routines ([`PORTS`]). A routine without a port is an error that
//! says so.

use std::rc::Rc;

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

static TABLE: &str = include_str!("../../../../rlib/native-routines.tsv");

mod base;
mod grdevices;
pub mod linpack;
mod stats;
mod utils;

/// The interface a routine is registered for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interface {
    C,
    Call,
    Fortran,
    External,
}

impl Interface {
    fn parse(s: &str) -> Option<Interface> {
        Some(match s {
            ".C" => Interface::C,
            ".Call" => Interface::Call,
            ".Fortran" => Interface::Fortran,
            ".External" => Interface::External,
            _ => return None,
        })
    }

    fn class(self) -> &'static str {
        match self {
            Interface::C => "CRoutine",
            Interface::Call => "CallRoutine",
            Interface::Fortran => "FortranRoutine",
            Interface::External => "ExternalRoutine",
        }
    }
}

/// A registered routine; the payload of its `address` external pointer.
pub struct Routine {
    pub package: Rc<str>,
    pub name: Rc<str>,
    pub interface: Interface,
    pub nargs: i32,
}

/// The payload of a DLL's `info` external pointer.
pub struct DllRef {
    pub name: Rc<str>,
}

pub struct Dll {
    pub name: Rc<str>,
    pub path: Rc<str>,
    pub dynamic_lookup: bool,
    pub force_symbols: bool,
    pub routines: Vec<Rc<Routine>>,
    /// The `handle` and `info` external pointers, kept so that every
    /// `DLLInfo` object for this DLL shares them.
    pub handle: Value,
    pub info: Value,
}

fn ext_ptr(tag: &str, class: Option<&str>, payload: Option<Rc<dyn std::any::Any>>) -> Value {
    let mut v = Value::ExtPtr(Rc::new(ExtPtr { tag: Value::sym(tag), attrs: Default::default(), payload }));
    if let Some(c) = class {
        v.set_attr_raw(s::CLASS, Value::str(c));
    }
    v
}

impl Dll {
    /// A DLL as GNU R's build registers it (unknown libraries register
    /// nothing and use dynamic lookup).
    pub fn new(name: &str, path: &str) -> Dll {
        let mut dynamic_lookup = true;
        let mut force_symbols = false;
        let mut routines = Vec::new();
        let name: Rc<str> = Rc::from(name);
        for line in TABLE.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 4 || f[0] != &*name {
                continue;
            }
            if f[1] == "DLL" {
                dynamic_lookup = f[2] == "TRUE";
                force_symbols = f[3] == "TRUE";
            } else if let Some(interface) = Interface::parse(f[1]) {
                routines.push(Rc::new(Routine {
                    package: name.clone(),
                    name: Rc::from(f[2]),
                    interface,
                    nargs: f[3].parse().unwrap_or(-1),
                }));
            }
        }
        let info = ext_ptr("DLLInfo", Some("DLLInfoReference"), Some(Rc::new(DllRef { name: name.clone() })));
        Dll {
            name,
            path: Rc::from(path),
            dynamic_lookup,
            force_symbols,
            routines,
            handle: ext_ptr("DLLHandle", Some("DLLHandle"), None),
            info,
        }
    }

    /// The `DLLInfo` object.
    // R: Rdynload.c:Rf_MakeDLLInfo
    pub fn to_value(&self) -> Value {
        let mut v = Value::named_list(vec![
            ("name", Value::str(&self.name)),
            ("path", Value::str(&self.path)),
            ("dynamicLookup", Value::lgl(self.dynamic_lookup)),
            ("handle", self.handle.clone()),
            ("info", self.info.clone()),
            ("forceSymbols", Value::lgl(self.force_symbols)),
        ]);
        v.set_attr_raw(s::CLASS, Value::str("DLLInfo"));
        v
    }
}

/// The `NativeSymbolInfo` object of a registered routine.
// R: Rdynload.c:createRSymbolObject
pub fn symbol_object(dll: &Dll, r: &Rc<Routine>) -> Value {
    let address = ext_ptr("registered native symbol", Some("RegisteredNativeSymbol"), Some(r.clone()));
    let mut v = Value::named_list(vec![
        ("name", Value::str(&r.name)),
        ("address", address),
        ("dll", dll.to_value()),
        ("numParameters", Value::int(r.nargs)),
    ]);
    v.set_attr_raw(s::CLASS, Value::strs_from([r.interface.class(), "NativeSymbolInfo"]));
    v
}

/// The DLL behind a `DLLInfo` object or its `info` reference.
pub fn dll_of<'a>(interp: &'a Interp, v: &Value) -> Option<&'a Dll> {
    let info = match v {
        Value::List(_) => v.list_get("info")?,
        v => v.clone(),
    };
    let Value::ExtPtr(p) = info else { return None };
    let r = p.payload.as_ref()?.clone().downcast::<DllRef>().ok()?;
    interp.dlls.iter().find(|d| d.name == r.name)
}

/// The routine an external pointer refers to.
pub fn routine_of(v: &Value) -> Option<Rc<Routine>> {
    let Value::ExtPtr(p) = v else { return None };
    p.payload.as_ref()?.clone().downcast::<Routine>().ok()
}

/// Find `name` among the routines of `package` ("" for any DLL),
/// restricted to an interface if given. DLLs that force the use of symbol
/// objects (all of GNU R's base packages) are not searched by name.
// R: Rdynload.c:R_FindSymbol
pub fn find_routine(interp: &Interp, name: &str, package: &str, interface: Option<Interface>) -> Option<Rc<Routine>> {
    // Searched most recently loaded first, as R does.
    interp
        .dlls
        .iter()
        .rev()
        .filter(|d| (package.is_empty() || &*d.name == package) && !d.force_symbols)
        .find_map(|d| d.find(name, interface))
}

impl Dll {
    /// A routine of this DLL by name.
    // R: Rdynload.c:R_dlsym (registered symbols)
    pub fn find(&self, name: &str, interface: Option<Interface>) -> Option<Rc<Routine>> {
        self.routines.iter().find(|r| &*r.name == name && interface.is_none_or(|i| i == r.interface)).cloned()
    }
}

/// A port: (interpreter, the `.Call`/`.External` call, arguments without
/// `.NAME`, the calling environment as `.External2` passes it).
pub type NativeFn = fn(&mut Interp, &Value, Vec<Arg>, &crate::env::Env) -> RResult<Value>;

/// Rust ports of registered routines, per package: (routine, function).
static PORTS: &[(&str, &[(&str, NativeFn)])] =
    &[("base", base::PORTS), ("grDevices", grdevices::PORTS), ("stats", stats::PORTS), ("utils", utils::PORTS)];

/// Argument `k` of a `.C`/`.Fortran` call as doubles.
pub fn real_arg(args: &[Arg], k: usize) -> Vec<f64> {
    match args.get(k).map(|a| &a.value) {
        Some(Value::Real(v)) => v.data.clone(),
        Some(v @ (Value::Int(_) | Value::Lgl(_))) => crate::coerce::to_real(v, &mut Default::default()),
        _ => vec![],
    }
}

/// Argument `k` of a `.C`/`.Fortran` call as integers.
pub fn int_arg(args: &[Arg], k: usize) -> Vec<i32> {
    match args.get(k).map(|a| &a.value) {
        Some(Value::Int(v) | Value::Lgl(v)) => v.data.clone(),
        Some(v @ Value::Real(_)) => crate::coerce::to_int(v, &mut Default::default()),
        _ => vec![],
    }
}

/// The value of `.C`/`.Fortran`: the list of all arguments, those the
/// routine wrote replaced by their new contents with the attributes of
/// the originals, named by the argument tags if any.
// R: dotcode.c:do_dotCode (result construction)
pub fn dotc_result(args: Vec<Arg>, replaced: Vec<(usize, Value)>) -> Value {
    let havenames = args.iter().any(|a| a.tag.is_some());
    let names: Vec<String> = args.iter().map(|a| a.tag.map(|t| t.name().to_string()).unwrap_or_default()).collect();
    let mut items: Vec<Value> = args.into_iter().map(|a| a.value).collect();
    for (k, mut v) in replaced {
        if let Some(orig) = items.get(k) {
            v.set_attrs(orig.attrs().cloned().map(Box::new));
            items[k] = v;
        }
    }
    let mut ans = Value::list(items);
    if havenames {
        ans.set_attr_raw(s::NAMES, Value::strs_from(names));
    }
    ans
}

/// Routines whose code is the same as an robigo `.Internal` of the same name
/// and arguments: stats' distribution functions (distn.c and random.c are
/// GNU R's `math2`/`math3`/`random2` dispatchers, which robigo's math and
/// random modules port).
fn is_internal_alias(r: &Routine) -> bool {
    if &*r.package != "stats" || r.interface != Interface::Call {
        return false;
    }
    let n = &*r.name;
    let dist = n.strip_prefix(['d', 'p', 'q', 'r']).unwrap_or("");
    [
        "beta",
        "binom",
        "cauchy",
        "chisq",
        "exp",
        "f",
        "gamma",
        "geom",
        "hyper",
        "lnorm",
        "logis",
        "nbinom",
        "nbinom_mu",
        "norm",
        "pois",
        "t",
        "unif",
        "weibull",
        "nchisq",
        "signrank",
        "wilcox",
    ]
    .contains(&dist)
}

/// Call a routine with already evaluated arguments.
pub fn invoke(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &crate::env::Env) -> RResult<Value> {
    if is_internal_alias(r) {
        if let Some(b) = crate::builtins::lookup_internal(&r.name) {
            let env = interp.global.clone();
            return interp.invoke_builtin(b, call, args, &env);
        }
    }
    let port =
        PORTS.iter().filter(|(p, _)| *p == &*r.package).flat_map(|(_, t)| t.iter()).find(|(n, _)| *n == &*r.name);
    match port {
        Some((_, f)) => f(interp, call, args, env),
        None => Err(Flow::error_call(
            call.clone(),
            format!("robigo: native routine '{}' of package '{}' is not implemented yet", r.name, r.package),
        )),
    }
}

/// The base DLL, registered at startup as in GNU R's `InitDynload`.
pub fn base_dll() -> Dll {
    Dll::new("base", "base")
}
