//! S4 objects and formal method dispatch in the evaluator: GNU R's
//! objects.c (`standardGeneric`, the primitive-methods tables,
//! `R_possible_dispatch`, `asS4`, `R_do_new_object`), attrib.c (slots,
//! `R_data_class`, `S4_extends`, `R_getS4DataSlot`) and eval.c
//! (`R_execMethod`). The methods package's own natives are ported in
//! `native/methods.rs`; they reach this through the same entry points as
//! methods.so reaches libR.
//!
//! Plain values pay nothing: every hook is behind the object's S4 bit (an
//! object has a class attribute first) or the methods package's state.

use std::collections::HashMap;
use std::rc::Rc;

use crate::env::{BIND_ACTIVE, BIND_MISSING_DEFAULT, Env};
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

/// What `R_standardGeneric_ptr` points to.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum StdGeneric {
    /// Not set: methods is not loaded.
    #[default]
    Unset,
    /// `dispatchNonGeneric`: calls to `standardGeneric` while methods
    /// loads (or after it was turned off) call the non-generic version.
    NonGeneric,
    /// methods' `R_dispatchGeneric` (table dispatch, the default).
    Table,
    /// methods' `R_standardGeneric` (the old MethodsList dispatch).
    Old,
}

/// GNU R's `prim_methods_t`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimCode {
    NoMethods,
    NeedsReset,
    HasMethods,
    Suppressed,
}

/// One primitive's entry in the tables of objects.c (`prim_methods`,
/// `prim_generics`, `prim_mlist`).
#[derive(Clone)]
pub struct PrimEntry {
    pub code: PrimCode,
    pub generic: Option<Value>,
    pub mlist: Option<Value>,
}

/// The S4 state of an interpreter (GNU R's statics in objects.c and
/// attrib.c).
pub struct S4State {
    pub std_generic: StdGeneric,
    /// `quick_method_check_ptr`: `Table` is `R_quick_dispatch`, `Old` is
    /// `R_quick_method_check`.
    pub quick: StdGeneric,
    /// `R_MethodsNamespace`.
    pub methods_ns: Option<Env>,
    /// `allowPrimitiveMethods`.
    pub allow_prim: bool,
    /// Keyed by the primitive's address (GNU R: `PRIMOFFSET`).
    pub prim: HashMap<usize, PrimEntry>,
    /// `R_S4_extends_table`.
    pub extends_table: Option<Env>,
}

impl Default for S4State {
    fn default() -> Self {
        S4State {
            std_generic: StdGeneric::Unset,
            quick: StdGeneric::Unset,
            methods_ns: None,
            allow_prim: true,
            prim: HashMap::new(),
            extends_table: None,
        }
    }
}

fn prim_key(op: &'static Builtin) -> usize {
    op as *const Builtin as usize
}

/// Symbols of the slot and dispatch code.
pub mod syms {
    use crate::sym::Sym;
    thread_local! {
        static SYMS: [Sym; 18] = [
            Sym::new(".Data"),
            Sym::new(".S3Class"),
            Sym::new(".xData"),
            Sym::new("\u{1}NULL\u{1}"),
            Sym::new("generic"),
            Sym::new("package"),
            Sym::new(".Generic"),
            Sym::new(".Methods"),
            Sym::new(".defined"),
            Sym::new(".Method"),
            Sym::new(".target"),
            Sym::new("__Deferred_Default_Marker__"),
            Sym::new("virtual"),
            Sym::new("prototype"),
            Sym::new("className"),
            Sym::new("internal"),
            Sym::new(".nextMethod"),
            Sym::new("getGeneric"),
        ];
    }
    fn get(i: usize) -> Sym {
        SYMS.with(|s| s[i])
    }
    pub fn data() -> Sym {
        get(0)
    }
    pub fn s3class() -> Sym {
        get(1)
    }
    pub fn xdata() -> Sym {
        get(2)
    }
    pub fn pseudo_null() -> Sym {
        get(3)
    }
    pub fn generic() -> Sym {
        get(4)
    }
    pub fn package() -> Sym {
        get(5)
    }
    pub fn dot_generic() -> Sym {
        get(6)
    }
    pub fn dot_methods() -> Sym {
        get(7)
    }
    pub fn dot_defined() -> Sym {
        get(8)
    }
    pub fn dot_method() -> Sym {
        get(9)
    }
    pub fn dot_target() -> Sym {
        get(10)
    }
    pub fn deferred_default() -> Sym {
        get(11)
    }
    pub fn virtual_() -> Sym {
        get(12)
    }
    pub fn prototype() -> Sym {
        get(13)
    }
    pub fn class_name() -> Sym {
        get(14)
    }
    pub fn internal() -> Sym {
        get(15)
    }
    pub fn dot_next_method() -> Sym {
        get(16)
    }
    pub fn get_generic() -> Sym {
        get(17)
    }
}

