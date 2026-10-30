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
mod hashtab;
pub mod linpack;
pub mod methods;
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
    /// The native code's address (a loaded shared object, called through
    /// the [`NativeBridge`]); 0 for routines served by Rust ports.
    pub addr: usize,
}

/// Loads and calls compiled code (`libR.so`'s side, robigo-capi), so
/// that the runtime does not depend on the C API crate. Registered once
/// at startup by the executable; without it only the Rust ports exist.
pub trait NativeBridge: Sync {
    /// Open a shared object and run its `R_init_<name>`; the handle
    /// identifies it to the other calls.
    fn load(&self, interp: &mut Interp, path: &str, name: &str, local: bool, now: bool) -> RResult<usize>;
    fn unload(&self, interp: &mut Interp, handle: usize);
    /// (useDynamicLookup, forceSymbols) after initialization.
    fn flags(&self, handle: usize) -> (bool, bool);
    /// The registered routines: (interface, name, address, numArgs).
    fn routines(&self, handle: usize) -> Vec<(Interface, String, usize, i32)>;
    /// Look a symbol up (registered first, then dynamically).
    fn lookup(&self, handle: usize, name: &str, interface: Option<Interface>) -> Option<(usize, i32, Interface)>;
    /// Call a routine with evaluated arguments (`.Call`, `.External`,
    /// `.C`, `.Fortran`).
    fn invoke(
        &self,
        interp: &mut Interp,
        call: &Value,
        r: &Routine,
        args: Vec<Arg>,
        env: &crate::env::Env,
    ) -> RResult<Value>;
    /// Whether runtime weak keys require clearing implementation caches.
    fn finalizers_active(&self) -> bool {
        false
    }
    /// Whether a runtime wrapper contains an environment or external pointer.
    fn is_finalizer_key(&self, key: &Value) -> bool {
        matches!(key, Value::Env(_) | Value::ExtPtr(_))
    }
    /// Register a runtime reference object in the native weakref queue.
    fn register_finalizer(&self, _interp: &mut Interp, _key: &Value, _fun: &Value, _onexit: bool) -> RResult<()> {
        Err(Flow::error("robigo: environment finalizers require the native bridge"))
    }
    /// Run the finalizers registered with `onexit = TRUE` at exit
    /// (memory.c:R_RunExitFinalizers).
    fn run_exit_finalizers(&self, _interp: &mut Interp) {}
    /// `gc()`: collect the C heap and run pending finalizers.
    fn gc(&self, interp: &mut Interp);
    /// A `.Internal` GNU R implements in its graphics engine (C code in
    /// libR: `recordGraphics`), called with the evaluated arguments.
    fn c_internal(
        &self,
        _interp: &mut Interp,
        name: &str,
        _call: &Value,
        _args: &[Arg],
        _env: &crate::env::Env,
    ) -> RResult<Value> {
        Err(crate::error::Flow::error(format!("robigo: '{name}' needs the graphics engine (libR)")))
    }
    /// The value of a C-owned variable (see [`c_owned_binding`]).
    fn owned_get(&self, _interp: &mut Interp, _holder: &Value) -> RResult<Value> {
        Ok(Value::Null)
    }
    /// A holder of C's object for a value assigned to a C-owned variable.
    fn owned_holder(&self, _interp: &mut Interp, value: Value) -> RResult<Value> {
        Ok(value)
    }
    /// options() set an option the C code reads from a global
    /// (`OutDec`, `max.contour.segments`).
    fn option_set(&self, _name: &str, _value: &Value) {}
    /// `.External.graphics`/`.Call.graphics` before the call: the current
    /// device's display-list recording, which is turned off.
    fn graphics_begin(&self, _interp: &mut Interp) -> RResult<bool> {
        Ok(false)
    }
    /// ... and after it: recording restored, the operation recorded.
    fn graphics_end(
        &self,
        _interp: &mut Interp,
        _call: &Value,
        _op: &Value,
        _args: &[Arg],
        _record: bool,
    ) -> RResult<()> {
        Ok(())
    }
}

static BRIDGE: std::sync::OnceLock<&'static dyn NativeBridge> = std::sync::OnceLock::new();

/// Install the bridge (once, before any `dyn.load`).
pub fn set_bridge(b: &'static dyn NativeBridge) {
    let _ = BRIDGE.set(b);
}

pub fn bridge() -> Option<&'static dyn NativeBridge> {
    BRIDGE.get().copied()
}

