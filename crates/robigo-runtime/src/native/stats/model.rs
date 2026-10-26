//! Model formulae: `C_termsform`, `C_modelframe`, `C_modelmatrix` and
//! `C_updateform` (port of GNU R's src/library/stats/src/model.c).
//!
//! Terms are bit strings over the model variables, coded as in R: bit
//! `k` (1-based) of a term lives in word `(k - 1) / 32` at position
//! `(-k) mod 32`, so the term order produced by sorting matches R's.

use std::rc::Rc;

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

const WORDSIZE: usize = 32;

type Term = Vec<u32>;

// ----------------------------------------------------------------------
// Expression helpers

/// The call's function name, if it is a symbol.
fn head_name(e: &Value) -> Option<Rc<str>> {
    match e {
        Value::Lang(l) => l.fun.as_sym().map(|s| s.name()),
        _ => None,
    }
}

fn is_call_to(e: &Value, name: &str) -> bool {
    head_name(e).is_some_and(|n| &*n == name)
}

/// R's `length()` of a call: function plus arguments.
fn lang_len(e: &Value) -> usize {
    match e {
        Value::Lang(l) => 1 + l.args.len(),
        _ => 0,
    }
}

/// CADR / CADDR of a call.
fn arg_n(e: &Value, k: usize) -> Value {
    match e {
        Value::Lang(l) => l.args.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn is_numeric(x: &Value) -> bool {
    match x {
        Value::Int(_) => !x.inherits("factor"),
        Value::Lgl(_) | Value::Real(_) => true,
        _ => false,
    }
}

fn as_real(x: &Value) -> f64 {
    crate::coerce::as_real(x)
}

fn is_zero_one(x: &Value) -> bool {
    is_numeric(x) && (as_real(x) == 0.0 || as_real(x) == 1.0)
}

fn is_zero(x: &Value) -> bool {
    is_numeric(x) && as_real(x) == 0.0
}

fn is_one(x: &Value) -> bool {
    is_numeric(x) && as_real(x) == 1.0
}

/// Structural equality of model variables (lisp EQUAL).
// R: model.c:MatchVar
fn match_var(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        (Value::Lang(x), Value::Lang(y)) => {
            match_var(&x.fun, &y.fun)
                && x.args.len() == y.args.len()
                && x.args.iter().zip(y.args.iter()).all(|(p, q)| p.tag == q.tag && match_var(&p.value, &q.value))
        }
        (Value::Pairlist(x), Value::Pairlist(y)) => {
            x.items.len() == y.items.len()
                && x.items.iter().zip(y.items.iter()).all(|(p, q)| p.tag == q.tag && match_var(&p.value, &q.value))
        }
        (Value::Sym(x), Value::Sym(y)) => x == y,
        _ if is_numeric(a) && is_numeric(b) => as_real(a) == as_real(b),
        (Value::Str(x), Value::Str(y)) => x.data.first() == y.data.first(),
        _ => false,
    }
}

fn deparse1line(v: &Value) -> String {
    crate::deparse::deparse_one_line(v)
}

// ----------------------------------------------------------------------
// termsform

struct TermsState {
    intercept: bool,
    parity: bool,
    response: bool,
    nwords: usize,
    /// The model variables (R's `varlist` without its `list` head).
    varlist: Vec<Value>,
    framenames: Option<Vec<String>>,
    have_dot: bool,
}

fn dot() -> Sym {
    Sym::new(".")
}

impl TermsState {
    // R: model.c:InstallVar
    fn install_var(&mut self, var: &Value) -> RResult<usize> {
        if !matches!(var, Value::Sym(_) | Value::Lang(_)) && !is_zero_one(var) {
            return Err(Flow::error("invalid term in model formula"));
        }
        if let Some(k) = self.varlist.iter().position(|v| match_var(var, v)) {
            return Ok(k + 1);
        }
        self.varlist.push(var.clone());
        Ok(self.varlist.len())
    }

    // R: model.c:CheckRHS
    fn check_rhs(&mut self, v: &Value) {
        match v {
            Value::Lang(l) => {
                self.check_rhs(&l.fun);
                for a in &l.args {
                    self.check_rhs(&a.value);
                }
            }
            Value::Pairlist(p) => {
                for a in &p.items {
                    self.check_rhs(&a.value);
                }
            }
            Value::Sym(s) => {
                if let Some(f) = &mut self.framenames {
                    let name = s.name();
                    // R removes each match as it scans the (shrinking) names.
                    let mut i = 0;
                    while i < f.len() {
                        if *f[i] == *name {
                            f.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // R: model.c:ExtractVars
    fn extract_vars(&mut self, formula: &Value) -> RResult<()> {
        if formula.is_null() || is_zero_one(formula) {
            return Ok(());
        }
        if let Value::Sym(s) = formula {
            if *s == dot() {
                self.have_dot = true;
            }
            if self.have_dot && self.framenames.is_some() {
                let names = self.framenames.clone().unwrap();
                let first = self.varlist.first().cloned().unwrap_or(Value::Null);
                for n in names {
                    let v = Value::Sym(Sym::new(&n));
                    if !match_var(&v, &first) {
                        self.install_var(&v)?;
                    }
                }
            } else {
                self.install_var(formula)?;
            }
            return Ok(());
        }
        if let Value::Lang(_) = formula {
            let name = head_name(formula);
            let len = lang_len(formula);
            match name.as_deref() {
                Some("~") => {
                    if self.response {
                        return Err(Flow::error("invalid model formula"));
                    }
                    if len < 3 {
                        self.response = false;
                        self.extract_vars(&arg_n(formula, 0))?;
                    } else {
                        self.response = true;
                        self.install_var(&arg_n(formula, 0))?;
                        self.extract_vars(&arg_n(formula, 1))?;
                    }
                }
                Some("+") => {
                    if len > 1 {
                        self.extract_vars(&arg_n(formula, 0))?;
                    }
                    if len > 2 {
                        self.extract_vars(&arg_n(formula, 1))?;
                    }
                }
                Some(":") | Some("*") | Some("%in%") | Some("/") => {
                    self.extract_vars(&arg_n(formula, 0))?;
                    self.extract_vars(&arg_n(formula, 1))?;
                }
                Some("^") => {
                    if !is_numeric(&arg_n(formula, 1)) {
                        return Err(Flow::error("invalid power in formula"));
                    }
                    self.extract_vars(&arg_n(formula, 0))?;
                }
                Some("-") => {
                    if len == 2 {
                        self.extract_vars(&arg_n(formula, 0))?;
                    } else {
                        self.extract_vars(&arg_n(formula, 0))?;
                        self.extract_vars(&arg_n(formula, 1))?;
                    }
                }
                Some("(") => self.extract_vars(&arg_n(formula, 0))?,
                _ => {
                    self.install_var(formula)?;
                }
            }
            return Ok(());
        }
        Err(Flow::error("invalid model formula in ExtractVars"))
    }

    fn alloc_term(&self) -> Term {
        vec![0; self.nwords]
    }

    // R: model.c:AllocTermSetBit1
    fn alloc_term_set_bit1(&mut self, var: &Value) -> RResult<Term> {
        let which = self.install_var(var)?;
        if self.nwords < (which - 1) / WORDSIZE + 1 {
            return Err(Flow::error(format!(
                "AllocT..Bit1({}): Need to increment nwords to {}. Should not happen!\n",
                deparse1line(var),
                self.nwords + 1
            )));
        }
        let mut t = self.alloc_term();
        set_bit(&mut t, which, true);
        Ok(t)
    }

    // R: model.c:EncodeVars
    fn encode_vars(&mut self, formula: &Value) -> RResult<Vec<Term>> {
        if formula.is_null() {
            return Ok(vec![]);
        }
        if is_one(formula) {
            self.intercept = self.parity;
            return Ok(vec![]);
        }
        if is_zero(formula) {
            self.intercept = !self.parity;
            return Ok(vec![]);
        }
        if let Value::Sym(s) = formula {
            if *s == dot() && self.framenames.is_some() {
                let names = self.framenames.clone().unwrap();
                let mut r = Vec::new();
                for (i, c) in names.iter().enumerate() {
                    if names[..i].contains(c) {
                        return Err(Flow::error(format!("duplicated name '{}' in data frame using '.'", c)));
                    }
                    r.push(self.alloc_term_set_bit1(&Value::Sym(Sym::new(c)))?);
                }
                return Ok(r);
            }
            return Ok(vec![self.alloc_term_set_bit1(formula)?]);
        }
        if let Value::Lang(_) = formula {
            let (a, b) = (arg_n(formula, 0), arg_n(formula, 1));
            return match head_name(formula).as_deref() {
                Some("~") => {
                    if lang_len(formula) < 3 {
                        self.encode_vars(&a)
                    } else {
                        self.encode_vars(&b)
                    }
                }
                Some("+") => {
                    if lang_len(formula) == 2 {
                        self.encode_vars(&a)
                    } else {
                        let mut l = self.encode_vars(&a)?;
                        l.extend(self.encode_vars(&b)?);
                        Ok(trim_repeats(l))
                    }
                }
                Some(":") => {
                    let l = self.encode_vars(&a)?;
                    let r = self.encode_vars(&b)?;
                    Ok(trim_repeats(or_product(&l, &r)))
                }
                Some("*") => {
                    let mut l = self.encode_vars(&a)?;
                    let mut r = self.encode_vars(&b)?;
                    let t = or_product(&l, &r);
                    r.extend(t);
                    l.extend(r);
                    Ok(trim_repeats(l))
                }
                Some("%in%") => {
                    let mut l = self.encode_vars(&a)?;
                    let r = self.encode_vars(&b)?;
                    let all = or_all(&r, self.nwords);
                    for t in l.iter_mut() {
                        for (w, x) in t.iter_mut().zip(all.iter()) {
                            *w |= *x;
                        }
                    }
                    Ok(trim_repeats(l))
                }
                Some("/") => {
                    let mut l = self.encode_vars(&a)?;
                    let mut r = self.encode_vars(&b)?;
                    let all = or_all(&l, self.nwords);
                    for t in r.iter_mut() {
                        for (w, x) in t.iter_mut().zip(all.iter()) {
                            *w |= *x;
                        }
                    }
                    l.extend(r);
                    Ok(trim_repeats(l))
                }
                Some("^") => {
                    // R: model.c:PowerTerms
                    let ip = crate::coerce::as_integer(&b);
                    if ip == NA_INTEGER || ip <= 1 {
                        return Err(Flow::error("invalid power in formula"));
                    }
                    let l = self.encode_vars(&a)?;
                    let mut right = l.clone();
                    for _ in 1..ip {
                        right = trim_repeats(or_product(&l, &right));
                    }
                    Ok(right)
                }
                Some("-") => {
                    if lang_len(formula) == 2 {
                        self.delete_terms(&Value::Null, &a)
                    } else {
                        self.delete_terms(&a, &b)
                    }
                }
                Some("(") => self.encode_vars(&a),
                _ => Ok(vec![self.alloc_term_set_bit1(formula)?]),
            };
        }
        Err(Flow::error("invalid model formula in EncodeVars"))
    }

    // R: model.c:DeleteTerms
    fn delete_terms(&mut self, left: &Value, right: &Value) -> RResult<Vec<Term>> {
        let mut l = self.encode_vars(left)?;
        self.parity = !self.parity;
        let r = self.encode_vars(right)?;
        self.parity = !self.parity;
        for t in &r {
            // R: model.c:StripTerm
            if term_zero(t) {
                self.intercept = false;
            }
            l.retain(|x| x != t);
        }
        Ok(l)
    }
}

fn bit_pos(which: usize) -> (usize, u32) {
    let word = (which - 1) / WORDSIZE;
    let offset = (-(which as i64)).rem_euclid(WORDSIZE as i64) as u32;
    (word, offset)
}

fn set_bit(t: &mut Term, which: usize, on: bool) {
    let (w, o) = bit_pos(which);
    if on {
        t[w] |= 1 << o;
    } else {
        t[w] &= !(1 << o);
    }
}

fn get_bit(t: &Term, which: usize) -> bool {
    let (w, o) = bit_pos(which);
    (t[w] >> o) & 1 == 1
}

fn term_zero(t: &Term) -> bool {
    t.iter().all(|&w| w == 0)
}

fn bit_count(t: &Term, nvar: usize) -> usize {
    (1..=nvar).filter(|&i| get_bit(t, i)).count()
}

fn or_product(l: &[Term], r: &[Term]) -> Vec<Term> {
    let mut out = Vec::with_capacity(l.len() * r.len());
    for a in l {
        for b in r {
            out.push(a.iter().zip(b.iter()).map(|(x, y)| x | y).collect());
        }
    }
    out
}

fn or_all(l: &[Term], nwords: usize) -> Term {
    let mut t = vec![0; nwords];
    for a in l {
        for (w, x) in t.iter_mut().zip(a.iter()) {
            *w |= *x;
        }
    }
    t
}

/// Remove zero terms and repeated terms.
// R: model.c:TrimRepeats
fn trim_repeats(list: Vec<Term>) -> Vec<Term> {
    let mut out: Vec<Term> = Vec::with_capacity(list.len());
    for t in list {
        if !term_zero(&t) && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Whether variable `which` of `this` is coded by contrasts (1) or
/// dummy variables (2).
// R: model.c:TermCode
fn term_code(preceding: &[Term], this: &Term, which: usize) -> i32 {
    let mut term = this.clone();
    set_bit(&mut term, which, false);
    if term_zero(&term) {
        return 1;
    }
    for t in preceding {
        if term.iter().zip(t.iter()).all(|(a, c)| a & !c == 0) {
            return 1;
        }
    }
    2
}

/// Replace `.` in a formula by `value`.
// R: model.c:ExpandDots
fn expand_dots(object: &Value, value: &Value) -> RResult<Value> {
    let bad = || Flow::error("invalid formula in 'update'");
    match object {
        Value::Sym(s) => Ok(if *s == dot() { value.clone() } else { object.clone() }),
        Value::Lang(l) => {
            let op = head_name(value);
            let opis = |names: &[&str]| op.as_deref().is_some_and(|o| names.contains(&o));
            let is_dot = |v: &Value| matches!(v, Value::Sym(s) if *s == dot());
            let paren = |v: Value| Value::lang(Value::sym("("), vec![Arg::pos(v)]);
            let mut args = l.args.clone();
            let n = lang_len(object);
            let mut expand = |k: usize, wrap: bool| -> RResult<()> {
                let e = expand_dots(&args[k].value, value)?;
                args[k].value = if wrap && is_dot(&args[k].value) { paren(e) } else { e };
                Ok(())
            };
            match head_name(object).as_deref() {
                Some("+") => match n {
                    2 => expand(0, false)?,
                    3 => {
                        expand(0, false)?;
                        expand(1, false)?;
                    }
                    _ => return Err(bad()),
                },
                Some("-") => match n {
                    2 => expand(0, opis(&["+", "-"]))?,
                    3 => {
                        expand(0, opis(&["+", "-"]))?;
                        expand(1, opis(&["+", "-"]))?;
                    }
                    _ => return Err(bad()),
                },
                Some("*") | Some("/") => {
                    if n != 3 {
                        return Err(bad());
                    }
                    expand(0, opis(&["+", "-"]))?;
                    expand(1, opis(&["+", "-"]))?;
                }
                Some(":") => {
                    if n != 3 {
                        return Err(bad());
                    }
                    expand(0, opis(&["+", "-", "*", "/"]))?;
                    expand(1, opis(&["+", "-"]))?;
                }
                Some("^") => {
                    if n != 3 {
                        return Err(bad());
                    }
                    expand(0, opis(&["+", "-", "*", "/", ":"]))?;
                    expand(1, opis(&["+", "-"]))?;
                }
                _ => {
                    let nargs = l.args.len();
                    for k in 0..nargs {
                        expand(k, false)?;
                    }
                    let fun = expand_dots(&l.fun, value)?;
                    return Ok(Value::lang(fun, args));
                }
            }
            Ok(Value::lang(l.fun.clone(), args))
        }
        _ => Ok(object.clone()),
    }
}

fn str_vec(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Str(s) => Some(s.data.iter().map(|e| e.as_deref().unwrap_or("NA").to_string()).collect()),
        _ => None,
    }
}

// R: model.c:termsform
pub fn termsform(i: &mut Interp, args: &[Arg]) -> RResult<Value> {
    let x = &args[0].value;
    if !is_call_to(x, "~") || (lang_len(x) != 2 && lang_len(x) != 3) {
        return Err(Flow::error("argument is not a valid model"));
    }
    let specials = args.get(1).map(|a| a.value.clone()).unwrap_or(Value::Null);
    if !specials.is_empty() && !matches!(specials, Value::Str(_)) {
        return Err(Flow::error("'specials' must be NULL or a character vector"));
    }
    let data = args.get(2).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let framenames = if data.is_null() || matches!(data, Value::Env(_)) {
        None
    } else if data.inherits("data.frame") {
        Some(data.names().and_then(|n| str_vec(&n)).unwrap_or_default())
    } else {
        return Err(Flow::error("'data' argument is of the wrong type"));
    };
    let mut st = TermsState {
        intercept: true,
        parity: true,
        response: false,
        nwords: 0,
        varlist: Vec::new(),
        framenames,
        have_dot: false,
    };
    let mut had_frame_names = false;
    if let Some(f) = &st.framenames {
        if !f.is_empty() {
            had_frame_names = true;
        }
        if lang_len(x) == 3 {
            st.check_rhs(&arg_n(x, 0));
        }
    }
    let keep_order = crate::coerce::as_logical(&args.get(3).map(|a| a.value.clone()).unwrap_or(Value::Null));
    let keep_order = keep_order != NA_LOGICAL && keep_order != 0;
    let allow_dot = crate::coerce::as_logical(&args.get(4).map(|a| a.value.clone()).unwrap_or(Value::Null));
    let allow_dot = allow_dot != NA_LOGICAL && allow_dot != 0;

    // Step 1: the variables
    st.extract_vars(x)?;
    let variables = Value::lang(
        Value::sym("list"),
        st.varlist.iter().map(|v| Arg::pos(v.clone())).collect(),
    );
    let mut nvar = st.varlist.len();
    st.nwords = nvar / WORDSIZE + 1;

    // Step 2: encode and expand the terms
    let mut formula = st.encode_vars(x)?;
    if st.varlist.len() != nvar {
        i.warning(format!(
            "'varlist' has changed (from nvar={}) to new {} after EncodeVars() -- should no longer happen!",
            nvar,
            st.varlist.len()
        ))?;
        nvar = st.varlist.len();
    }
    let response = st.response as usize;

    // Step 2a: variable names
    let varnames: Vec<String> = st.varlist.iter().map(deparse1line).collect();

    let mut attrs: Vec<(Sym, Value)> = vec![(Sym::new("variables"), variables)];

    // Step 2b: offsets
    let is_offset = |k: usize| varnames[k].starts_with("offset(");
    let offsets: Vec<i32> = (response..nvar).filter(|&l| is_offset(l)).map(|l| l as i32 + 1).collect();
    if !offsets.is_empty() {
        attrs.push((Sym::new("offset"), Value::ints(offsets)));
        formula.retain(|t| !(1..=nvar).any(|k| get_bit(t, k) && is_offset(k - 1)));
    }
    let nterm = formula.len();

    // Step 3: order terms by their number of variables
    let counts: Vec<usize> = formula.iter().map(|t| bit_count(t, nvar)).collect();
    let bitmax = counts.iter().copied().max().unwrap_or(0);
    let ord: Vec<i32> = if keep_order {
        counts.iter().map(|&c| c as i32).collect()
    } else {
        let mut sorted = Vec::with_capacity(nterm);
        let mut ord = Vec::with_capacity(nterm);
        for c in 0..=bitmax {
            for n in 0..nterm {
                if counts[n] == c {
                    sorted.push(formula[n].clone());
                    ord.push(c as i32);
                }
            }
        }
        formula = sorted;
        ord
    };

    // Step 4: the factor pattern
    let mut pattern = vec![0i32; nterm * nvar];
    for (n, t) in formula.iter().enumerate() {
        for k in 1..=nvar {
            if get_bit(t, k) {
                pattern[k - 1 + n * nvar] = term_code(&formula[..n], t, k);
            }
        }
    }

    // Step 5: term labels
    let termlabs: Vec<String> = formula
        .iter()
        .map(|t| (1..=nvar).filter(|&k| get_bit(t, k)).map(|k| varnames[k - 1].as_str()).collect::<Vec<_>>().join(":"))
        .collect();
    let factors = if nterm > 0 {
        let mut m = Value::ints(pattern);
        m.set_attr_raw(s::DIM, Value::ints(vec![nvar as i32, nterm as i32]));
        m.set_attr_raw(
            s::DIMNAMES,
            Value::list(vec![Value::strs_from(&varnames), Value::strs_from(&termlabs)]),
        );
        m
    } else {
        Value::ints(vec![])
    };
    attrs.push((Sym::new("factors"), factors));
    attrs.push((Sym::new("term.labels"), Value::strs_from(&termlabs)));

    if let Value::Str(sp) = &specials {
        let items = sp
            .data
            .iter()
            .map(|ss| {
                let ss = ss.as_deref().unwrap_or("NA");
                let hits: Vec<i32> = (0..nvar)
                    .filter(|&l| varnames[l].starts_with(ss) && varnames[l][ss.len()..].starts_with('('))
                    .map(|l| l as i32 + 1)
                    .collect();
                Arg::new(Some(Sym::new(ss)), if hits.is_empty() { Value::Null } else { Value::ints(hits) })
            })
            .collect();
        attrs.push((Sym::new("specials"), Value::pairlist(items)));
    }

    // Step 6: substitute the data frame's variables for `.`
    let mut ans = x.clone();
    if st.have_dot {
        let names = st.framenames.clone().unwrap_or_default();
        if !names.is_empty() {
            let mut rhs = Value::sym(&names[0]);
            for n in &names[1..] {
                rhs = Value::lang(Value::sym("+"), vec![Arg::pos(rhs), Arg::pos(Value::sym(n))]);
            }
            let Value::Lang(l) = &ans else { unreachable!() };
            let mut args = l.args.clone();
            let k = if lang_len(&ans) == 3 { 1 } else { 0 };
            args[k].value = expand_dots(&args[k].value, &rhs)?;
            ans = Value::lang(l.fun.clone(), args);
        } else if !allow_dot && !had_frame_names {
            return Err(Flow::error("'.' in formula and no 'data' argument"));
        }
    }

    attrs.push((Sym::new("order"), Value::ints(ord)));
    attrs.push((Sym::new("intercept"), Value::int(st.intercept as i32)));
    attrs.push((Sym::new("response"), Value::int(st.response as i32)));
    attrs.push((s::CLASS, Value::str("terms")));
    ans.set_attrs(Some(Box::new(Attrs(attrs))));
    Ok(ans)
}

// R: model.c:updateform
pub fn updateform(old: &Value, new: &Value) -> RResult<Value> {
    if !matches!(old, Value::Lang(_)) || (!matches!(new, Value::Lang(_)) && !is_call_to(old, "~")) || !is_call_to(new, "~") {
        return Err(Flow::error("formula expected"));
    }
    let Value::Lang(nl) = new else { unreachable!() };
    let mut args = nl.args.clone();
    if lang_len(old) == 3 {
        let lhs = arg_n(old, 0);
        let rhs = arg_n(old, 1);
        if args.len() == 1 {
            args.insert(0, Arg::pos(lhs.clone()));
        }
        args[0].value = expand_dots(&args[0].value, &lhs)?;
        args[1].value = expand_dots(&args[1].value, &rhs)?;
    } else {
        let rhs = arg_n(old, 0);
        let k = if args.len() == 2 { 1 } else { 0 };
        args[k].value = expand_dots(&args[k].value, &rhs)?;
    }
    let mut ans = Value::lang(nl.fun.clone(), args);
    if let Some(e) = old.get_attr(Sym::new(".Environment")) {
        ans.set_attr_raw(Sym::new(".Environment"), e);
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// modelframe

/// R's `nrows()`.
fn nrows(v: &Value) -> usize {
    if v.inherits("data.frame") {
        return match v {
            Value::List(l) => l.data.first().map(nrows).unwrap_or(0),
            _ => 0,
        };
    }
    match v.get_attr(s::DIM) {
        Some(Value::Int(d)) if !d.data.is_empty() => d.data[0] as usize,
        _ => v.len(),
    }
}

/// R's `ncols()`.
fn ncols(v: &Value) -> usize {
    match v.get_attr(s::DIM) {
        Some(Value::Int(d)) if d.data.len() >= 2 => d.data[1] as usize,
        _ => 1,
    }
}

fn type_name(v: &Value) -> String {
    v.type_name().to_string()
}

// R: model.c:modelframe
pub fn modelframe(i: &mut Interp, args: &[Arg], rho: &Env) -> RResult<Value> {
    let a = |k: usize| args.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let (terms, row_names, variables, varnames, dots, dotnames, subset, na_action) =
        (a(0), a(1), a(2), a(3), a(4), a(5), a(6), a(7));
    let Value::List(vars) = &variables else { return Err(Flow::error("invalid variables")) };
    let Some(vnames) = str_vec(&varnames) else { return Err(Flow::error("invalid variable names")) };
    if vars.data.len() != vnames.len() {
        return Err(Flow::error("number of variables != number of variable names"));
    }
    let dots_v: Vec<Value> = match &dots {
        Value::List(d) => d.data.clone(),
        Value::Null => vec![],
        _ => return Err(Flow::error("invalid extra variables")),
    };
    if dots_v.len() != dotnames.len() {
        return Err(Flow::error("number of variables != number of variable names"));
    }
    let dnames = if dots_v.is_empty() {
        vec![]
    } else {
        str_vec(&dotnames).ok_or_else(|| Flow::error("invalid extra variable names"))?
    };
    let mut data: Vec<Value> = vars.data.clone();
    let mut names: Vec<String> = vnames;
    for (d, n) in dots_v.iter().zip(dnames.iter()) {
        if d.is_null() {
            continue;
        }
        if n.len() + 3 > 256 {
            return Err(Flow::error(format!("overlong names in '{}'", n)));
        }
        data.push(d.clone());
        names.push(format!("({})", n));
    }
    let nc = data.len();
    let nr = if nc > 0 {
        let nr = nrows(&data[0]);
        for (k, v) in data.iter().enumerate() {
            if !matches!(
                v,
                Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_)
            ) {
                return Err(Flow::error(format!("invalid type ({}) for variable '{}'", type_name(v), names[k])));
            }
            if nrows(v) != nr {
                return Err(Flow::error(format!("variable lengths differ (found for '{}')", names[k])));
            }
        }
        nr
    } else {
        row_names.len()
    };
    let mut df = Value::list(data);
    df.set_attr_raw(s::NAMES, Value::strs_from(&names));
    df.set_attr_raw(s::CLASS, Value::str("data.frame"));
    if row_names.len() == nr {
        df.set_attr_raw(Sym::new("row.names"), row_names);
    } else {
        df.set_attr_raw(Sym::new("row.names"), Value::ints(vec![NA_INTEGER, nr as i32]));
    }
    if !subset.is_null() {
        let call = Value::lang(
            Value::sym("[.data.frame"),
            vec![
                Arg::pos(quote(df)),
                Arg::pos(quote(subset)),
                Arg::pos(Value::missing_arg()),
                Arg::pos(Value::lgl(false)),
            ],
        );
        df = i.eval(&call, rho)?;
    }
    let ans = if !na_action.is_null() {
        df.set_attr_raw(Sym::new("terms"), terms);
        let f = match &na_action {
            Value::Str(s) if !s.data.is_empty() => Value::sym(s.data[0].as_deref().unwrap_or("NA")),
            v => v.clone(),
        };
        let call = Value::lang(f, vec![Arg::pos(quote(df.clone()))]);
        let mut ans = i.eval(&call, rho)?;
        let (Value::List(al), Value::List(dl)) = (&ans, &df) else {
            return Err(Flow::error("invalid result from na.action"));
        };
        if al.data.len() != dl.data.len() {
            return Err(Flow::error("invalid result from na.action"));
        }
        // Transfer all but tsp and dim attributes, possibly lost by
        // subsetting in na.action.
        let fixes: Vec<(usize, Value)> = (0..al.data.len())
            .filter(|&k| !al.data[k].ptr_eq(&dl.data[k]))
            .map(|k| {
                let mut v = al.data[k].clone();
                crate::attrib::copy_most_attrib_no_ts(&dl.data[k], &mut v);
                (k, v)
            })
            .collect();
        if let Value::List(al) = &mut ans {
            let al = Rc::make_mut(al);
            for (k, v) in fixes {
                al.data[k] = v;
            }
        }
        ans
    } else {
        df
    };
    Ok(ans)
}

/// A value as an argument of a constructed call.
fn quote(v: Value) -> Value {
    match v {
        Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => Value::lang(Value::sym("quote"), vec![Arg::pos(v)]),
        v => v,
    }
}

// ----------------------------------------------------------------------
// modelmatrix

fn column_names(x: &Value) -> Option<Vec<String>> {
    match x.get_attr(s::DIMNAMES) {
        Some(Value::List(l)) if l.data.len() >= 2 => str_vec(&l.data[1]),
        _ => None,
    }
}

fn is_factor(v: &Value) -> bool {
    matches!(v, Value::Int(_)) && v.inherits("factor")
}

fn nlevels(v: &Value) -> i32 {
    v.get_attr(s::LEVELS).map(|l| l.len() as i32).unwrap_or(0)
}

fn reals(v: &Value) -> Vec<f64> {
    match v {
        Value::Real(r) => r.data.clone(),
        Value::Int(r) | Value::Lgl(r) => r.data.iter().map(|&x| crate::builtins::arith::int_to_real(x)).collect(),
        _ => vec![],
    }
}

fn ints(v: &Value) -> Vec<i32> {
    match v {
        Value::Int(r) | Value::Lgl(r) => r.data.clone(),
        _ => vec![],
    }
}

// R: model.c:modelmatrix
pub fn modelmatrix(i: &mut Interp, args: &[Arg], rho: &Env) -> RResult<Value> {
    let terms = args.first().map(|a| a.value.clone()).unwrap_or(Value::Null);
    let attr_lgl = |name: &str| {
        let v = crate::coerce::as_logical(&terms.get_attr(Sym::new(name)).unwrap_or(Value::Null));
        v != NA_LOGICAL && v != 0
    };
    let intrcept = attr_lgl("intercept");
    let risponse = crate::coerce::as_logical(&terms.get_attr(Sym::new("response")).unwrap_or(Value::Null));
    let risponse = if risponse == NA_LOGICAL { 0 } else { risponse as usize };

    let factors_v = terms.get_attr(Sym::new("factors")).unwrap_or(Value::Null);
    let (n_var, nterms, mut factors) = if factors_v.is_empty() {
        (1usize, 0usize, vec![])
    } else if matches!(factors_v, Value::Int(_)) && crate::attrib::dims(&factors_v).is_some_and(|d| d.len() == 2) {
        let d = crate::attrib::dims(&factors_v).unwrap();
        (d[0] as usize, d[1] as usize, ints(&factors_v))
    } else {
        return Err(Flow::error("invalid 'terms' argument"));
    };
    let mut vnames: Vec<String> = vec![];
    if !factors_v.is_empty() {
        let dn = factors_v.get_attr(s::DIMNAMES);
        let first = match &dn {
            Some(Value::List(l)) if !l.data.is_empty() => Some(l.data[0].clone()),
            _ => None,
        };
        match first.as_ref().and_then(str_vec) {
            Some(v) => vnames = v,
            None if n_var as i64 - (intrcept as i64) > 0 || first.is_none() => {
                return Err(Flow::error("invalid 'terms' argument"));
            }
            None => {}
        }
    }

    let vars_v = args.get(1).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let Value::List(vars) = &vars_v else { return Err(Flow::error("invalid model frame")) };
    if vars.data.len() < n_var {
        return Err(Flow::error("invalid model frame"));
    }
    if vars.data.is_empty() {
        return Err(Flow::error("do not know how many cases"));
    }
    let n = nrows(&vars.data[0]);
    let rnames = crate::attrib::get_attrib(&vars_v, Sym::new("row.names"));

    let mut variable: Vec<Value> = Vec::with_capacity(n_var);
    let mut nlevs = vec![0i32; n_var];
    let mut columns = vec![0usize; n_var];
    for k in 0..n_var {
        let mut var_i = vars.data[k].clone();
        if nrows(&var_i) != n {
            return Err(Flow::error(format!("variable lengths differ (found for variable {})", k)));
        }
        if is_factor(&var_i) {
            nlevs[k] = nlevels(&var_i);
            if nlevs[k] < 1 {
                return Err(Flow::error(format!("variable {} has no levels", k + 1)));
            }
        } else if matches!(var_i, Value::Lgl(_)) {
            nlevs[k] = 2;
        } else if is_numeric(&var_i) {
            var_i = i.coerce(&var_i, VType::Real)?;
        }
        columns[k] = ncols(&var_i);
        variable.push(var_i);
    }

    // No intercept: code the first factor by dummy variables.
    if !intrcept {
        'done: for j in 0..nterms {
            for k in risponse..n_var {
                if nlevs[k] > 1 && factors[k + j * n_var] > 0 {
                    factors[k + j * n_var] = 2;
                    break 'done;
                }
            }
        }
    }

    // Contrast (1) and dummy-variable (2) matrices.
    let mut contr1: Vec<Value> = vec![Value::Null; n_var];
    let mut contr2: Vec<Value> = vec![Value::Null; n_var];
    for k in 0..n_var {
        if nlevs[k] == 0 {
            continue;
        }
        let mut code = 0;
        for j in 0..nterms {
            match factors[k + j * n_var] {
                1 => code |= 1,
                2 => code |= 2,
                _ => {}
            }
        }
        let call = |flag: bool| {
            Value::lang(Value::sym("contrasts"), vec![Arg::pos(quote(variable[k].clone())), Arg::pos(Value::lgl(flag))])
        };
        if code & 1 != 0 {
            contr1[k] = i.eval(&call(true), rho)?;
        }
        if code & 2 != 0 {
            contr2[k] = i.eval(&call(false), rho)?;
        }
    }

    // An rhs term identical to the response generates nothing.
    let mut rhs_response: Option<usize> = None;
    if risponse > 0 {
        for j in 0..nterms {
            if factors[risponse - 1 + j * n_var] != 0 {
                let k = (0..n_var).filter(|&k| factors[k + j * n_var] > 0).count();
                if k == 1 {
                    rhs_response = Some(j);
                    break;
                }
            }
        }
    }

    // Column counts.
    let mut count = vec![0usize; nterms];
    let mut dnc: f64 = if intrcept { 1.0 } else { 0.0 };
    for j in 0..nterms {
        if Some(j) == rhs_response {
            i.warning("the response appeared on the right-hand side and was dropped")?;
            count[j] = 0;
            continue;
        }
        let mut dk: f64 = 1.0;
        for k in 0..n_var {
            let f = factors[k + j * n_var];
            if f != 0 {
                if nlevs[k] != 0 {
                    match f {
                        1 => dk *= ncols(&contr1[k]) as f64,
                        2 => dk *= ncols(&contr2[k]) as f64,
                        _ => {}
                    }
                } else {
                    dk *= columns[k] as f64;
                }
            }
        }
        if dk > i32::MAX as f64 {
            return Err(Flow::error(format!("term {} would require {} columns", j + 1, robigo_syntax::cfmt::fmt_g(dk, 0, false, false))));
        }
        count[j] = dk as usize;
        dnc += dk;
    }
    if dnc > i32::MAX as f64 {
        return Err(Flow::error(format!("matrix would require {} columns", robigo_syntax::cfmt::fmt_g(dnc, 0, false, false))));
    }
    let nc = dnc as usize;

    let mut assign: Vec<i32> = Vec::with_capacity(nc);
    if intrcept {
        assign.push(0);
    }
    for j in 0..nterms {
        if count[j] == 0 {
            i.warning(format!("problem with term {} in model.matrix: no columns are assigned", j + 1))?;
        }
        for _ in 0..count[j] {
            assign.push(j as i32 + 1);
        }
    }

    // Column labels.
    let mut xnames: Vec<String> = Vec::with_capacity(nc);
    if intrcept {
        xnames.push("(Intercept)".into());
    }
    for j in 0..nterms {
        if Some(j) == rhs_response {
            continue;
        }
        for kk in 0..count[j] {
            let mut first = true;
            let mut indx = kk;
            let mut buf = String::new();
            for k in 0..n_var {
                let mut ll = factors[k + j * n_var] as usize;
                if ll == 0 {
                    continue;
                }
                let var_i = &variable[k];
                if !first {
                    buf.push(':');
                }
                first = false;
                if is_factor(var_i) || matches!(var_i, Value::Lgl(_)) {
                    let c = if ll == 1 { &contr1[k] } else { &contr2[k] };
                    let x = column_names(c);
                    ll = ncols(c);
                    buf.push_str(&vnames[k]);
                    match x {
                        None => buf.push_str(&(indx % ll + 1).to_string()),
                        Some(x) => buf.push_str(&x[indx % ll]),
                    }
                } else if matches!(var_i, Value::Cplx(_)) {
                    return Err(Flow::error("complex variables are not currently allowed in model matrices"));
                } else if is_numeric(var_i) {
                    let x = column_names(var_i);
                    ll = ncols(var_i);
                    buf.push_str(&vnames[k]);
                    if ll > 1 {
                        match x {
                            None => buf.push_str(&(indx % ll + 1).to_string()),
                            Some(x) => buf.push_str(&x[indx % ll]),
                        }
                    }
                } else {
                    return Err(Flow::error(format!(
                        "variables of type '{}' are not allowed in model matrices",
                        type_name(var_i)
                    )));
                }
                indx /= ll;
            }
            xnames.push(buf);
        }
    }

    // The design matrix.
    let mut rx = vec![0.0f64; n * nc];
    let mut jstart = intrcept as usize;
    let mut jnext = jstart;
    if intrcept {
        for v in rx.iter_mut().take(n) {
            *v = 1.0;
        }
    }
    for k in 0..nterms {
        if Some(k) == rhs_response {
            continue;
        }
        for vi in 0..n_var {
            if columns[vi] == 0 {
                continue;
            }
            let fik = factors[vi + k * n_var];
            if fik == 0 {
                continue;
            }
            let var_i = &variable[vi];
            let contrast = if fik == 1 { &contr1[vi] } else { &contr2[vi] };
            let cdata = reals(&i.coerce(contrast, VType::Real)?);
            let (nrc, ncc) = (nrows(contrast), ncols(contrast));
            let block = &mut rx[jstart * n..];
            let ncx = jnext - jstart;
            if jnext == jstart {
                if nlevs[vi] > 0 {
                    let adj = matches!(var_i, Value::Lgl(_)) as i32;
                    let v = ints(var_i);
                    // R: model.c:firstfactor
                    for j in 0..ncc {
                        for r in 0..n {
                            block[j * n + r] =
                                if v[r] == NA_INTEGER { NA_REAL } else { cdata[j * nrc + (v[r] - 1 + adj) as usize] };
                        }
                    }
                    jnext += ncc;
                } else {
                    // R: model.c:firstvar
                    let c = reals(var_i);
                    let ncv = ncols(var_i);
                    for j in 0..ncv {
                        block[j * n..j * n + n].copy_from_slice(&c[j * n..j * n + n]);
                    }
                    jnext += ncv;
                }
            } else if nlevs[vi] > 0 {
                let adj = matches!(var_i, Value::Lgl(_)) as i32;
                let v = ints(var_i);
                // R: model.c:addfactor
                for kc in (0..ncc).rev() {
                    for j in 0..ncx {
                        for r in 0..n {
                            let xv = block[j * n + r];
                            block[(kc * ncx + j) * n + r] = if v[r] == NA_INTEGER {
                                NA_REAL
                            } else {
                                cdata[kc * nrc + (v[r] - 1 + adj) as usize] * xv
                            };
                        }
                    }
                }
                jnext += ncx * (ncc - 1);
            } else {
                // R: model.c:addvar
                let c = reals(var_i);
                let ncv = ncols(var_i);
                for kc in (0..ncv).rev() {
                    for j in 0..ncx {
                        for r in 0..n {
                            let xv = block[j * n + r];
                            block[(kc * ncx + j) * n + r] = c[kc * n + r] * xv;
                        }
                    }
                }
                jnext += ncx * (ncv - 1);
            }
        }
        jstart = jnext;
    }
    let mut x = Value::reals(rx);
    x.set_attr_raw(s::DIM, Value::ints(vec![n as i32, nc as i32]));
    let rn = match rnames {
        Value::Null => Value::Null,
        v @ Value::Str(_) => v,
        v => crate::coerce::coerce_vector(&v, VType::Str, &mut Default::default())?,
    };
    x.set_attr_raw(s::DIMNAMES, Value::list(vec![rn, Value::strs_from(&xnames)]));
    x.set_attr_raw(Sym::new("assign"), Value::ints(assign));
    Ok(x)
}