/// The primitive whose method tables hold the methods of `generic` (the
/// dispatch name a builtin passes). `as.numeric` is the `as.double`
/// primitive in GNU R (base binds `as.numeric` to it); robigo's builtins
/// pass the name they were called by.
pub fn prim_op(generic: &str) -> Option<&'static Builtin> {
    crate::builtins::lookup_primitive(if generic == "as.numeric" { "as.double" } else { generic })
}

/// `R_deferred_default_method()`.
pub fn deferred_default() -> Value {
    Value::Sym(syms::deferred_default())
}

fn is_deferred_default(v: &Value) -> bool {
    matches!(v, Value::Sym(s) if *s == syms::deferred_default())
}

/// R's `isValidString`: a character vector whose first element is not NA.
pub fn is_valid_string(v: &Value) -> bool {
    matches!(v, Value::Str(s) if !s.data.is_empty() && s.data[0].is_some())
}

/// R's `isValidStringF`: a valid string whose first element is not "".
pub fn is_valid_string_f(v: &Value) -> bool {
    matches!(v, Value::Str(s) if !s.data.is_empty() && s.data[0].as_deref().is_some_and(|x| !x.is_empty()))
}

/// `R_findVarInFrame`: the binding's value (promises unforced; active
/// bindings called).
pub fn find_var_in_frame(interp: &mut Interp, env: &Env, sym: Sym) -> RResult<Option<Value>> {
    let b = env.0.frame().borrow().get(sym).map(|b| (b.value.clone(), b.flags));
    match b {
        None => Ok(None),
        Some((f, flags)) if flags & BIND_ACTIVE != 0 => {
            let global = interp.global.clone();
            interp.call_value(&f, vec![], &global).map(Some)
        }
        Some((v, _)) => Ok(Some(v)),
    }
}

/// `R_findVar`: the first binding of `sym` from `env` outwards.
pub fn find_var(interp: &mut Interp, env: &Env, sym: Sym) -> RResult<Option<Value>> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        if let Some(v) = find_var_in_frame(interp, &e, sym)? {
            return Ok(Some(v));
        }
        cur = e.parent();
    }
    Ok(None)
}

/// Whether the binding of `sym` in `env` is a missing argument
/// (`R_GetVarLocMISSING`), or `None` without a binding.
pub fn binding_missing(env: &Env, sym: Sym) -> Option<bool> {
    let f = env.0.frame().borrow();
    let b = f.get(sym)?;
    Some(b.flags & BIND_MISSING_DEFAULT != 0 || b.value.is_missing_arg())
}

/// Whether two values are the same object (not just equal).
pub fn same_object(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Lgl(x), Value::Lgl(y)) | (Value::Int(x), Value::Int(y)) => Rc::ptr_eq(x, y),
        (Value::Real(x), Value::Real(y)) => Rc::ptr_eq(x, y),
        (Value::Cplx(x), Value::Cplx(y)) => Rc::ptr_eq(x, y),
        (Value::Str(x), Value::Str(y)) => Rc::ptr_eq(x, y),
        (Value::Raw(x), Value::Raw(y)) => Rc::ptr_eq(x, y),
        (Value::List(x), Value::List(y)) | (Value::Expr(x), Value::Expr(y)) => Rc::ptr_eq(x, y),
        (Value::Pairlist(x), Value::Pairlist(y)) => Rc::ptr_eq(x, y),
        (Value::Obj(x), Value::Obj(y)) => Rc::ptr_eq(x, y),
        _ => a.ptr_eq(b),
    }
}

/// The first string of a character vector, "NA" for NA, like R's
/// `CHAR(asChar(x))` on strings.
fn as_char(v: &Value) -> String {
    match v {
        Value::Str(s) => s.data.first().map(|x| x.as_deref().unwrap_or("NA").to_string()).unwrap_or_default(),
        Value::Sym(s) => s.name().to_string(),
        _ => {
            let c = crate::coerce::to_str(v);
            c.first().map(|x| x.as_deref().unwrap_or("NA").to_string()).unwrap_or_else(|| "NA".to_string())
        }
    }
}

