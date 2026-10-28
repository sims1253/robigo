//! The foreign-function interfaces and DLL management: `dyn.load`,
//! `getLoadedDLLs`, `getRegisteredRoutines`, `getSymbolInfo`, `is.loaded`,
//! `.Call`, `.External`, `.C`, `.Fortran` (GNU R's Rdynload.c and
//! dotcode.c). See [`crate::native`] for how robigo handles native code.

use super::{arg, internal, prim};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::native::{self, Dll, Interface};
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("dyn.load", do_dynload, 4).invisible(),
    internal("dyn.unload", do_dynunload, 1).invisible(),
    internal("getLoadedDLLs", do_get_dll_table, 0),
    internal("getRegisteredRoutines", do_get_registered_routines, 1),
    internal("getSymbolInfo", do_get_symbol_info, 3),
    internal("is.loaded", do_isloaded, -1),
    prim(".C", do_dot_c, -1),
    prim(".Fortran", do_dot_fortran, -1),
    prim(".External", do_external, -1),
    prim(".External2", do_external, -1),
    prim(".Call", do_dotcall, -1),
    prim(".External.graphics", do_external_gr, -1),
    prim(".Call.graphics", do_dotcall_gr, -1),
];

fn path_arg(v: &Value) -> RResult<String> {
    match v {
        Value::Str(s) if s.data.len() == 1 => Ok(s.data[0].as_deref().unwrap_or("NA").to_string()),
        _ => Err(Flow::error("character argument expected")),
    }
}

/// The absolute, tilde-expanded path of a shared object.
// R: unix/dynload.c:getFullDLLPath
fn full_dll_path(path: &str) -> String {
    let p = super::io::expand_file_name(path);
    if p.starts_with('/') {
        return p;
    }
    let cwd = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
    format!("{}/{}", cwd, p)
}

// R: Rdynload.c:do_dynload, AddDLL, R_RegisterDLL
fn do_dynload(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let path = full_dll_path(&path_arg(&arg(&args, 0))?);
    if let Some(k) = i.dlls.iter().position(|d| &*d.path == path) {
        // Already loaded: move it to the end of the list.
        let d = i.dlls.remove(k);
        i.dlls.push(d);
        return Ok(i.dlls.last().unwrap().to_value());
    }
    if let Err(e) = std::fs::metadata(&path) {
        return Err(Flow::error(format!(
            "unable to load shared object '{}':\n  {}: cannot open shared object file: {}",
            path,
            path,
            super::io::os_error_message(&e)
        )));
    }
    let base = path.rsplit('/').next().unwrap_or(&path);
    let name = base.strip_suffix(".so").filter(|n| !n.is_empty()).unwrap_or(base);
    // GNU R's own packages keep their Rust ports until their sources are
    // built against robigo (docs/core.md §8: such a build carries the
    // marker symbol); everything else is loaded
    // A base package's shared object may also come from robigo's build of
    // GNU R's sources (scripts/build-natives.sh); it is loaded in place of
    // GNU R's under GNU R's path, so the DLL looks as in GNU R.
    let file = if native::has_table(name) && !native::is_robigo_build(&path) {
        native::robigo_native(name)
    } else {
        Some(path.clone())
    };
    match native::bridge() {
        Some(b) if file.is_some() => {
            let file = file.unwrap();
            let local = !matches!(arg(&args, 1), Value::Lgl(v) if v.data.first() == Some(&0));
            let now = !matches!(arg(&args, 2), Value::Lgl(v) if v.data.first() == Some(&0));
            let h = b.load(i, &file, name, local, now)?;
            i.dlls.push(Dll::loaded(name, &path, h, b));
        }
        _ => i.dlls.push(Dll::new(name, &path)),
    }
    Ok(i.dlls.last().unwrap().to_value())
}

// R: Rdynload.c:do_dynunload
fn do_dynunload(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let path = full_dll_path(&path_arg(&arg(&args, 0))?);
    match i.dlls.iter().position(|d| &*d.path == path) {
        Some(k) => {
            let d = i.dlls.remove(k);
            if d.native != 0 {
                if let Some(b) = native::bridge() {
                    b.unload(i, d.native);
                }
            }
            Ok(Value::Null)
        }
        None => Err(Flow::error(format!("shared object '{}' was not loaded", path))),
    }
}

