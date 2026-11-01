//! S3 dispatch: `UseMethod`, internal generics and group generics
//! (GNU R's objects.c `usemethod` and eval.c `DispatchOrEval`/`DispatchGroup`).

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::{Context, Interp};
use crate::sym::{Sym, s};
use crate::value::*;

/// R's `R_data_class2`: the class vector used for S3 dispatch, including
/// implicit classes.
pub fn dispatch_class(v: &Value) -> Vec<crate::value::RString> {
    if let Some(Value::Str(c)) = v.get_attr(s::CLASS) {
        if !c.data.is_empty() {
            return c.data.iter().map(|x| x.clone().unwrap_or_else(|| crate::value::RString::from("NA"))).collect();
        }
    }
    let ndim = match v.get_attr(s::DIM) {
        Some(d) => d.len(),
        None => 0,
    };
    let mut out: Vec<&str> = Vec::new();
    match ndim {
        0 => {}
        2 => out.extend(["matrix", "array"]),
        _ => out.push("array"),
    }
    match v {
        Value::Int(_) => out.extend(["integer", "numeric"]),
        Value::Real(_) => out.extend(["double", "numeric"]),
        Value::Lang(l) => out.push(lang_class(l)),
        other => out.push(implicit_type_class(other)),
    }
    out.into_iter().map(crate::value::RString::from).collect()
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
        #[cfg(feature = "bytecode-fidelity")]
        Value::Bytecode(_) => "bytecode",
        Value::Dots(_) => "...",
        Value::ExtPtr(_) => "externalptr",
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
    pub generic: String,
    pub group: String,
    pub classes: Vec<crate::value::RString>,
    pub method: Value,
    pub call_env: Env,
    pub def_env: Env,
}