/// The data part (`.Data`, else `.xData`) of an S4 OBJSXP, as
/// `R_getS4DataSlot(x, <type>)` sees it before checking the type.
pub fn s4_data_part(x: &Value) -> Option<&Value> {
    let Value::Obj(o) = x else { return None };
    let a = o.attrs.as_deref()?;
    if !a.1 {
        return None;
    }
    a.get(syms::data()).or_else(|| a.get(syms::xdata()))
}

/// `R_data_class(obj, singleString)` as strings.
// R: attrib.c:R_data_class
pub fn data_class(obj: &Value, single: bool) -> Value {
    if let Some(klass) = obj.get_attr(s::CLASS) {
        let n = klass.len();
        if n == 1 || (n > 0 && !single) {
            return klass;
        }
        if n > 0 {
            return Value::strs(vec![klass.str1().flatten()]);
        }
    }
    if single {
        let nd = obj.get_attr(s::DIM).map(|d| d.len()).unwrap_or(0);
        if nd > 0 {
            return Value::str(if nd == 2 { "matrix" } else { "array" });
        }
        let c = crate::dispatch::implicit_class(obj);
        return Value::strs(vec![c.last().cloned()]);
    }
    crate::attrib::class_of(obj)
}

impl Interp {
    /// `isMethodsDispatchOn()`.
    #[inline]
    pub fn methods_dispatch_on(&self) -> bool {
        matches!(self.s4.std_generic, StdGeneric::Table | StdGeneric::Old)
    }

    /// `R_MethodsNamespace` (the global environment until methods sets it).
    pub fn methods_namespace(&self) -> Env {
        self.s4.methods_ns.clone().unwrap_or_else(|| self.global.clone())
    }

    /// `R_set_standardGeneric_ptr`.
    pub fn set_standard_generic(&mut self, val: StdGeneric, envir: Option<Env>) -> StdGeneric {
        let old = self.s4.std_generic;
        self.s4.std_generic = val;
        if let Some(e) = envir {
            self.s4.methods_ns = Some(e);
        }
        if self.s4.methods_ns.is_none() {
            self.s4.methods_ns = Some(self.global.clone());
        }
        old
    }

    /// `R_has_methods(op)`: could there be S4 methods for this primitive?
    // R: objects.c:R_has_methods
    #[inline]
    pub fn has_methods(&self, op: Option<&'static Builtin>) -> bool {
        if !self.methods_dispatch_on() {
            return false;
        }
        let Some(op) = op else { return true };
        if !self.s4.allow_prim {
            return false;
        }
        match self.s4.prim.get(&prim_key(op)) {
            None => false,
            Some(e) => !matches!(e.code, PrimCode::NoMethods | PrimCode::Suppressed),
        }
    }

    /// `R_primitive_methods(op)`.
    pub fn primitive_methods(&self, op: &'static Builtin) -> Value {
        self.s4.prim.get(&prim_key(op)).and_then(|e| e.mlist.clone()).unwrap_or(Value::Null)
    }

    /// `R_primitive_generic(op)`.
    pub fn primitive_generic(&self, op: &'static Builtin) -> Value {
        self.s4.prim.get(&prim_key(op)).and_then(|e| e.generic.clone()).unwrap_or(Value::Null)
    }

    /// `do_set_prim_method(op, code, fundef, mlist)`; returns the previous
    /// generic.
    // R: objects.c:do_set_prim_method
    pub fn set_prim_method(
        &mut self,
        op: &'static Builtin,
        code_string: &str,
        fundef: Value,
        mlist: Value,
    ) -> RResult<Value> {
        let b = code_string.as_bytes();
        let code = match (b.first(), b.get(1)) {
            (Some(b'c'), _) => PrimCode::NoMethods,
            (Some(b'r'), _) => PrimCode::NeedsReset,
            (Some(b's'), Some(b'e')) => PrimCode::HasMethods,
            (Some(b's'), Some(b'u')) => PrimCode::Suppressed,
            _ => {
                return Err(Flow::error(format!(
                    "invalid primitive methods code (\"{}\"): should be \"clear\", \"reset\", \"set\", or \"suppress\"",
                    code_string
                )));
            }
        };
        let e = self.s4.prim.entry(prim_key(op)).or_insert(PrimEntry {
            code: PrimCode::NoMethods,
            generic: None,
            mlist: None,
        });
        e.code = code;
        let value = e.generic.clone().unwrap_or(Value::Null);
        if code == PrimCode::Suppressed {
        } else if code == PrimCode::NoMethods && e.generic.is_some() {
            e.generic = None;
            e.mlist = None;
        } else if !fundef.is_null() && e.generic.is_none() {
            if !matches!(fundef, Value::Closure(_)) {
                return Err(Flow::error(format!(
                    "the formal definition of a primitive generic must be a function object (got type '{}')",
                    fundef.type_name()
                )));
            }
            e.generic = Some(fundef);
        }
        if code == PrimCode::HasMethods && !mlist.is_null() {
            e.mlist = Some(mlist);
        }
        Ok(value)
    }