// R: Rdynload.c:R_getDllTable
fn do_get_dll_table(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let items: Vec<Value> = i.dlls.iter().map(|d| d.to_value()).collect();
    let names = Value::strs_from(i.dlls.iter().map(|d| d.name.to_string()));
    let mut v = Value::list(items);
    v.set_attr_raw(s::NAMES, names);
    v.set_attr_raw(s::CLASS, Value::str("DLLInfoList"));
    Ok(v)
}

// R: Rdynload.c:R_getRegisteredRoutines
fn do_get_registered_routines(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let Some(dll) = (match &x {
        Value::ExtPtr(_) => native::dll_of(i, &x),
        _ => None,
    }) else {
        return Err(Flow::error("R_getRegisteredRoutines() expects a DllInfo reference"));
    };
    let kinds = [Interface::C, Interface::Call, Interface::Fortran, Interface::External];
    let lists: Vec<Value> = kinds
        .iter()
        .map(|k| {
            Value::list(
                dll.routines.iter().filter(|r| r.interface == *k).map(|r| native::symbol_object(dll, r)).collect(),
            )
        })
        .collect();
    let mut v = Value::list(lists);
    v.set_attr_raw(s::NAMES, Value::strs_from([".C", ".Call", ".Fortran", ".External"]));
    Ok(v)
}

// R: Rdynload.c:R_getSymbolInfo
fn do_get_symbol_info(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'name' argument")),
    };
    let spackage = arg(&args, 1);
    let found = match &spackage {
        v if v.is_empty() => native::find_routine(i, &name, "", None),
        Value::Str(p) => native::find_routine(i, &name, p.data[0].as_deref().unwrap_or(""), None),
        v @ Value::ExtPtr(_) => match native::dll_of(i, v) {
            Some(d) => d.find(&name, None),
            None => return Err(Flow::error("NULL value passed for DllInfo")),
        },
        _ => return Err(Flow::error("must pass package name or DllInfo reference")),
    };
    Ok(match found {
        Some(r) => {
            let dll = i.dlls.iter().find(|d| d.name == r.package).unwrap();
            native::symbol_object(dll, &r)
        }
        None => Value::Null,
    })
}

// R: dotcode.c:do_isloaded
fn do_isloaded(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let str_arg = |v: &Value| match v {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => Some(s.data[0].clone().unwrap().to_string()),
        _ => None,
    };
    let Some(sym) = str_arg(&arg(&args, 0)) else {
        return Err(Flow::error("invalid 'symbol' argument"));
    };
    let pkg = if args.len() >= 2 {
        str_arg(&arg(&args, 1)).ok_or_else(|| Flow::error("invalid 'PACKAGE' argument"))?
    } else {
        String::new()
    };
    let interface = if args.len() >= 3 {
        let t = str_arg(&arg(&args, 2)).ok_or_else(|| Flow::error("invalid 'type' argument"))?;
        match t.as_str() {
            "C" => Some(Interface::C),
            "Fortran" => Some(Interface::Fortran),
            "Call" => Some(Interface::Call),
            "External" => Some(Interface::External),
            _ => None,
        }
    } else {
        None
    };
    Ok(Value::lgl(native::find_routine(i, &sym, &pkg, interface).is_some()))
}

