//! S3 dispatch: `UseMethod`, internal generics and group generics
//! (GNU R's objects.c `usemethod` and eval.c `DispatchOrEval`/`DispatchGroup`).

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::{Context, Interp};
use crate::sym::{Sym, s};
use crate::value::*;

/// R: eval.c:DispatchGroup compares the two Ops methods with
/// `R_compute_identical(lsxp, rsxp, 16 + 1 + 2 + 4)`: bitwise numbers and
/// NAs, attributes in order, closure environments compared, srcrefs ignored.
const OPS_METHOD_IDENTICAL: crate::builtins::identical::IdFlags = crate::builtins::identical::IdFlags {
    num_eq: false,
    single_na: false,
    attrib_as_set: false,
    ignore_env: false,
    ignore_bytecode: true,
    ignore_srcref: true,
};

/// R's `R_data_class2`: the class vector used for S3 dispatch, including
/// implicit classes.
pub fn dispatch_class(v: &Value) -> Vec<crate::value::RString> {
    class_strings(&dispatch_class_value(v))
}

/// The elements of a class vector as dispatch reads them
/// (`translateChar`: `NA` reads as "NA").
pub fn class_strings(klass: &Value) -> Vec<crate::value::RString> {
    match klass {
        Value::Str(c) => {
            c.data.iter().map(|x| x.clone().unwrap_or_else(|| crate::value::RString::from("NA"))).collect()
        }
        _ => vec![],
    }
}

/// The class name an element of a class vector dispatches on.
#[inline]
pub(crate) fn class_elt(x: &RStr) -> &str {
    x.as_deref().unwrap_or("NA")
}

/// R's `R_data_class2` for an object that is not S4, as R returns it: the
/// class attribute itself, or one of the shared implicit class vectors
/// (attrib.c `Type2DefaultClass`, built once per type and shape).
// R: attrib.c:R_data_class2
pub fn dispatch_class_value(v: &Value) -> Value {
    if let Some(c @ Value::Str(_)) = v.get_attr(s::CLASS) {
        if !c.is_empty() {
            return c;
        }
    }
    let shape: u8 = match v.get_attr(s::DIM) {
        None => 0,
        Some(d) if d.is_empty() => 0,
        Some(d) if d.len() == 2 => 2,
        Some(_) => 1,
    };
    let tclass: &'static str = match v {
        Value::Int(_) => "integer",
        Value::Real(_) => "double",
        Value::Lang(l) => lang_class(l),
        other => implicit_type_class(other),
    };
    thread_local! {
        static IMPLICIT: std::cell::RefCell<Vec<(u8, &'static str, Value)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    IMPLICIT.with(|c| {
        if let Some((_, _, k)) = c.borrow().iter().find(|(sh, t, _)| *sh == shape && *t == tclass) {
            return k.clone();
        }
        let mut out: Vec<&str> = Vec::new();
        match shape {
            0 => {}
            2 => out.extend(["matrix", "array"]),
            _ => out.push("array"),
        }
        match v {
            Value::Int(_) | Value::Real(_) => out.extend([tclass, "numeric"]),
            _ => out.push(tclass),
        }
        let k = Value::strs(out.into_iter().map(|x| Some(crate::value::RString::from(x))).collect());
        c.borrow_mut().push((shape, tclass, k.clone()));
        k
    })
}

/// `class(x)` for objects without a class attribute (R's `R_data_class`
/// with singleString = FALSE).
pub fn implicit_class(v: &Value) -> Vec<crate::value::RString> {
    if let Some(Value::Str(c)) = v.get_attr(s::CLASS) {
        if !c.data.is_empty() {
            return c.data.iter().map(|x| x.clone().unwrap_or_else(|| crate::value::RString::from("NA"))).collect();
        }
    }
    let ndim = v.get_attr(s::DIM).map(|d| d.len()).unwrap_or(0);
    match ndim {
        0 => {}
        2 => return vec![crate::value::RString::from("matrix"), crate::value::RString::from("array")],
        _ => return vec![crate::value::RString::from("array")],
    }
    let s = match v {
        Value::Int(_) => "integer",
        Value::Real(_) => "numeric",
        Value::Lang(l) => lang_class(l),
        other => implicit_type_class(other),
    };
    vec![crate::value::RString::from(s)]
}