    /// `R_set_prim_method(fname, op, code_vec, fundef, mlist)`.
    // R: objects.c:R_set_prim_method
    pub fn r_set_prim_method(
        &mut self,
        fname: Value,
        op: &Value,
        code_vec: &Value,
        fundef: Value,
        mlist: Value,
    ) -> RResult<Value> {
        if !is_valid_string(code_vec) {
            return Err(Flow::error("argument 'code' must be a character string"));
        }
        let code_string = as_char(code_vec);
        if op.is_null() {
            let value = Value::lgl(self.s4.allow_prim);
            match code_string.as_bytes().first() {
                Some(b'c' | b'C') => self.s4.allow_prim = false,
                Some(b's' | b'S') => self.s4.allow_prim = true,
                _ => {}
            }
            return Ok(value);
        }
        let op = match op {
            Value::Builtin(b) => *b,
            _ => {
                let internal = self.do_slot(op, syms::internal())?;
                let name = as_char(&internal);
                match crate::builtins::lookup_internal(&name) {
                    Some(b) => b,
                    None => {
                        return Err(Flow::error(format!(
                            "'internal' slot does not name an internal function: {}",
                            name
                        )));
                    }
                }
            }
        };
        self.set_prim_method(op, &code_string, fundef, mlist)?;
        Ok(fname)
    }

    /// `get_primitive_methods(op, rho)`: the environment of the generic
    /// for a primitive, from `getGeneric(name)`.
    fn get_primitive_methods(&mut self, op: &'static Builtin, rho: &Env) -> RResult<Value> {
        let call = Value::lang(Value::Sym(syms::get_generic()), vec![Arg::pos(Value::str(op.name))]);
        let val = self.eval(&call, rho)?;
        match &val {
            Value::Closure(c) if val.is_s4() => Ok(Value::Env(c.env.clone())),
            _ => Err(Flow::error(format!(
                "object returned as generic function \"{}\" does not appear to be one",
                op.name
            ))),
        }
    }

    /// `R_possible_dispatch`: try the formal method of primitive `op` for
    /// `args` (promises whose values are set). `None` means: go on with the
    /// internal code.
    // R: objects.c:R_possible_dispatch
    pub fn possible_dispatch(
        &mut self,
        call: &Value,
        op: &'static Builtin,
        args: Vec<Arg>,
        rho: &Env,
    ) -> RResult<Option<Value>> {
        let key = prim_key(op);
        let Some(entry) = self.s4.prim.get(&key) else {
            return Err(Flow::error("invalid primitive operation given for dispatch"));
        };
        let mut current = entry.code;
        if matches!(current, PrimCode::NoMethods | PrimCode::Suppressed) {
            return Ok(None);
        }
        if current == PrimCode::NeedsReset {
            self.set_prim_method(op, "suppressed", Value::Null, Value::Null)?;
            let mlist = self.get_primitive_methods(op, rho)?;
            self.set_prim_method(op, "set", Value::Null, mlist)?;
            current = self.s4.prim[&key].code;
        }
        let mlist = self.s4.prim[&key].mlist.clone();
        if let Some(mlist) = mlist.filter(|m| !m.is_null()) {
            if self.s4.quick != StdGeneric::Unset {
                let value = if self.s4.quick == StdGeneric::Table {
                    crate::native::methods::quick_dispatch(self, &args, &mlist)?
                } else {
                    crate::native::methods::quick_method_check(self, &args, &mlist)?
                };
                if matches!(value, Value::Builtin(_)) {
                    return Ok(None);
                }
                if let Value::Closure(c) = &value {
                    if value.inherits("internalDispatchMethod") {
                        return Ok(None);
                    }
                    let vars = vec![(syms::dot_generic(), Value::str(op.name))];
                    return self.apply_closure(call, c, args, rho, Some(vars)).map(Some);
                }
            }
        }
        let fundef = self.s4.prim[&key].generic.clone();
        let Some(Value::Closure(fundef)) = fundef else {
            return Err(Flow::error(format!(
                "primitive function \"{}\" has been set for methods but no generic function supplied",
                op.name
            )));
        };
        let value = self.apply_closure(call, &fundef, args, rho, None)?;
        if let Some(e) = self.s4.prim.get_mut(&key) {
            e.code = current;
        }
        if is_deferred_default(&value) { Ok(None) } else { Ok(Some(value)) }
    }