/// Resolve `.NAME` and drop `PACKAGE`/`NAOK`/`DUP`/`ENCODING` arguments.
// R: dotcode.c:resolveNativeRoutine, pkgtrim, naokfind
fn resolve(
    i: &mut Interp,
    call: &Value,
    mut args: Vec<Arg>,
    interface: Interface,
) -> RResult<(std::rc::Rc<native::Routine>, Vec<Arg>)> {
    let mut package = String::new();
    let tag_is = |a: &Arg, n: &str| a.tag.is_some_and(|t| t.with_name(|x| x == n));
    if let Some(k) = args.iter().position(|a| tag_is(a, "PACKAGE")) {
        let p = args.remove(k);
        match &p.value {
            Value::Str(s) if s.data.len() == 1 => package = s.data[0].as_deref().unwrap_or("").to_string(),
            _ => return Err(Flow::error_call(call.clone(), "PACKAGE argument must be a single character string")),
        }
    }
    if matches!(interface, Interface::C | Interface::Fortran) {
        args.retain(|a| !(tag_is(a, "NAOK") || tag_is(a, "DUP") || tag_is(a, "ENCODING")));
    }
    if args.is_empty() {
        return Err(Flow::error_call(call.clone(), "'.NAME' is missing"));
    }
    let name = args.remove(0).value;
    let routine = match &name {
        Value::Str(s) if s.data.len() == 1 => {
            let n = s.data[0].as_deref().unwrap_or("NA");
            match native::find_routine(i, n, &package, Some(interface)) {
                Some(r) => r,
                None if package.is_empty() => {
                    return Err(Flow::error_call(
                        call.clone(),
                        format!("{} symbol name \"{}\" not in load table", symbol_kind(interface), n),
                    ));
                }
                None => {
                    return Err(Flow::error_call(
                        call.clone(),
                        format!(
                            "\"{}\" not available for {}() for package \"{}\"",
                            n,
                            match interface {
                                Interface::C => ".C",
                                Interface::Fortran => ".Fortran",
                                Interface::Call => ".Call",
                                Interface::External => ".External",
                            },
                            package
                        ),
                    ));
                }
            }
        }
        Value::List(_) if name.inherits("NativeSymbolInfo") => {
            match name.list_get("address").as_ref().and_then(native::routine_of) {
                Some(r) => r,
                None => return Err(Flow::error_call(call.clone(), "NULL value passed as symbol address")),
            }
        }
        Value::ExtPtr(_) => match native::routine_of(&name) {
            Some(r) => r,
            None => return Err(Flow::error_call(call.clone(), "NULL value passed as symbol address")),
        },
        _ => {
            return Err(Flow::error_call(
                call.clone(),
                "first argument must be a string (of length 1) or native symbol reference",
            ));
        }
    };
    Ok((routine, args))
}

/// R: dotcode.c:resolveNativeRoutine — "Fortran" for .Fortran, "C" for
/// every other interface.
fn symbol_kind(i: Interface) -> &'static str {
    if i == Interface::Fortran { "Fortran" } else { "C" }
}

#[allow(dead_code)]
fn interface_name(i: Interface) -> &'static str {
    match i {
        Interface::C => "C",
        Interface::Call => "Call",
        Interface::Fortran => "Fortran",
        Interface::External => "External",
    }
}

fn dot_code(i: &mut Interp, call: &Value, args: Vec<Arg>, interface: Interface, env: &Env) -> RResult<Value> {
    let (r, args) = resolve(i, call, args, interface)?;
    // R: dotcode.c do_External/do_dotCode — a registered routine with a
    // known arity (numArgs > -1) is checked on every interface, .External
    // included.
    if r.nargs >= 0 && args.len() as i32 != r.nargs {
        return Err(Flow::error_call(
            call.clone(),
            format!("Incorrect number of arguments ({}), expecting {} for '{}'", args.len(), r.nargs, r.name),
        ));
    }
    native::invoke(i, call, &r, args, env)
}

fn do_dotcall(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    dot_code(i, call, args, Interface::Call, env)
}

fn do_external(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    dot_code(i, call, args, Interface::External, env)
}

// R: dotcode.c:do_Externalgr, do_dotcallgr — the device's display list
// records the call (not while it runs); on an error recording stays off,
// as there
fn graphics_call(i: &mut Interp, call: &Value, args: Vec<Arg>, interface: Interface, env: &Env) -> RResult<Value> {
    let Some(b) = native::bridge() else {
        return dot_code(i, call, args, interface, env);
    };
    let record = b.graphics_begin(i)?;
    let name = if interface == Interface::Call { ".Call.graphics" } else { ".External.graphics" };
    let op = super::lookup_primitive(name).map(Value::Builtin).unwrap_or(Value::Null);
    let r = dot_code(i, call, args.clone(), interface, env)?;
    b.graphics_end(i, call, &op, &args, record)?;
    Ok(r)
}

fn do_external_gr(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    graphics_call(i, call, args, Interface::External, env)
}

fn do_dotcall_gr(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    graphics_call(i, call, args, Interface::Call, env)
}

fn do_dot_c(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    dot_code(i, call, args, Interface::C, env)
}

fn do_dot_fortran(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    dot_code(i, call, args, Interface::Fortran, env)
}