/// Whether GNU R's build registers routines for this DLL name (its
/// native code is served by Rust ports until it is built against robigo).
pub fn has_table(name: &str) -> bool {
    TABLE.lines().any(|l| l.split('\t').next() == Some(name))
}

/// Whether a shared object is one of GNU R's base packages built from
/// source against robigo (`scripts/build-natives.sh` links in the
/// symbol `robigo_base_native`). The marker is a dynamic symbol name,
/// so only the dynamic string table is searched: these files are
/// megabytes and every base-package dyn.load checks one.
pub fn is_robigo_build(path: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let marker = b"robigo_base_native";
    let Ok(f) = std::fs::File::open(path) else { return false };
    let mut f = std::io::BufReader::with_capacity(1 << 16, f);
    let u64_at = |buf: &[u8], off: usize| u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
    // ELF64 header: program headers at e_phoff (u64 at 0x20), count
    // e_phnum (u16 at 0x38), entry size e_phentsize (u16 at 0x36).
    let mut eh = [0u8; 64];
    if f.read_exact(&mut eh).is_err() || &eh[..4] != b"\x7fELF" || eh[4] != 2 {
        return false;
    }
    let phoff = u64_at(&eh, 0x20) as usize;
    let phentsize = u16::from_le_bytes([eh[0x36], eh[0x37]]) as usize;
    let phnum = u16::from_le_bytes([eh[0x38], eh[0x39]]) as usize;
    if phoff == 0 || phentsize < 56 || phnum > 64 {
        return false;
    }
    if f.seek(SeekFrom::Start(phoff as u64)).is_err() {
        return false;
    }
    let mut ph = vec![0u8; phentsize * phnum];
    if f.read_exact(&mut ph).is_err() {
        return false;
    }
    let entry = |i: usize| &ph[i * phentsize..(i + 1) * phentsize];
    // PT_DYNAMIC (type 2): the vaddr -> file offset map from the PT_LOAD
    // segments (type 1), then DT_STRTAB (5) / DT_STRSZ (10).
    let loads: Vec<(u64, u64, u64)> = (0..phnum)
        .filter(|&i| u32::from_le_bytes(entry(i)[0..4].try_into().unwrap()) == 1)
        .map(|i| {
            let e = entry(i);
            (u64_at(e, 0x10), u64_at(e, 0x08), u64_at(e, 0x20)) // vaddr, off, filesz
        })
        .collect();
    let to_off = |vaddr: u64| -> Option<u64> {
        loads.iter().find(|(va, _, sz)| vaddr >= *va && vaddr < va + sz).map(|(va, off, _)| off + (vaddr - va))
    };
    let dynseg = (0..phnum).find(|&i| u32::from_le_bytes(entry(i)[0..4].try_into().unwrap()) == 2).and_then(|i| {
        let e = entry(i);
        let off = to_off(u64_at(e, 0x10))?;
        let sz = u64_at(e, 0x20) as usize;
        Some((off, sz))
    });
    let Some((dyn_off, dyn_sz)) = dynseg else { return false };
    if f.seek(SeekFrom::Start(dyn_off)).is_err() {
        return false;
    }
    let mut dynbuf = vec![0u8; dyn_sz];
    if f.read_exact(&mut dynbuf).is_err() {
        return false;
    }
    let (mut strtab, mut strsz) = (None, 0usize);
    for ent in dynbuf.chunks_exact(16) {
        let tag = u64::from_le_bytes(ent[0..8].try_into().unwrap());
        let val = u64::from_le_bytes(ent[8..16].try_into().unwrap());
        match tag {
            5 => strtab = Some(val),
            10 => strsz = val as usize,
            0 => break,
            _ => {}
        }
    }
    let (Some(strtab), strsz) = (strtab, strsz) else { return false };
    let strsz = strsz.min(1 << 20);
    let Some(off) = to_off(strtab) else { return false };
    if f.seek(SeekFrom::Start(off)).is_err() {
        return false;
    }
    let mut strs = vec![0u8; strsz];
    match f.read_exact(&mut strs) {
        // The dynamic string table is NUL-separated names; comparing
        // whole names (a vectorized NUL scan plus one memcmp per name)
        // beats a sliding-window compare over every byte position.
        Ok(()) => strs.split(|&b| b == 0).any(|name| name == marker),
        Err(_) => false,
    }
}