    /// The S4 part of `DispatchOrEval`: `x` is the first argument's value,
    /// `args` the evaluated arguments.
    pub fn try_dispatch_s4(&mut self, generic: &str, call: &Value, args: &[Arg], env: &Env) -> RResult<Option<Value>> {
        let Some(op) = prim_op(generic) else { return Ok(None) };
        if !self.has_methods(Some(op)) {
            return Ok(None);
        }
        let pargs = self.forced_promargs(call, args, env);
        self.possible_dispatch(call, op, pargs, env)
    }

    /// The S4 part of `DispatchGroup`.
    pub fn try_dispatch_group_s4(
        &mut self,
        generic: &str,
        is_ops: bool,
        call: &Value,
        args: &[Arg],
        env: &Env,
    ) -> RResult<Option<Value>> {
        let Some(op) = prim_op(generic) else { return Ok(None) };
        if !self.has_methods(Some(op)) {
            return Ok(None);
        }
        let mut pargs = self.forced_promargs(call, args, env);
        if is_ops {
            for a in pargs.iter_mut() {
                a.tag = None;
            }
        }
        self.possible_dispatch(call, op, pargs, env)
    }

    // ------------------------------------------------------------------
    // standardGeneric

    /// The generic function for a call to `standardGeneric(fname, ...)`.
    // R: objects.c:get_this_generic
    fn get_this_generic(&self, args: &[Arg]) -> Value {
        if args.len() > 1 {
            return args[1].value.clone();
        }
        let fname = args[0].value.str1().flatten();
        for c in self.frames.iter().rev() {
            if c.builtin || !c.function.is_object() {
                continue;
            }
            if let Some(g) = c.function.get_attr(syms::generic()) {
                if is_valid_string(&g) && g.str1().flatten() == fname {
                    return c.function.clone();
                }
            }
        }
        Value::Null
    }

    /// `do_standardGeneric`.
    // R: objects.c:do_standardGeneric
    pub fn standard_generic(&mut self, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
        if args.is_empty() {
            return Err(Flow::error_call(
                call.clone(),
                "argument to 'standardGeneric' must be a non-empty character string",
            ));
        }
        crate::builtins::character::check1arg(&args, call, "f")?;
        if self.s4.std_generic == StdGeneric::Unset {
            self.warningcall(
                call,
                "'standardGeneric' called without 'methods' dispatch enabled (will be ignored)".to_string(),
            )?;
            self.set_standard_generic(StdGeneric::NonGeneric, None);
        }
        let arg = &args[0].value;
        if !is_valid_string_f(arg) {
            return Err(Flow::error_call(
                call.clone(),
                "argument to 'standardGeneric' must be a non-empty character string",
            ));
        }
        let fdef = self.get_this_generic(&args);
        if fdef.is_null() {
            return Err(Flow::error(format!(
                "call to standardGeneric(\"{}\") apparently not from the body of that generic function",
                as_char(arg)
            )));
        }
        let fname = arg.clone();
        match self.s4.std_generic {
            StdGeneric::NonGeneric | StdGeneric::Unset => self.dispatch_non_generic(&fname, env),
            StdGeneric::Table => crate::native::methods::dispatch_generic(self, &fname, env, &fdef),
            StdGeneric::Old => crate::native::methods::standard_generic_old(self, &fname, env, &fdef),
        }
    }

    /// `dispatchNonGeneric`: call the non-generic version of `name` with the
    /// arguments of the call to the generic.
    // R: objects.c:dispatchNonGeneric
    fn dispatch_non_generic(&mut self, name: &Value, env: &Env) -> RResult<Value> {
        let symbol = Sym::new(&as_char(name));
        // (R's search of the enclosing frames always ends here.)
        let fun = self.base.get_local(symbol);
        let fun = match fun {
            Some(Value::Promise(p)) => self.force_promise(&p)?,
            Some(f) => f,
            None => {
                return Err(Flow::error(format!(
                    "unable to find a non-generic version of function \"{}\"",
                    as_char(name)
                )));
            }
        };
        let Some(k) = self.frames.iter().rposition(|c| !c.builtin && c.cloenv.ptr_eq(env)) else {
            return Err(Flow::error("dispatchNonGeneric: no context of the generic"));
        };
        let (call, sysparent) = (self.frames[k].call.clone(), self.frames[k].sysparent.clone());
        let e = match &call {
            Value::Lang(l) => {
                let mut nl = (**l).clone();
                nl.fun = fun;
                Value::Lang(Rc::new(nl))
            }
            other => other.clone(),
        };
        self.eval(&e, &sysparent)
    }