impl Interp {
    /// `topenv(envir)`: first enclosing global, base, package or namespace
    /// environment, or one that defines `.packageName`.
    // R: envir.c:topenv
    pub fn topenv(&self, env: &Env) -> Env {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if e.ptr_eq(&self.global)
                || e.ptr_eq(&self.base)
                || e.ptr_eq(&self.base_ns)
                || e.package_env_name().is_some()
                || is_namespace(&e)
                || e.has_local(crate::sym::s::DOT_PACKAGE_NAME)
            {
                return e;
            }
            if e.ptr_eq(&self.empty) {
                break;
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
        let top = self.topenv(callenv);
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
        let klass = if obj.is_s4() { self.data_class2(obj)? } else { dispatch_class(obj) };
        for (i, c) in klass.iter().enumerate() {
            let method = Sym::dotted(generic, c);
            if let Some(f) = self.lookup_method(method, callenv, defenv)? {
                if &*method.name() == "sort.list" {
                    continue;
                }
                let dot_class = if i > 0 {
                    let mut v = Value::strs(klass[i..].iter().map(|x| Some(x.clone())).collect());
                    v.set_attr_raw(s::PREVIOUS, Value::strs(klass.iter().map(|x| Some(x.clone())).collect()));
                    v
                } else {
                    Value::strs(klass.iter().map(|x| Some(x.clone())).collect())
                };
                let info = DispatchInfo {
                    generic: generic.to_string(),
                    group: String::new(),
                    classes: vec![],
                    method: Value::Sym(method),
                    call_env: callenv.clone(),
                    def_env: defenv.clone(),
                };
                return self.dispatch_method(&f, dot_class, &info, call, promargs, rho).map(Some);
            }
        }
        let method = Sym::dotted(generic, "default");
        if let Some(f) = self.lookup_method(method, callenv, defenv)? {
            let info = DispatchInfo {
                generic: generic.to_string(),
                group: String::new(),
                classes: vec![],
                method: Value::Sym(method),
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
            (s::DOT_GENERIC, Value::str(&info.generic)),
            (s::DOT_CLASS, dot_class),
            (s::DOT_METHOD, Value::strs(vec![Some(info.method.as_sym().unwrap().name().into())])),
            (s::DOT_GENERIC_CALL_ENV, Value::Env(info.call_env.clone())),
            (s::DOT_GENERIC_DEF_ENV, Value::Env(info.def_env.clone())),
            (s::DOT_GROUP, Value::str(&info.group)),
        ];
        let newcall = match call {
            Value::Lang(l) => {
                let mut nl = (**l).clone();
                nl.fun = info.method.clone();
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
        let rho1 = Env::new(Some(env.clone()));
        self.frames.push(Context {
            call: call.clone(),
            function: Value::Null,
            cloenv: rho1.clone(),
            sysparent: env.clone(),
            promargs: pargs.clone(),
            srcref: self.srcref.clone(),
            on_exit: vec![],
            handlers: self.handlers.clone(),
            restarts: self.restarts.clone(),
            generic: false,
            builtin: true,
        });
        let base = self.base.clone();
        let r = self.usemethod(generic, obj, call, pargs, &rho1, env, &base);
        let ctx = self.frames.pop().unwrap();
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
        let class_of = |v: &Value| -> Vec<crate::value::RString> {
            match v.get_attr(s::CLASS) {
                Some(Value::Str(c)) => {
                    c.data.iter().map(|x| x.clone().unwrap_or_else(|| crate::value::RString::from("NA"))).collect()
                }
                _ => vec![],
            }
        };
        let lclass = class_of(&args[0].value);
        let rclass = if nargs == 2 { class_of(&args[1].value) } else { vec![] };
        let (lsxp, lgr, lmeth, lwhich) = self.findmethod(&lclass, group, generic, env)?;
        let (rsxp, rgr, rmeth, rwhich) =
            if nargs == 2 { self.findmethod(&rclass, group, generic, env)? } else { (None, String::new(), None, 0) };
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
                } else if !crate::builtins::identical::identical(l, r) {
                    let msg = format!("Incompatible methods (\"{}\", \"{}\") for \"{}\"", lname, rname, generic);
                    self.warningcall(call, msg)?;
                    return Ok(None);
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
        let dispatch_class_name = lclass[lwhich].clone();
        let m: Vec<RStr> = args[..nargs]
            .iter()
            .map(|a| {
                let t = class_of(&a.value);
                if t.iter().any(|c| *c == dispatch_class_name) {
                    Some(lmeth.name().into())
                } else {
                    Some(crate::value::RString::from(""))
                }
            })
            .collect();
        let vars = vec![
            (s::DOT_GENERIC, Value::str(generic)),
            (s::DOT_CLASS, Value::strs(lclass[lwhich..].iter().map(|c| Some(c.clone())).collect())),
            (s::DOT_METHOD, Value::strs(m)),
            (s::DOT_GENERIC_CALL_ENV, Value::Env(env.clone())),
            (s::DOT_GENERIC_DEF_ENV, Value::Env(self.base.clone())),
            (s::DOT_GROUP, Value::str(&lgr)),
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

    #[allow(clippy::type_complexity)]
    fn findmethod(
        &mut self,
        class: &[crate::value::RString],
        group: &str,
        generic: &str,
        env: &Env,
    ) -> RResult<(Option<Value>, String, Option<Sym>, usize)> {
        let base = self.base.clone();
        for (i, c) in class.iter().enumerate() {
            let meth = Sym::dotted(generic, c);
            if let Some(f) = self.lookup_method(meth, env, &base)? {
                return Ok((Some(f), String::new(), Some(meth), i));
            }
            let meth = Sym::new(&format!("{}.{}", group, c));
            if let Some(f) = self.lookup_method(meth, env, &base)? {
                return Ok((Some(f), group.to_string(), Some(meth), i));
            }
        }
        Ok((None, String::new(), None, class.len()))
    }
}

pub fn is_namespace(e: &Env) -> bool {
    e.namespace_name().is_some()
}