/// robigo's build of a base package's native code from GNU R's sources
/// (scripts/build-natives.sh): `$ROBIGO_NATIVES/<pkg>/<pkg>.so`, else
/// `natives/<pkg>/<pkg>.so` beside the executable's directory
/// (`.robigo/bin/robigo`, `.robigo/natives`).
pub fn robigo_native(pkg: &str) -> Option<String> {
    let dir = match std::env::var("ROBIGO_NATIVES") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => std::env::current_exe().ok()?.parent()?.parent()?.join("natives"),
    };
    let so = dir.join(pkg).join(format!("{pkg}.so"));
    let so = so.to_str()?.to_string();
    is_robigo_build(&so).then_some(so)
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
    /// The [`NativeBridge`] handle of a loaded shared object; 0 for DLLs
    /// served by Rust ports.
    pub native: usize,
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
                    addr: 0,
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
            native: 0,
            handle: ext_ptr("DLLHandle", Some("DLLHandle"), None),
            info,
        }
    }

    /// A shared object loaded through the bridge, with the routines its
    /// `R_init_<name>` registered.
    pub fn loaded(name: &str, path: &str, native: usize, b: &dyn NativeBridge) -> Dll {
        let mut d = Dll::new(name, path);
        let (dynamic, force) = b.flags(native);
        d.native = native;
        d.dynamic_lookup = dynamic;
        d.force_symbols = force;
        d.routines = b
            .routines(native)
            .into_iter()
            .map(|(interface, n, addr, nargs)| {
                Rc::new(Routine { package: d.name.clone(), name: Rc::from(n.as_str()), interface, nargs, addr })
            })
            .collect();
        d
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
        if let Some(r) =
            self.routines.iter().find(|r| &*r.name == name && interface.is_none_or(|i| i == r.interface)).cloned()
        {
            return Some(r);
        }
        // R: Rdynload.c:R_dlsym — then dlsym, if the DLL allows it
        if self.native != 0 && self.dynamic_lookup {
            let (addr, nargs, interface) = bridge()?.lookup(self.native, name, interface)?;
            return Some(Rc::new(Routine { package: self.name.clone(), name: Rc::from(name), interface, nargs, addr }));
        }
        None
    }
}

/// A port: (interpreter, the `.Call`/`.External` call, arguments without
/// `.NAME`, the calling environment as `.External2` passes it).
pub type NativeFn = fn(&mut Interp, &Value, Vec<Arg>, &crate::env::Env) -> RResult<Value>;

/// Rust ports of registered routines, per package: (routine, function).
static PORTS: &[(&str, &[(&str, NativeFn)])] = &[
    ("base", base::PORTS),
    ("grDevices", grdevices::PORTS),
    ("methods", methods::PORTS),
    ("stats", stats::PORTS),
    ("utils", utils::PORTS),
];

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
/// Packages whose ported routines keep their Rust port while the
/// package's own native code (built from GNU R's sources) is loaded: the
/// bridge copies every argument and result between the runtime and C (a
/// model frame's 20,000 row names into CHARSXPs and back made `lm` twice
/// as slow), so the ports stay until arguments are shared (G3). grDevices,
/// graphics and grid are not listed: their state (devices, display lists)
/// must live in one implementation.
const PORTS_FIRST: &[&str] = &["stats"];

pub fn invoke(interp: &mut Interp, call: &Value, r: &Routine, args: Vec<Arg>, env: &crate::env::Env) -> RResult<Value> {
    let ported = PORTS_FIRST.contains(&&*r.package) && (is_internal_alias(r) || port_of(r).is_some());
    if r.addr != 0 && !ported {
        if let Some(b) = bridge() {
            return b.invoke(interp, call, r, args, env);
        }
    }
    if is_internal_alias(r) {
        if r.name.starts_with('r') {
            return crate::builtins::random::stats_random(interp, call, args, &r.name);
        }
        if let Some(b) = crate::builtins::lookup_internal(&r.name) {
            let env = interp.global.clone();
            return interp.invoke_builtin(b, call, args, &env);
        }
    }
    match port_of(r) {
        Some(f) => f(interp, call, args, env),
        None => Err(Flow::error_call(
            call.clone(),
            format!("robigo: native routine '{}' of package '{}' is not implemented yet", r.name, r.package),
        )),
    }
}

/// The Rust port of a routine.
fn port_of(r: &Routine) -> Option<NativeFn> {
    PORTS
        .iter()
        .filter(|(p, _)| *p == &*r.package)
        .flat_map(|(_, t)| t.iter())
        .find(|(n, _)| *n == &*r.name)
        .map(|(_, f)| *f)
}

/// The base DLL, registered at startup as in GNU R's `InitDynload`.
pub fn base_dll() -> Dll {
    Dll::new("base", "base")
}