    /// `R_execMethod(op, rho)`: run method `op` in a new frame holding the
    /// bindings of the generic's frame `rho` for its formals, as the call
    /// of the generic.
    // R: eval.c:R_execMethod
    pub fn exec_method(&mut self, op: &Rc<Closure>, rho: &Env) -> RResult<Value> {
        let newrho = Env::with_capacity(Some(op.env.clone()), op.formals.len() + 6);
        for (k, f) in op.formals.iter().enumerate() {
            let symbol = f.tag.unwrap_or(s::EMPTY);
            let b = rho.0.frame().borrow().get(symbol).map(|b| (b.value.clone(), b.flags));
            let Some((val, flags)) = b else {
                return Err(Flow::error(format!(
                    "could not find symbol \"{}\" in environment of the generic function",
                    symbol.name()
                )));
            };
            let missing = flags & BIND_MISSING_DEFAULT != 0;
            // A default argument of the generic, not yet forced, becomes the
            // method's own default, evaluated in the method's frame. (robigo
            // binds constant defaults without a promise.)
            let unforced_default = missing
                && match &val {
                    Value::Promise(p) => {
                        p.value.borrow().is_none() && p.env.borrow().as_ref().is_some_and(|e| e.ptr_eq(rho))
                    }
                    v => !v.is_missing_arg(),
                };
            let val = if unforced_default {
                if f.value.is_missing_arg() { Value::missing_arg() } else { crate::interp::default_arg(op, k, &newrho) }
            } else {
                val
            };
            newrho.bind_arg(symbol, val, flags & BIND_MISSING_DEFAULT);
        }
        for sym in [syms::dot_defined(), syms::dot_method(), syms::dot_target()] {
            let v = find_var_in_frame(self, rho, sym)?.unwrap_or(Value::Null);
            newrho.set_local(sym, v);
        }
        for sym in [syms::dot_generic(), syms::dot_methods()] {
            let v = find_var(self, rho, sym)?.unwrap_or(Value::Null);
            newrho.set_local(sym, v);
        }
        let Some(k) = self.current_frame() else {
            return Err(Flow::error("R_execMethod: no calling context"));
        };
        let (call, callerenv, arglist) =
            (self.frames[k].call.clone(), self.frames[k].sysparent.clone(), self.frames[k].promargs.clone());
        self.exec_closure(&call, op, arglist, newrho, callerenv)
    }

    // ------------------------------------------------------------------
    // classes

    /// `S4_extends(klass, use_tab)`: the S3 class vector of an S4 class
    /// (its superclasses), cached.
    // R: attrib.c:S4_extends
    pub fn s4_extends(&mut self, klass: &Value, use_tab: bool) -> RResult<Value> {
        if !self.methods_dispatch_on() {
            return Ok(klass.clone());
        }
        let class = as_char(klass);
        let table = self.s4_extends_table();
        if use_tab {
            if let Some(v) = table.get_local(Sym::new(&class)) {
                return Ok(v);
            }
        }
        let ns = self.methods_namespace();
        let e = Value::lang(Value::sym(".extendsForS3"), vec![Arg::pos(klass.clone())]);
        let val = self.eval(&e, &ns)?;
        table.set_local(Sym::new(&class), val.clone());
        Ok(val)
    }

    fn s4_extends_table(&mut self) -> Env {
        self.s4.extends_table.get_or_insert_with(|| Env::new(None)).clone()
    }

    /// `.cache_class(class, extends)`.
    // R: attrib.c:cache_class
    pub fn cache_class(&mut self, class: &str, klass: Value) -> Value {
        let table = self.s4_extends_table();
        if klass.is_null() {
            table.remove(Sym::new(class));
        } else {
            table.set_local(Sym::new(class), klass.clone());
        }
        klass
    }

    /// `R_data_class2(obj)`: the class vector S3 dispatch uses.
    // R: attrib.c:R_data_class2
    pub fn data_class2(&mut self, obj: &Value) -> RResult<Vec<Rc<str>>> {
        if obj.is_s4() {
            if let Some(klass) = obj.get_attr(s::CLASS).filter(|k| !k.is_empty()) {
                let v = self.s4_extends(&klass, true)?;
                return Ok(crate::coerce::to_str(&v)
                    .into_iter()
                    .map(|x| x.unwrap_or_else(|| Rc::from("NA")))
                    .collect());
            }
        }
        Ok(crate::dispatch::dispatch_class(obj))
    }