fn lang_class(l: &Lang) -> &'static str {
    match l.fun.as_sym().map(|s| s.name()) {
        Some(n) => match &*n {
            "if" => "if",
            "while" => "while",
            "for" => "for",
            "=" => "=",
            "<-" => "<-",
            "(" => "(",
            "{" => "{",
            _ => "call",
        },
        None => "call",
    }
}

fn implicit_type_class(v: &Value) -> &'static str {
    match v {
        Value::Null => "NULL",
        Value::Sym(_) => "name",
        Value::Lgl(_) => "logical",
        Value::Int(_) => "integer",
        Value::Real(_) => "numeric",
        Value::Cplx(_) => "complex",
        Value::Str(_) => "character",
        Value::Raw(_) => "raw",
        Value::List(_) => "list",
        Value::Expr(_) => "expression",
        Value::Pairlist(_) => "pairlist",
        Value::Lang(_) => "call",
        Value::Closure(_) | Value::Builtin(_) => "function",
        Value::Env(_) => "environment",
        Value::Promise(_) => "promise",
        Value::Bytecode(_) => "bytecode",
        Value::Dots(_) => "...",
        // (a deferred package ALTREP value by its type: crate::altrep)
        Value::ExtPtr(_) => match crate::altrep::deferred_type(v) {
            Some(crate::value::VType::Str) => "character",
            Some(crate::value::VType::List) => "list",
            _ => "externalptr",
        },
        // R: attrib.c:R_data_class (OBJSXP)
        Value::Obj(_) => {
            if v.is_s4() {
                "S4"
            } else {
                "object"
            }
        }
    }
}

pub struct DispatchInfo {
    /// `.Generic`, a character string.
    pub generic: Value,
    /// `.Group`, a character string.
    pub group: Value,
    pub method: Sym,
    pub call_env: Env,
    pub def_env: Env,
}

/// R's `R_BlankScalarString`, the `.Group` of a method `UseMethod` and
/// internal dispatch call: one shared vector, as in R.
pub(crate) fn blank_scalar_string() -> Value {
    thread_local! {
        static BLANK: Value = Value::str("");
    }
    BLANK.with(|b| b.clone())
}

/// `.Method` for a `UseMethod` dispatch, the method's name as a character
/// scalar. R allocates one per dispatch (`ScalarString(PRINTNAME(...))`);
/// values are copy-on-modify here, so one shared vector per method is
/// indistinguishable and saves an allocation per call. The table is
/// direct-mapped by symbol id; a slot is used only for its own symbol.
fn method_name_value(method: Sym) -> Value {
    const SLOTS: usize = 64;
    thread_local! {
        static NAMES: std::cell::RefCell<[Option<(Sym, Value)>; SLOTS]> =
            const { std::cell::RefCell::new([const { None }; SLOTS]) };
    }
    let slot = method.index() % SLOTS;
    NAMES.with(|t| {
        let mut t = t.borrow_mut();
        if let Some((sym, v)) = &t[slot] {
            if *sym == method {
                return v.clone();
            }
        }
        let v = Value::strs(vec![Some(method.name())]);
        t[slot] = Some((method, v.clone()));
        v
    })
}

/// `topenv(callenv)` for a run of method lookups from one call
/// environment. Without R code running in between (which could change an
/// enclosure or bind `.packageName`), every lookup's `topenv` is the same,
/// so it is computed once and recomputed only after R code ran.
pub(crate) struct LookupScope {
    top: Option<(u64, Env)>,
}

impl LookupScope {
    pub(crate) fn new() -> Self {
        LookupScope { top: None }
    }

    fn top(&mut self, interp: &Interp, callenv: &Env) -> Env {
        let now = crate::env::r_epoch();
        match &self.top {
            Some((epoch, top)) if *epoch == now => top.clone(),
            _ => {
                let top = interp.topenv(callenv);
                self.top = Some((now, top.clone()));
                top
            }
        }
    }
}

impl Interp {
    /// `topenv(envir)`: first enclosing global, base, package or namespace
    /// environment, or one that defines `.packageName`.
    // R: envir.c:topenv
    pub fn topenv(&self, env: &Env) -> Env {
        self.topenv_from(env, None)
    }