/// A variable whose value C code keeps and mutates in place (GNU R code
/// such as grid's `SET_VECTOR_ELT(findVar(".GRID.STATE", ...), i, state)`
/// shares the object between C and R; the bridge hands C its own copy).
/// The binding becomes an active binding over C's object, which `holder`
/// (from the bridge) keeps: reading the variable converts C's current
/// object, assigning replaces it.
pub fn c_owned_binding(env: &crate::env::Env, sym: crate::sym::Sym, holder: Value) {
    let v = crate::sym::Sym::new("v");
    let cell = crate::env::Env::new(Some(env.clone()));
    cell.set_local(crate::sym::Sym::new(".x"), holder);
    let f = Value::Closure(std::rc::Rc::new(crate::value::Closure {
        formals: std::rc::Rc::from(vec![Arg { tag: Some(v), value: Value::missing_arg() }]),
        body: Value::lang(Value::Builtin(&C_OWNED), vec![Arg::pos(Value::Sym(v))]),
        env: cell,
        attrs: None,
        code: Default::default(),
        inline: Default::default(),
        defaults: Default::default(),
        jit: Default::default(),
    }));
    env.set_local_flags(sym, f, crate::env::BIND_ACTIVE);
}

/// The holder of a C-owned variable's object, given its binding function.
pub fn c_owned_holder(f: &Value) -> Option<Value> {
    match f {
        Value::Closure(c) => match &c.body {
            Value::Lang(l) if matches!(&l.fun, Value::Builtin(b) if std::ptr::eq(*b, &C_OWNED)) => {
                c.env.get_local(crate::sym::Sym::new(".x"))
            }
            _ => None,
        },
        _ => None,
    }
}

static C_OWNED: Builtin = crate::builtins::special(".robigo.c_owned", do_c_owned, 1);

fn do_c_owned(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &crate::env::Env) -> RResult<Value> {
    let Some(Value::Sym(v)) = args.first().map(|a| &a.value) else {
        return Err(crate::error::Flow::error("robigo: invalid C-owned binding"));
    };
    let x = crate::sym::Sym::new(".x");
    let Some(cell) = env.parent() else { return Ok(Value::Null) };
    let holder = cell.get_local(x).unwrap_or(Value::Null);
    let Some(b) = bridge() else { return Ok(Value::Null) };
    if crate::builtins::lang::missing_sym(i, call, *v, env)? {
        b.owned_get(i, &holder)
    } else {
        let value = i.eval(&Value::Sym(*v), env)?;
        let holder = b.owned_holder(i, value)?;
        cell.set_local(x, holder);
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_marker_with_distinct_file_and_virtual_addresses() {
        // ELF64: one load segment with a nonzero address bias, containing
        // PT_DYNAMIC and its string table. Debian shared libraries can
        // have this layout in their writable segment.
        let mut elf = vec![0u8; 0x200];
        elf[..5].copy_from_slice(b"\x7fELF\x02");
        let put = |buf: &mut [u8], at: usize, n: u64| buf[at..at + 8].copy_from_slice(&n.to_le_bytes());
        put(&mut elf, 0x20, 64);
        elf[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        elf[0x38..0x3a].copy_from_slice(&2u16.to_le_bytes());
        for (header, kind, offset, address, size) in [(64, 1u32, 0x100, 0x1100, 0x100), (120, 2, 0x100, 0x1100, 48)] {
            elf[header..header + 4].copy_from_slice(&kind.to_le_bytes());
            put(&mut elf, header + 8, offset);
            put(&mut elf, header + 16, address);
            put(&mut elf, header + 32, size);
            put(&mut elf, header + 40, size + 0x100);
        }
        put(&mut elf, 0x100, 5); // DT_STRTAB
        put(&mut elf, 0x108, 0x1180);
        put(&mut elf, 0x110, 10); // DT_STRSZ
        put(&mut elf, 0x118, 20);
        elf[0x180..0x180 + 19].copy_from_slice(b"robigo_base_native\0");
        let path = std::env::temp_dir().join(format!("robigo-native-marker-{}.so", std::process::id()));
        std::fs::write(&path, &elf).unwrap();
        assert!(super::is_robigo_build(path.to_str().unwrap()));
        elf[0x180] = b'x';
        std::fs::write(&path, &elf).unwrap();
        assert!(!super::is_robigo_build(path.to_str().unwrap()));
        std::fs::remove_file(path).unwrap();
    }
}