    // ------------------------------------------------------------------
    // slots

    /// `R_do_slot(obj, name)`.
    // R: attrib.c:R_do_slot
    pub fn do_slot(&mut self, obj: &Value, name: Sym) -> RResult<Value> {
        if name == syms::data() {
            return self.data_part(obj);
        }
        match obj.get_attr(name) {
            Some(Value::Sym(p)) if p == syms::pseudo_null() => Ok(Value::Null),
            Some(v) => Ok(v),
            None => {
                if name == syms::s3class() {
                    return Ok(data_class(obj, false));
                }
                if name == s::NAMES && matches!(obj, Value::List(_)) {
                    return Ok(Value::Null);
                }
                let class = obj.get_attr(s::CLASS);
                let Some(class) = class else {
                    return Err(Flow::error(format!(
                        "cannot get a slot (\"{}\") from an object of type \"{}\"",
                        name.name(),
                        obj.type_name()
                    )));
                };
                Err(Flow::error(format!(
                    "no slot of name \"{}\" for this object of class \"{}\"",
                    name.name(),
                    as_char(&class)
                )))
            }
        }
    }

    /// A slot name argument as a symbol (`R_SLOT_INIT`).
    pub fn slot_name(&self, name: &Value) -> RResult<Sym> {
        match name {
            Value::Sym(s) => Ok(*s),
            Value::Str(v) if v.data.len() == 1 => Ok(Sym::new(v.data[0].as_deref().unwrap_or("NA"))),
            _ => Err(Flow::error("invalid type or length for slot name")),
        }
    }

    /// `R_has_slot(obj, name)`.
    pub fn has_slot(&self, obj: &Value, name: Sym) -> bool {
        if name == syms::data() && !matches!(obj, Value::Obj(_)) {
            return true;
        }
        obj.get_attr(name).is_some()
    }

    /// `R_do_slot_assign(obj, name, value)`.
    // R: attrib.c:R_do_slot_assign
    pub fn do_slot_assign(&mut self, mut obj: Value, name: &Value, value: Value) -> RResult<Value> {
        if obj.is_null() {
            return Err(Flow::error("attempt to set slot on NULL object"));
        }
        let name = match name {
            Value::Str(v) if v.data.len() == 1 => Sym::new(v.data[0].as_deref().unwrap_or("NA")),
            Value::Sym(s) => *s,
            _ => return Err(Flow::error("invalid type or length for slot name")),
        };
        if name == syms::data() {
            return self.set_data_part(obj, value);
        }
        let value = if value.is_null() { Value::Sym(syms::pseudo_null()) } else { value };
        if obj.attrs_slot_mut().is_none() && !matches!(obj, Value::Env(_) | Value::ExtPtr(_)) {
            return Err(Flow::error(format!(
                "robigo: slots on objects of type '{}' are not implemented yet",
                obj.type_name()
            )));
        }
        obj.set_attr_raw(name, value);
        Ok(obj)
    }

    fn data_part(&mut self, obj: &Value) -> RResult<Value> {
        let ns = self.methods_namespace();
        let e = Value::lang(
            Value::sym("getDataPart"),
            vec![Arg::pos(quote_value(obj.clone())), Arg::pos(Value::lgl(true))],
        );
        let mut val = self.eval(&e, &ns)?;
        val.set_s4(false);
        Ok(val)
    }

    fn set_data_part(&mut self, obj: Value, rhs: Value) -> RResult<Value> {
        let ns = self.methods_namespace();
        let e = Value::lang(Value::sym("setDataPart"), vec![Arg::pos(quote_value(obj)), Arg::pos(quote_value(rhs))]);
        let mut val = self.eval(&e, &ns)?;
        val.set_s4(true);
        Ok(val)
    }

    /// `R_getS4DataSlot(obj, type)`; `type` `None` is ANYSXP, `Some("S4")`
    /// is OBJSXP, otherwise a `typeof` name.
    // R: attrib.c:R_getS4DataSlot
    pub fn get_s4_data_slot(&mut self, obj: &Value, ty: Option<&str>) -> RResult<Value> {
        let objsxp = ty == Some("S4");
        let value;
        if !matches!(obj, Value::Obj(_)) || objsxp {
            let s3class = obj.get_attr(syms::s3class());
            if s3class.is_none() && objsxp {
                return Ok(Value::Null);
            }
            let mut o = obj.clone();
            match s3class {
                Some(c) => {
                    o.set_attr_raw(s::CLASS, c);
                    o.set_attr_raw(syms::s3class(), Value::Null);
                }
                None => o.set_attr_raw(s::CLASS, Value::Null),
            }
            o.set_s4(false);
            if objsxp {
                return Ok(o);
            }
            value = o;
        } else {
            let mut v = obj.get_attr(syms::data()).unwrap_or(Value::Null);
            if v.is_null() {
                v = obj.get_attr(syms::xdata()).unwrap_or(Value::Null);
            }
            value = v;
        }
        if !value.is_null() && (ty.is_none() || ty == Some(value.type_name())) { Ok(value) } else { Ok(Value::Null) }
    }