    /// `topenv(envir, matchThisEnv)`: the search also stops at
    /// `matchThisEnv` (the `topLevelEnvironment` option, which `sys.source`
    /// sets to its `envir`).
    // R: envir.c:topenv
    pub fn topenv_from(&self, env: &Env, target: Option<&Env>) -> Env {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            // R's loop condition: R_EmptyEnv is never matched against
            // `target`, even when target is the empty environment.
            if e.ptr_eq(&self.empty) {
                break;
            }
            if target.is_some_and(|t| e.ptr_eq(t))
                || e.ptr_eq(&self.global)
                || e.ptr_eq(&self.base)
                || e.ptr_eq(&self.base_ns)
                || e.package_env_name().is_some()
                || is_namespace(&e)
                || e.has_local(crate::sym::s::DOT_PACKAGE_NAME)
            {
                return e;
            }
            cur = e.parent();
        }
        self.global.clone()
    }

    fn find_fun_in_range(&mut self, sym: Sym, from: &Env, to: &Env) -> RResult<Option<Value>> {
        let mut cur = Some(from.clone());
        while let Some(e) = cur {
            if e.ptr_eq(&self.empty) {
                break;
            }
            if let Some(v) = e.get_local(sym) {
                let v = self.force(v)?;
                if v.is_function() {
                    return Ok(Some(v));
                }
            }
            if e.ptr_eq(to) {
                return Ok(None);
            }
            cur = e.parent();
        }
        Ok(None)
    }

    fn find_fun_base_after_global(&mut self, sym: Sym, from: &Env) -> RResult<Option<Value>> {
        let mut cur = Some(from.clone());
        while let Some(e) = cur {
            if e.ptr_eq(&self.empty) {
                break;
            }
            if let Some(v) = e.get_local(sym) {
                let v = self.force(v)?;
                if v.is_function() {
                    return Ok(Some(v));
                }
            }
            cur = if e.ptr_eq(&self.global) { Some(self.base.clone()) } else { e.parent() };
        }
        Ok(None)
    }

    /// `R_LookupMethod`.
    pub fn lookup_method(&mut self, method: Sym, callenv: &Env, defenv: &Env) -> RResult<Option<Value>> {
        self.lookup_method_in(method, callenv, defenv, &mut LookupScope::new())
    }

    /// [`Self::lookup_method`], with `topenv(callenv)` from `scope`.
    // R: objects.c:R_LookupMethod
    pub(crate) fn lookup_method_in(
        &mut self,
        method: Sym,
        callenv: &Env,
        defenv: &Env,
        scope: &mut LookupScope,
    ) -> RResult<Option<Value>> {
        let top = scope.top(self, callenv);
        if let Some(f) = self.find_fun_in_range(method, callenv, &top)? {
            return Ok(Some(f));
        }
        if let Some(table) = defenv.get_local(s::DOT_S3_METHODS_TABLE) {
            if let Value::Env(t) = self.force(table)? {
                if let Some(v) = t.get_local(method) {
                    let v = self.force(v)?;
                    if v.is_function() {
                        return Ok(Some(v));
                    }
                }
            }
        }
        // Methods registered for base generics live in base's table.
        if !defenv.ptr_eq(&self.base) {
            if let Some(Value::Env(t)) = self.base_binding(s::DOT_S3_METHODS_TABLE) {
                if let Some(v) = t.get_local(method) {
                    let v = self.force(v)?;
                    if v.is_function() {
                        return Ok(Some(v));
                    }
                }
            }
        }
        // The base namespace (aliased to the base env here) is enclosed by
        // R_GlobalEnv in R, so lookups from base code continue there.
        let start = if top.ptr_eq(&self.global) {
            Some(self.base.clone())
        } else if top.ptr_eq(&self.base_ns) {
            Some(self.global.clone())
        } else {
            top.parent()
        };
        match start {
            Some(st) => self.find_fun_base_after_global(method, &st),
            None => Ok(None),
        }
    }

    /// Find a method for `generic` and the object's classes and apply it.
    /// `cptr` is the index of the context of the generic (or a temporary
    /// context for internal dispatch). Returns `None` if no method exists.
    #[allow(clippy::too_many_arguments)]
    pub fn usemethod(
        &mut self,
        generic: &str,
        obj: &Value,
        call: &Value,
        promargs: Vec<Arg>,
        rho: &Env,
        callenv: &Env,
        defenv: &Env,
    ) -> RResult<Option<Value>> {
        self.usemethod_ex(generic, None, obj, call, promargs, rho, callenv, defenv)
    }

    /// [`Self::usemethod`]; `generic_val` is `generic` as an R character
    /// string to bind as `.Generic`, when the caller has one.
    // R: objects.c:usemethod
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn usemethod_ex(
        &mut self,
        generic: &str,
        generic_val: Option<&Value>,
        obj: &Value,
        call: &Value,
        promargs: Vec<Arg>,
        rho: &Env,
        callenv: &Env,
        defenv: &Env,
    ) -> RResult<Option<Value>> {
        let klass = if obj.is_s4() { self.data_class2_value(obj)? } else { dispatch_class_value(obj) };
        let mut scope = LookupScope::new();
        let gval = || generic_val.cloned().unwrap_or_else(|| Value::str(generic));
        if let Value::Str(kv) = &klass {
            for (i, c) in kv.data.iter().enumerate() {
                let method = Sym::dotted(generic, class_elt(c));
                if let Some(f) = self.lookup_method_in(method, callenv, defenv, &mut scope)? {
                    if method.with_name(|n| n == "sort.list") {
                        continue;
                    }
                    let dot_class = if i > 0 {
                        let mut v = Value::strs(kv.data[i..].to_vec());
                        v.set_attr_raw(s::PREVIOUS, klass.clone());
                        v
                    } else {
                        klass.clone()
                    };
                    let info = DispatchInfo {
                        generic: gval(),
                        group: blank_scalar_string(),
                        method,
                        call_env: callenv.clone(),
                        def_env: defenv.clone(),
                    };
                    return self.dispatch_method(&f, dot_class, &info, call, promargs, rho).map(Some);
                }
            }
        }
        let method = Sym::dotted(generic, "default");
        if let Some(f) = self.lookup_method_in(method, callenv, defenv, &mut scope)? {
            let info = DispatchInfo {
                generic: gval(),
                group: blank_scalar_string(),
                method,
                call_env: callenv.clone(),
                def_env: defenv.clone(),
            };
            return self.dispatch_method(&f, Value::Null, &info, call, promargs, rho).map(Some);
        }
        Ok(None)
    }

    fn dispatch_method(
        &mut self,
        f: &Value,
        dot_class: Value,
        info: &DispatchInfo,
        call: &Value,
        promargs: Vec<Arg>,
        rho: &Env,
    ) -> RResult<Value> {
        let vars = vec![
            (s::DOT_GENERIC, info.generic.clone()),
            (s::DOT_CLASS, dot_class),
            (s::DOT_METHOD, method_name_value(info.method)),
            (s::DOT_GENERIC_CALL_ENV, Value::Env(info.call_env.clone())),
            (s::DOT_GENERIC_DEF_ENV, Value::Env(info.def_env.clone())),
            (s::DOT_GROUP, info.group.clone()),
        ];
        let newcall = match call {
            Value::Lang(l) => {
                let mut nl = (**l).clone();
                nl.fun = Value::Sym(info.method);
                Value::Lang(Rc::new(nl))
            }
            other => other.clone(),
        };
        // Mark the generic's frame so the method inherits its sysparent.
        if let Some(c) = self.frames.last_mut() {
            c.generic = true;
        }
        let r = match f {
            Value::Closure(c) => self.apply_closure(&newcall, c, promargs, rho, Some(vars)),
            Value::Builtin(_) => self.apply_function(f, promargs, rho, &newcall),
            _ => Err(Flow::error("attempt to apply non-function")),
        };
        if let Some(c) = self.frames.last_mut() {
            c.generic = false;
        }
        r
    }

    /// `DispatchOrEval` for builtins whose arguments are already evaluated.
    /// Returns `Some(result)` if a method was applied.
    pub fn try_dispatch(&mut self, generic: &str, call: &Value, args: &[Arg], env: &Env) -> RResult<Option<Value>> {
        let Some(x) = args.first() else { return Ok(None) };
        if !x.value.is_object() {
            return Ok(None);
        }
        // R: eval.c:DispatchOrEval — formal methods come first.
        if x.value.is_s4() && self.methods_dispatch_on() {
            if let Some(v) = self.try_dispatch_s4(generic, call, args, env)? {
                return Ok(Some(v));
            }
        }
        if let Value::Lang(l) = call {
            if let Some(sym) = l.fun_sym() {
                if sym.name().ends_with(".default") {
                    return Ok(None);
                }
            }
        }
        let pargs = self.forced_promargs(call, args, env);
        self.dispatch_s3_temp_context(generic, &x.value.clone(), call, pargs, env)
    }

    /// `DispatchOrEval` with promised arguments: S4 methods of the
    /// primitive (for S4 objects), then S3 methods.
    pub fn dispatch_with_temp_context(
        &mut self,
        generic: &str,
        obj: &Value,
        call: &Value,
        pargs: Vec<Arg>,
        env: &Env,
    ) -> RResult<Option<Value>> {
        if obj.is_s4() && self.methods_dispatch_on() {
            if let Some(op) = crate::s4::prim_op(generic) {
                if self.has_methods(Some(op)) {
                    if let Some(v) = self.possible_dispatch(call, op, pargs.clone(), env)? {
                        return Ok(Some(v));
                    }
                }
            }
        }
        self.dispatch_s3_temp_context(generic, obj, call, pargs, env)
    }

    /// [`Self::dispatch_with_temp_context`] for `@<-`, whose arguments are
    /// promised already.
    pub fn dispatch_promised(
        &mut self,
        generic: &str,
        obj: &Value,
        call: &Value,
        pargs: Vec<Arg>,
        env: &Env,
    ) -> RResult<Option<Value>> {
        if let Value::Lang(l) = call {
            if let Some(sym) = l.fun_sym() {
                if sym.name().ends_with(".default") && !obj.is_s4() {
                    return Ok(None);
                }
            }
        }
        self.dispatch_with_temp_context(generic, obj, call, pargs, env)
    }

    /// S3 dispatch with promise arguments (for specials such as `[`).
    fn dispatch_s3_temp_context(
        &mut self,
        generic: &str,
        obj: &Value,
        call: &Value,
        pargs: Vec<Arg>,
        env: &Env,
    ) -> RResult<Option<Value>> {
        // R: eval.c:DispatchOrEval — begincontext(CTXT_RETURN, call, rho1,
        // rho, pargs, op): a function context, so sys.calls(),
        // sys.function() and parent.frame() see it as GNU R's do (rlang's
        // error_call() climbs from an S3 method to this frame's call).
        let rho1 = Env::new(Some(env.clone()));
        let op = crate::s4::prim_op(generic)
            .or_else(|| crate::builtins::lookup_internal(generic))
            .map_or(Value::Null, Value::Builtin);
        if self.srcref.is_some() {
            self.srcref_begin(false);
        }
        self.frames.push(Context {
            call: call.clone(),
            function: op,
            cloenv: rho1.clone(),
            sysparent: env.clone(),
            promargs: pargs.clone(),
            on_exit: vec![],
            handlers: self.handlers.clone(),
            restarts: self.restarts.clone(),
            generic: false,
            builtin: false,
            prev_bc: self.bc,
        });
        let base = self.base.clone();
        let r = self.usemethod(generic, obj, call, pargs, &rho1, env, &base);
        let ctx = self.frames.pop().unwrap();
        self.srcref_ended(self.frames.len());
        self.handlers = ctx.handlers;
        self.restarts = ctx.restarts;
        r
    }

    /// Wrap evaluated arguments in forced promises whose expressions are the
    /// original call arguments where they correspond one-to-one.
    pub fn forced_promargs(&self, call: &Value, args: &[Arg], _env: &Env) -> Vec<Arg> {
        let exprs: Option<Vec<Value>> = match call {
            Value::Lang(l)
                if l.args.len() == args.len() && !l.args.iter().any(|a| a.value.as_sym() == Some(s::DOTS)) =>
            {
                Some(l.args.iter().map(|a| a.value.clone()).collect())
            }
            _ => None,
        };
        args.iter()
            .enumerate()
            .map(|(i, a)| {
                let expr = exprs.as_ref().map(|e| e[i].clone()).unwrap_or_else(|| a.value.clone());
                Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced(expr, a.value.clone()))))
            })
            .collect()
    }

    /// `DispatchGroup` for Ops, Math, Summary, Complex and matrixOps.
    pub fn try_dispatch_group(
        &mut self,
        group: &str,
        generic: &str,
        call: &Value,
        args: &[Arg],
        env: &Env,
    ) -> RResult<Option<Value>> {
        let a0 = args.first().map(|a| a.value.is_object()).unwrap_or(false);
        let a1 = args.get(1).map(|a| a.value.is_object()).unwrap_or(false);
        if !a0 && !a1 {
            return Ok(None);
        }
        // R: eval.c:DispatchGroup — try for a formal method first.
        if self.methods_dispatch_on() {
            let use_s4 = match args.len() {
                1 => args[0].value.is_s4(),
                2 => args[0].value.is_s4() || args[1].value.is_s4(),
                _ => true,
            };
            if use_s4 {
                let is_ops = group == "Ops" || group == "matrixOps";
                if let Some(v) = self.try_dispatch_group_s4(generic, is_ops, call, args, env)? {
                    return Ok(Some(v));
                }
            }
        }
        if let Value::Lang(l) = call {
            if let Some(sym) = l.fun_sym() {
                let n = sym.name();
                if let Some(dot) = n.find('.') {
                    if &n[dot + 1..] == "default" {
                        return Ok(None);
                    }
                }
            }
        }
        let is_ops = group == "Ops" || group == "matrixOps";
        let nargs = if is_ops { args.len() } else { 1 };
        if nargs == 1 && !a0 {
            return Ok(None);
        }
        // The class attributes (no copies: their strings are read in place).
        let class_of = |v: &Value| -> Value {
            match v.get_attr(s::CLASS) {
                Some(c @ Value::Str(_)) => c,
                _ => Value::Null,
            }
        };
        let lclass = class_of(&args[0].value);
        let rclass = if nargs == 2 { class_of(&args[1].value) } else { Value::Null };
        let mut scope = LookupScope::new();
        let (lsxp, lgr, lmeth, lwhich) = self.findmethod(&lclass, group, generic, env, &mut scope)?;
        let (rsxp, rgr, rmeth, rwhich) = if nargs == 2 {
            self.findmethod(&rclass, group, generic, env, &mut scope)?
        } else {
            (None, false, None, 0)
        };
        if lsxp.is_none() && rsxp.is_none() {
            return Ok(None);
        }
        let (mut lsxp, mut lgr, mut lmeth, mut lclass, mut lwhich) = (lsxp, lgr, lmeth, lclass, lwhich);
        let mut rsxp = rsxp;
        if let (Some(l), Some(r)) = (&lsxp, &rsxp) {
            if !l.ptr_eq(r) {
                let lname = lmeth.as_ref().map(|m| m.name().to_string()).unwrap_or_default();
                let rname = rmeth.as_ref().map(|m| m.name().to_string()).unwrap_or_default();
                if rname == "Ops.difftime" && matches!(lname.as_str(), "+.POSIXt" | "-.POSIXt" | "+.Date" | "-.Date") {
                    rsxp = None;
                } else if lname == "Ops.difftime" && matches!(rname.as_str(), "+.POSIXt" | "+.Date") {
                    lsxp = None;
                } else if !crate::builtins::identical::identical_flags(l, r, OPS_METHOD_IDENTICAL) {
                    // R: eval.c:DispatchGroup — ask chooseOpsMethod(), forward then reversed.
                    let (x, y) = (&args[0].value, &args[1].value);
                    if self.choose_ops_method(x, y, l, r, call, false, env)? {
                        rsxp = None;
                    } else if self.choose_ops_method(y, x, r, l, call, true, env)? {
                        lsxp = None;
                    } else {
                        let msg = format!("Incompatible methods (\"{}\", \"{}\") for \"{}\"", lname, rname, generic);
                        self.warning(msg)?;
                        return Ok(None);
                    }
                }
            }
        }
        if lsxp.is_none() {
            lsxp = rsxp;
            lmeth = rmeth;
            lgr = rgr;
            lclass = rclass;
            lwhich = rwhich;
        }
        let Some(f) = lsxp else { return Ok(None) };
        let lmeth = lmeth.unwrap();
        let Value::Str(lcv) = &lclass else { unreachable!("a method was found for a class") };
        let dispatch_class_name = class_elt(&lcv.data[lwhich]);
        let mname = lmeth.name();
        let m: Vec<RStr> = args[..nargs]
            .iter()
            .map(|a| match class_of(&a.value) {
                Value::Str(t) if t.data.iter().any(|c| class_elt(c) == dispatch_class_name) => Some(mname.clone()),
                _ => Some(crate::value::RString::from("")),
            })
            .collect();
        let dot_class = if lwhich == 0 { lclass.clone() } else { Value::strs(lcv.data[lwhich..].to_vec()) };
        let vars = vec![
            (s::DOT_GENERIC, Value::str(generic)),
            (s::DOT_CLASS, dot_class),
            (s::DOT_METHOD, Value::strs(m)),
            (s::DOT_GENERIC_CALL_ENV, Value::Env(env.clone())),
            (s::DOT_GENERIC_DEF_ENV, Value::Env(self.base.clone())),
            (s::DOT_GROUP, if lgr { Value::str(group) } else { blank_scalar_string() }),
        ];
        let newcall = match call {
            Value::Lang(l) => Value::lang(Value::Sym(lmeth), l.args.clone()),
            other => other.clone(),
        };
        let mut pargs = self.forced_promargs(call, args, env);
        if is_ops {
            for a in pargs.iter_mut() {
                a.tag = None;
            }
        }
        match &f {
            Value::Closure(c) => self.apply_closure(&newcall, c, pargs, env, Some(vars)).map(Some),
            _ => self.apply_function(&f, pargs, env, &newcall).map(Some),
        }
    }

    /// R: eval.c:R_chooseOpsMethod — evaluates
    /// `base::chooseOpsMethod(x, y, mx, my, cl, rev)` in a new environment
    /// enclosed by `rho`.
    #[allow(clippy::too_many_arguments)]
    fn choose_ops_method(
        &mut self,
        x: &Value,
        y: &Value,
        mx: &Value,
        my: &Value,
        call: &Value,
        rev: bool,
        rho: &Env,
    ) -> RResult<bool> {
        let (xs, ys, mxs, mys, cls, revs) =
            (Sym::new("x"), Sym::new("y"), Sym::new("mx"), Sym::new("my"), Sym::new("cl"), Sym::new("rev"));
        let newrho = Env::new(Some(rho.clone()));
        newrho.set_local(xs, x.clone());
        newrho.set_local(ys, y.clone());
        newrho.set_local(mxs, mx.clone());
        newrho.set_local(mys, my.clone());
        newrho.set_local(cls, call.clone());
        newrho.set_local(revs, Value::lgl(rev));
        let fun = Value::lang(
            Value::Sym(s::NAMESPACE_GET),
            vec![Arg::pos(Value::Sym(Sym::new("base"))), Arg::pos(Value::Sym(Sym::new("chooseOpsMethod")))],
        );
        let expr =
            Value::lang(fun, [xs, ys, mxs, mys, cls, revs].into_iter().map(|s| Arg::pos(Value::Sym(s))).collect());
        let ans = self.eval(&expr, &newrho)?;
        if matches!(ans, Value::Null) {
            return Ok(false);
        }
        // R: coerce.c:asRbool
        let l = crate::builtins::summary::as_logical2(&ans, true, call)?;
        if l == NA_LOGICAL {
            return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
        }
        Ok(l != 0)
    }

    /// The method for a class vector: generic or group method, which, the
    /// method's name and the class's position (`class.len()` for none).
    /// The flag says whether it is the group method.
    // R: eval.c:findmethod
    #[allow(clippy::type_complexity)]
    fn findmethod(
        &mut self,
        class: &Value,
        group: &str,
        generic: &str,
        env: &Env,
        scope: &mut LookupScope,
    ) -> RResult<(Option<Value>, bool, Option<Sym>, usize)> {
        let base = self.base.clone();
        let Value::Str(cv) = class else { return Ok((None, false, None, 0)) };
        for (i, c) in cv.data.iter().enumerate() {
            let c = class_elt(c);
            let meth = Sym::dotted(generic, c);
            if let Some(f) = self.lookup_method_in(meth, env, &base, scope)? {
                return Ok((Some(f), false, Some(meth), i));
            }
            let meth = Sym::dotted(group, c);
            if let Some(f) = self.lookup_method_in(meth, env, &base, scope)? {
                return Ok((Some(f), true, Some(meth), i));
            }
        }
        Ok((None, false, None, cv.data.len()))
    }
}

pub fn is_namespace(e: &Env) -> bool {
    e.namespace_name().is_some()
}