    /// `simple_as_environment(arg)`: the environment an S4 object extending
    /// "environment" holds, `None` for anything else.
    pub fn simple_as_environment(&mut self, v: &Value) -> RResult<Option<Env>> {
        if v.is_s4() && matches!(v, Value::Obj(_)) {
            if let Value::Env(e) = self.get_s4_data_slot(v, Some("environment"))? {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// `asS4(s, flag, complete)`.
    // R: objects.c:asS4
    pub fn as_s4(&mut self, mut s: Value, flag: bool, complete: i32) -> RResult<Value> {
        if flag == s.is_s4() {
            return Ok(s);
        }
        if flag {
            s.set_s4(true);
        } else {
            if complete != 0 {
                let value = self.get_s4_data_slot(&s, None)?;
                if !value.is_null() && !value.is_s4() {
                    return Ok(value);
                } else if complete == 1 {
                    let c = data_class(&s, false);
                    return Err(Flow::error(format!(
                        "object of class \"{}\" does not correspond to a valid S3 object",
                        as_char(&c)
                    )));
                } else {
                    return Ok(s);
                }
            }
            s.set_s4(false);
        }
        Ok(s)
    }

    /// `R_do_new_object(class_def)`.
    // R: objects.c:R_do_new_object
    pub fn do_new_object(&mut self, class_def: &Value) -> RResult<Value> {
        let e = self.do_slot(class_def, syms::virtual_())?;
        if crate::coerce::as_logical(&e) != 0 {
            let e = self.do_slot(class_def, syms::class_name())?;
            return Err(Flow::error(format!(
                "trying to generate an object from a virtual class (\"{}\")",
                as_char(&e)
            )));
        }
        let e = self.do_slot(class_def, syms::class_name())?;
        let mut value = self.do_slot(class_def, syms::prototype())?;
        let x_data_type = matches!(value, Value::Env(_) | Value::Sym(_) | Value::ExtPtr(_));
        if (matches!(value, Value::Obj(_)) || e.get_attr(syms::package()).is_some()) && !x_data_type {
            value.set_attr_raw(s::CLASS, e);
            value.set_s4(true);
        }
        Ok(value)
    }

    /// `R_getClassDef(what)` from the methods namespace (`NULL` if
    /// undefined).
    pub fn get_class_def(&mut self, what: &Value) -> RResult<Value> {
        if !self.methods_dispatch_on() {
            return Err(Flow::error("'methods' package not yet loaded"));
        }
        let ns = self.methods_namespace();
        let e = Value::lang(Value::sym("getClassDef"), vec![Arg::pos(what.clone())]);
        self.eval(&e, &ns)
    }

    /// `PrintObjectS4`: `methods::show(s)` evaluated in `env`.
    // R: print.c:PrintObjectS4
    pub fn show_s4(&mut self, v: &Value, env: &Env) -> RResult<()> {
        let ns = self.ns_registry.get_local(Sym::new("methods"));
        let Some(Value::Env(ns)) = ns else {
            return Err(Flow::error("missing methods namespace: this should not happen"));
        };
        let fun = match ns.get_local(Sym::new("show")) {
            Some(Value::Promise(p)) => self.force_promise(&p)?,
            Some(f) => f,
            None => return Err(Flow::error("missing show() in methods namespace: this should not happen")),
        };
        let call = Value::lang(fun, vec![Arg::pos(quote_value(v.clone()))]);
        self.eval(&call, env)?;
        Ok(())
    }
}

/// An argument for a constructed call that evaluates to `v` itself (R's C
/// code puts values into calls directly; language objects need quoting).
pub fn quote_value(v: Value) -> Value {
    match &v {
        Value::Sym(_) | Value::Lang(_) | Value::Promise(_) | Value::Dots(_) => {
            Value::Promise(Rc::new(Promise::forced(v.clone(), v)))
        }
        _ => v,
    }
}
