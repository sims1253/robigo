//! Printing of R values. Port of GNU R's print.c, printvector.c and
//! printarray.c.

use crate::env::Env;
use crate::error::RResult;
use crate::format::*;
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

const TAGBUFLEN: usize = 256;
const R_MIN_LBLOFF: usize = 2;

pub struct Printer {
    pub pp: PrintParams,
    pub tagbuf: String,
    pub env: Env,
    pub out: String,
    /// Extra arguments to forward to `print()` for objects.
    pub call_args: Vec<Arg>,
}

impl Interp {
    pub fn print_params(&self) -> PrintParams {
        let mut pp = PrintParams {
            digits: self.option_int("digits", 7).clamp(1, 22) as usize,
            scipen: self.option_int("scipen", 0),
            width: self.option_int("width", 80).max(10) as usize,
            ..PrintParams::default()
        };
        if let Some(Value::Str(v)) = self.get_option("OutDec") {
            let _ = v;
        }
        pp.max = self.option_int("max.print", 99999).max(0) as usize;
        pp
    }
}

pub fn print_value(interp: &mut Interp, v: &Value, env: &Env) -> RResult<()> {
    let mut p = Printer { pp: interp.print_params(), tagbuf: String::new(), env: env.clone(), out: String::new(), call_args: vec![] };
    p.value_rec(interp, v)?;
    p.flush(interp);
    Ok(())
}

impl Printer {
    pub fn new(interp: &Interp, env: &Env) -> Printer {
        Printer { pp: interp.print_params(), tagbuf: String::new(), env: env.clone(), out: String::new(), call_args: vec![] }
    }

    pub fn flush(&mut self, interp: &mut Interp) {
        if !self.out.is_empty() {
            let s = std::mem::take(&mut self.out);
            interp.write_out(&s);
        }
    }

    fn p(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn dispatch(&mut self, interp: &mut Interp, v: &Value) -> RResult<()> {
        if v.is_object() {
            self.flush(interp);
            let save = self.tagbuf.clone();
            let mask = Env::new(Some(self.env.clone()));
            let xsym = Sym::new("x");
            mask.set_local(xsym, v.clone());
            let base = interp.base.clone();
            let print = interp.find_fun(Sym::new("print"), &base)?;
            let mut args = vec![Arg::pos(Value::Sym(xsym))];
            args.extend(self.call_args.iter().cloned());
            let call = Value::lang(print, args);
            interp.eval(&call, &mask)?;
            self.tagbuf = save;
            Ok(())
        } else {
            self.value_rec(interp, v)
        }
    }

    pub fn value_rec(&mut self, interp: &mut Interp, v: &Value) -> RResult<()> {
        match v {
            Value::Null => self.p("NULL\n"),
            Value::Sym(_) => {
                let t = crate::deparse::deparse_lines(v, 60, true, crate::deparse::SIMPLEDEPARSE, None);
                self.p(&t[0]);
                self.p("\n");
            }
            Value::Builtin(b) => self.print_special(interp, b),
            Value::Expr(_) | Value::Lang(_) => {
                let lines = crate::deparse::deparse_lines(v, 60, true, crate::deparse::DEFAULTDEPARSE, None);
                for l in lines {
                    self.p(&l);
                    self.p("\n");
                }
            }
            Value::Closure(c) => {
                let lines = crate::deparse::deparse_lines(v, 60, true, crate::deparse::DEFAULTDEPARSE, None);
                for l in lines {
                    self.p(&l);
                    self.p("\n");
                }
                if !c.env.ptr_eq(&interp.global) {
                    let e = encode_environment(&c.env);
                    self.p(&e);
                    self.p("\n");
                }
            }
            Value::Env(e) => {
                let s = encode_environment(e);
                self.p(&s);
                self.p("\n");
            }
            Value::Promise(_) => self.p("<promise: 0x0>\n"),
            Value::Dots(_) => self.p("<...>\n"),
            Value::ExtPtr(_) => self.p("<pointer: 0x0>\n"),
            Value::List(_) => {
                self.generic_vector(interp, v)?;
                return Ok(());
            }
            Value::Pairlist(pl) => {
                // Print like a list.
                let names: Vec<RStr> =
                    pl.items.iter().map(|a| Some(a.tag.map(|t| t.name()).unwrap_or_else(|| "".into()))).collect();
                let mut lv = Vector::new(pl.items.iter().map(|a| a.value.clone()).collect());
                if pl.items.iter().any(|a| a.tag.is_some()) {
                    lv.attrs = Some(Box::new(Attrs(vec![(s::NAMES, Value::strs(names))])));
                }
                let l = Value::List(std::rc::Rc::new(lv));
                self.generic_vector(interp, &l)?;
                return Ok(());
            }
            Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_) => {
                let dim = v.get_attr(s::DIM);
                match dim {
                    Some(Value::Int(d)) if d.data.len() == 1 => {
                        let dn = v.get_attr(s::DIMNAMES);
                        let rowname = dn.as_ref().and_then(|dn| match dn {
                            Value::List(l) => l.data.first().cloned().filter(|x| !x.is_null()),
                            _ => None,
                        });
                        if let Some(names) = rowname {
                            let title = dn.as_ref().and_then(|dn| dn.get_attr(s::NAMES)).and_then(|n| n.str1()).flatten();
                            if let Some(t) = title {
                                self.p(&t);
                                self.p("\n");
                            }
                            self.named_vector(v, &names);
                        } else {
                            self.vector(v, true);
                        }
                    }
                    Some(Value::Int(d)) if d.data.len() == 2 => {
                        let (rl, cl, rn, cn) = matrix_dimnames(v);
                        self.matrix(v, d.data[0] as usize, d.data[1] as usize, rl, cl, rn, cn, self.pp.quote, self.pp.right);
                    }
                    Some(Value::Int(d)) if d.data.len() > 2 => {
                        self.array(interp, v, &d.data)?;
                    }
                    _ => match v.get_attr(s::NAMES) {
                        Some(names) => self.named_vector(v, &names),
                        None => self.vector(v, true),
                    },
                }
            }
        }
        self.attributes(interp, v, false)
    }

    fn print_special(&mut self, interp: &mut Interp, b: &Builtin) {
        let args_env = interp.base.get_local(Sym::new(".ArgsEnv"));
        let mut def = None;
        for envname in [".ArgsEnv", ".GenericArgsEnv"] {
            let e = if envname == ".ArgsEnv" { args_env.clone() } else { interp.base.get_local(Sym::new(envname)) };
            if let Some(Value::Env(e)) = e {
                if let Some(f) = e.get_local(Sym::new(b.name)) {
                    def = Some(f);
                    break;
                }
            }
        }
        match def {
            Some(f) => {
                let t = crate::deparse::deparse_lines(&f, 60, true, crate::deparse::DEFAULTDEPARSE, None);
                self.p(&t[0]);
                self.p(" ");
                self.p(&format!(".Primitive(\"{}\")\n", b.name));
            }
            None => self.p(&format!(".Primitive(\"{}\")\n", b.name)),
        }
    }

    fn vector_index(&mut self, i: usize, w: usize) {
        let lab = format!("[{}]", i);
        let pad = w.saturating_sub(lab.len());
        self.p(&" ".repeat(pad));
        self.p(&lab);
    }

    pub fn vector(&mut self, v: &Value, indx: bool) {
        let n = v.len();
        if n == 0 {
            self.p(match v {
                Value::Lgl(_) => "logical(0)\n",
                Value::Int(_) => "integer(0)\n",
                Value::Real(_) => "numeric(0)\n",
                Value::Cplx(_) => "complex(0)\n",
                Value::Str(_) => "character(0)\n",
                Value::Raw(_) => "raw(0)\n",
                _ => "NULL\n",
            });
            return;
        }
        let n_pr = if n <= self.pp.max + 1 { n } else { self.pp.max };
        let pp = self.pp.clone();
        let labwidth = if indx { index_width(n) + 2 } else { 0 };
        // Encoded elements with their field width (including gap).
        let (items, w, is_char): (Vec<String>, usize, bool) = match v {
            Value::Lgl(x) => {
                let w = format_logical(&x.data[..n_pr], pp.na_width);
                (x.data[..n_pr].iter().map(|&e| encode_logical(e, w, &pp.na_string)).collect(), w + pp.gap, false)
            }
            Value::Int(x) => {
                let w = format_integer(&x.data[..n_pr], pp.na_width);
                (x.data[..n_pr].iter().map(|&e| encode_integer(e, w, &pp.na_string)).collect(), w + pp.gap, false)
            }
            Value::Real(x) => {
                let (w, d, e) = format_real(&x.data[..n_pr], pp.digits, pp.scipen, 0, pp.na_width);
                (x.data[..n_pr].iter().map(|&e2| encode_real0(e2, w, d, e, &pp.na_string)).collect(), w + pp.gap, false)
            }
            Value::Cplx(x) => {
                let (r, i) = format_complex(&x.data[..n_pr], pp.digits, pp.scipen, 0, pp.na_width);
                let w = r.0 + i.0 + 2;
                let items = x.data[..n_pr]
                    .iter()
                    .map(|z| {
                        if is_na_real(z.re) || is_na_real(z.im) {
                            encode_real0(NA_REAL, w + pp.gap, 0, 0, &pp.na_string)
                        } else {
                            encode_complex(*z, (r.0 + pp.gap, r.1, r.2), i, &pp.na_string)
                        }
                    })
                    .collect();
                (items, w + pp.gap, false)
            }
            Value::Str(x) => {
                let quote = if pp.quote { Some('"') } else { None };
                let w = format_string(&x.data[..n_pr], quote, &pp);
                let just = if pp.right { Justify::Right } else { Justify::Left };
                (x.data[..n_pr].iter().map(|e| encode_string(e, w, quote, just, &pp)).collect(), w, true)
            }
            Value::Raw(x) => (x.data[..n_pr].iter().map(|b| format!("{:02x}", b)).collect(), 2 + pp.gap, true),
            _ => return,
        };
        let mut width;
        if indx {
            self.vector_index(1, labwidth);
            width = labwidth;
        } else {
            width = 0;
        }
        for (i, item) in items.iter().enumerate() {
            let need = if is_char { w + pp.gap } else { w };
            if i > 0 && width + need > pp.width {
                self.p("\n");
                if indx {
                    self.vector_index(i + 1, labwidth);
                    width = labwidth;
                } else {
                    width = 0;
                }
            }
            if is_char {
                self.p(&" ".repeat(pp.gap));
                self.p(item);
                width += w + pp.gap;
            } else {
                self.p(&" ".repeat(w.saturating_sub(item.chars().count())));
                self.p(item);
                width += w;
            }
        }
        self.p("\n");
        if n_pr < n {
            self.p(&format!(
                " [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n",
                n - n_pr
            ));
        }
    }

    pub fn named_vector(&mut self, v: &Value, names: &Value) {
        let n = v.len();
        if n == 0 {
            self.p("named ");
            self.vector(v, true);
            return;
        }
        let pp = self.pp.clone();
        let n_pr = if n <= pp.max + 1 { n } else { pp.max };
        let names: Vec<RStr> = match names {
            Value::Str(s) => s.data.clone(),
            _ => vec![None; n],
        };
        let (items, mut w): (Vec<String>, usize) = match v {
            Value::Lgl(x) => {
                let w = format_logical(&x.data[..n_pr], pp.na_width);
                (x.data[..n_pr].iter().map(|&e| encode_logical(e, w, &pp.na_string)).collect(), w)
            }
            Value::Int(x) => {
                let w = format_integer(&x.data[..n_pr], pp.na_width);
                (x.data[..n_pr].iter().map(|&e| encode_integer(e, w, &pp.na_string)).collect(), w)
            }
            Value::Real(x) => {
                let (w, d, e) = format_real(&x.data[..n_pr], pp.digits, pp.scipen, 0, pp.na_width);
                (x.data[..n_pr].iter().map(|&e2| encode_real0(e2, w, d, e, &pp.na_string)).collect(), w)
            }
            Value::Cplx(x) => {
                let (r, i) = format_complex(&x.data[..n_pr], pp.digits, pp.scipen, 0, pp.na_width);
                let w = r.0 + i.0 + 2;
                (x.data[..n_pr].iter().map(|z| encode_complex(*z, r, i, &pp.na_string)).collect(), w)
            }
            Value::Str(x) => {
                let quote = if pp.quote { Some('"') } else { None };
                let w = format_string(&x.data[..n_pr], quote, &pp);
                (x.data[..n_pr].iter().map(|e| encode_string(e, w, quote, Justify::Right, &pp)).collect(), w)
            }
            Value::Raw(x) => (x.data[..n_pr].iter().map(|b| format!("{:02x}", b)).collect(), 2),
            _ => return,
        };
        let wn = format_string(&names[..n_pr.min(names.len())], None, &pp);
        if w < wn {
            w = wn;
        }
        let nperline = (pp.width / (w + pp.gap)).max(1);
        let nlines = n_pr.div_ceil(nperline);
        for i in 0..nlines {
            if i > 0 {
                self.p("\n");
            }
            for j in 0..nperline {
                let k = i * nperline + j;
                if k >= n_pr {
                    break;
                }
                let nm = names.get(k).cloned().unwrap_or(None);
                let s = encode_string(&nm, w, None, Justify::Right, &pp);
                self.p(&s);
                self.p(&" ".repeat(pp.gap));
            }
            self.p("\n");
            for j in 0..nperline {
                let k = i * nperline + j;
                if k >= n_pr {
                    break;
                }
                let item = &items[k];
                let pad = w.saturating_sub(str_width(item));
                self.p(&" ".repeat(pad));
                self.p(item);
                self.p(&" ".repeat(pp.gap));
            }
        }
        self.p("\n");
        if n_pr < n {
            self.p(&format!(
                " [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n",
                n - n_pr
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn matrix(
        &mut self,
        v: &Value,
        r: usize,
        c: usize,
        rl: Option<Vec<RStr>>,
        cl: Option<Vec<RStr>>,
        rn: Option<String>,
        cn: Option<String>,
        quote: bool,
        right: bool,
    ) {
        let pp = self.pp.clone();
        let r_pr = r.min(if c > 0 { pp.max / c.max(1) } else { r });
        let mut rlabw = match &rl {
            Some(rl) => format_string(rl, None, &pp),
            None => index_width(r + 1) + 3,
        };
        let mut lbloff = 0;
        if let Some(rn) = &rn {
            let rnw = str_width(rn);
            if rnw < rlabw + R_MIN_LBLOFF {
                lbloff = R_MIN_LBLOFF;
            } else {
                lbloff = rnw - rlabw;
            }
            rlabw += lbloff;
        }
        let is_str = matches!(v, Value::Str(_));
        let qc = if quote { Some('"') } else { None };
        // Encode each column.
        let mut cols: Vec<Vec<String>> = Vec::with_capacity(c);
        let mut w: Vec<usize> = Vec::with_capacity(c);
        for j in 0..c {
            let range = j * r..j * r + r;
            let (enc, wj): (Vec<String>, usize) = match v {
                Value::Lgl(x) => {
                    let wj = format_logical(&x.data[range.clone()], pp.na_width);
                    (x.data[range].iter().map(|&e| encode_logical(e, wj, &pp.na_string)).collect(), wj)
                }
                Value::Int(x) => {
                    let wj = format_integer(&x.data[range.clone()], pp.na_width);
                    (x.data[range].iter().map(|&e| encode_integer(e, wj, &pp.na_string)).collect(), wj)
                }
                Value::Real(x) => {
                    let (wj, d, e) = format_real(&x.data[range.clone()], pp.digits, pp.scipen, 0, pp.na_width);
                    (x.data[range].iter().map(|&e2| encode_real0(e2, wj, d, e, &pp.na_string)).collect(), wj)
                }
                Value::Cplx(x) => {
                    let (rr, ii) = format_complex(&x.data[range.clone()], pp.digits, pp.scipen, 0, pp.na_width);
                    let wj = rr.0 + ii.0 + 2;
                    (x.data[range].iter().map(|z| encode_complex(*z, rr, ii, &pp.na_string)).collect(), wj)
                }
                Value::Str(x) => {
                    let wj = format_string(&x.data[range.clone()], qc, &pp);
                    let just = if right { Justify::Right } else { Justify::Left };
                    (x.data[range].iter().map(|e| encode_string(e, wj, qc, just, &pp)).collect(), wj)
                }
                Value::Raw(x) => (x.data[range].iter().map(|b| format!("{:02x}", b)).collect(), 2),
                _ => (vec![], 0),
            };
            let clabw = match &cl {
                Some(cl) => match &cl[j] {
                    None => pp.na_width_noquote,
                    Some(s) => str_width(s),
                },
                None => index_width(j + 1) + 3,
            };
            let mut wj = wj.max(clabw);
            if !is_str {
                wj += pp.gap;
            }
            w.push(wj);
            cols.push(enc);
        }
        let extra = if is_str { pp.gap } else { 0 };
        let print_row_lab = |this: &mut Printer| {
            if let Some(cn) = &cn {
                this.p(&" ".repeat(rlabw));
                this.p(cn);
                this.p("\n");
            }
            match &rn {
                Some(rn) => {
                    this.p(rn);
                    this.p(&" ".repeat(rlabw.saturating_sub(str_width(rn))));
                }
                None => this.p(&" ".repeat(rlabw)),
            }
        };
        let row_label = |this: &mut Printer, i: usize| {
            this.p("\n");
            match &rl {
                Some(rl) => {
                    let (s, l) = match &rl[i] {
                        None => (pp.na_string_noquote.clone(), pp.na_width_noquote),
                        Some(s) => {
                            let e = escape_string(s, None);
                            let l = str_width(&e);
                            (e, l)
                        }
                    };
                    this.p(&" ".repeat(lbloff));
                    this.p(&s);
                    this.p(&" ".repeat(rlabw.saturating_sub(l + lbloff)));
                }
                None => {
                    let lab = format!("[{},]", i + 1);
                    this.p(&" ".repeat(rlabw.saturating_sub(lab.len())));
                    this.p(&lab);
                }
            }
        };
        if c == 0 {
            print_row_lab(self);
            for i in 0..r {
                row_label(self, i);
            }
            self.p("\n");
            return;
        }
        let mut jmin = 0;
        while jmin < c {
            let mut width = rlabw;
            let mut jmax = jmin;
            loop {
                width += w[jmax] + extra;
                jmax += 1;
                if !(jmax < c && width + w[jmax] + extra < pp.width) {
                    break;
                }
            }
            print_row_lab(self);
            for j in jmin..jmax {
                if is_str {
                    // Column labels for strings: left- or right-aligned.
                    let (lab, l) = match &cl {
                        Some(cl) => match &cl[j] {
                            None => (pp.na_string_noquote.clone(), pp.na_width_noquote),
                            Some(s) => {
                                let e = escape_string(s, None);
                                let l = str_width(&e);
                                (e, l)
                            }
                        },
                        None => {
                            let lab = format!("[,{}]", j + 1);
                            let l = lab.len();
                            (lab, l)
                        }
                    };
                    if right {
                        self.p(&" ".repeat(pp.gap + w[j].saturating_sub(l)));
                        self.p(&lab);
                    } else {
                        self.p(&" ".repeat(pp.gap));
                        self.p(&lab);
                        self.p(&" ".repeat(w[j].saturating_sub(l)));
                    }
                } else {
                    match &cl {
                        Some(cl) => {
                            let (lab, l) = match &cl[j] {
                                None => (pp.na_string_noquote.clone(), pp.na_width_noquote),
                                Some(s) => {
                                    let e = escape_string(s, None);
                                    let l = str_width(&e);
                                    (e, l)
                                }
                            };
                            self.p(&" ".repeat(w[j].saturating_sub(l)));
                            self.p(&lab);
                        }
                        None => {
                            let lab = format!("[,{}]", j + 1);
                            self.p(&" ".repeat(w[j].saturating_sub(lab.len())));
                            self.p(&lab);
                        }
                    }
                }
            }
            for i in 0..r_pr {
                row_label(self, i);
                for j in jmin..jmax {
                    let item = &cols[j][i];
                    if is_str {
                        self.p(&" ".repeat(pp.gap));
                        self.p(item);
                    } else {
                        self.p(&" ".repeat(w[j].saturating_sub(str_width(item))));
                        self.p(item);
                    }
                }
            }
            self.p("\n");
            jmin = jmax;
        }
        if r_pr < r {
            self.p(&format!(
                " [ reached getOption(\"max.print\") -- omitted {} rows ]\n",
                r - r_pr
            ));
        }
    }

    fn array(&mut self, interp: &mut Interp, v: &Value, dims: &[i32]) -> RResult<()> {
        let _ = interp;
        let nr = dims[0] as usize;
        let nc = dims[1] as usize;
        let b = nr * nc;
        let total: usize = dims.iter().map(|&d| d as usize).product();
        let nb = if b == 0 { 0 } else { total / b };
        let dn = v.get_attr(s::DIMNAMES);
        let dnl: Option<Vec<Value>> = match &dn {
            Some(Value::List(l)) => Some(l.data.clone()),
            _ => None,
        };
        let dnn: Option<Vec<RStr>> = dn.as_ref().and_then(|d| d.get_attr(s::NAMES)).and_then(|n| match n {
            Value::Str(s) => Some(s.data.clone()),
            _ => None,
        });
        let strs = |x: Option<&Value>| -> Option<Vec<RStr>> {
            match x {
                Some(Value::Str(s)) => Some(s.data.clone()),
                _ => None,
            }
        };
        let rl = dnl.as_ref().and_then(|l| strs(l.first()));
        let cl = dnl.as_ref().and_then(|l| strs(l.get(1)));
        let rn = dnn.as_ref().and_then(|n| n.first().cloned().flatten()).map(|s| s.to_string()).filter(|s| !s.is_empty());
        let cn = dnn.as_ref().and_then(|n| n.get(1).cloned().flatten()).map(|s| s.to_string()).filter(|s| !s.is_empty());
        for k in 0..nb {
            // Header ", , i, j"
            let mut header = String::from(", ");
            let mut kk = k;
            for (d, &dim) in dims.iter().enumerate().skip(2) {
                let ix = kk % dim as usize;
                kk /= dim as usize;
                header.push_str(", ");
                let lab = dnl.as_ref().and_then(|l| strs(l.get(d))).and_then(|v| v.get(ix).cloned().flatten());
                let name = dnn.as_ref().and_then(|n| n.get(d).cloned().flatten()).filter(|s| !s.is_empty());
                match (lab, name) {
                    (Some(l), Some(n)) => header.push_str(&format!("{} = {}", n, l)),
                    (Some(l), None) => header.push_str(&l),
                    (None, _) => header.push_str(&(ix + 1).to_string()),
                }
            }
            self.p(&header);
            self.p("\n\n");
            let slice = slice_value(v, k * b, b);
            self.matrix(&slice, nr, nc, rl.clone(), cl.clone(), rn.clone(), cn.clone(), self.pp.quote, self.pp.right);
            self.p("\n");
        }
        Ok(())
    }

    fn generic_vector(&mut self, interp: &mut Interp, v: &Value) -> RResult<()> {
        let Value::List(l) = v else { return Ok(()) };
        let ns = l.data.len();
        if let Some(Value::Int(d)) = v.get_attr(s::DIM) {
            if d.data.len() > 1 {
                // Array-like list: print summaries.
                let t: Vec<RStr> = l.data.iter().map(|x| rstr(&list_elt_summary(x))).collect();
                let tv = Value::strs(t);
                if d.data.len() == 2 {
                    let (rl, cl, rn, cn) = matrix_dimnames(v);
                    self.matrix(&tv, d.data[0] as usize, d.data[1] as usize, rl, cl, rn, cn, false, self.pp.right);
                }
                return self.attributes(interp, v, false);
            }
        }
        let names: Option<Vec<RStr>> = match v.get_attr(s::NAMES) {
            Some(Value::Str(n)) => Some(n.data.clone()),
            _ => None,
        };
        let taglen = self.tagbuf.len();
        if ns > 0 {
            let n_pr = if ns <= self.pp.max + 1 { ns } else { self.pp.max };
            for i in 0..n_pr {
                if i > 0 {
                    self.p("\n");
                }
                let name = names.as_ref().and_then(|n| n.get(i).cloned());
                match name {
                    Some(Some(nm)) if !nm.is_empty() => {
                        if taglen + nm.len() > TAGBUFLEN {
                            if taglen <= TAGBUFLEN {
                                self.tagbuf.push_str("$...");
                            }
                        } else if crate::deparse::is_valid_name(&nm) {
                            self.tagbuf.push_str(&format!("${}", nm));
                        } else {
                            self.tagbuf.push_str(&format!("$`{}`", nm));
                        }
                    }
                    Some(None) => self.tagbuf.push_str("$<NA>"),
                    _ => {
                        if taglen + index_width(i) > TAGBUFLEN {
                            if taglen <= TAGBUFLEN {
                                self.tagbuf.push_str("$...");
                            }
                        } else {
                            self.tagbuf.push_str(&format!("[[{}]]", i + 1));
                        }
                    }
                }
                let tb = self.tagbuf.clone();
                self.p(&tb);
                self.p("\n");
                self.dispatch(interp, &l.data[i])?;
                self.tagbuf.truncate(taglen);
            }
            self.p("\n");
            if n_pr < ns {
                self.p(&format!(
                    " [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n",
                    ns - n_pr
                ));
            }
        } else {
            if names.is_some() {
                self.p("named ");
            }
            self.p("list()\n");
        }
        self.attributes(interp, v, false)
    }

    fn attributes(&mut self, interp: &mut Interp, v: &Value, use_slots: bool) -> RResult<()> {
        let Some(attrs) = v.attrs().cloned().or_else(|| v.ref_attrs()) else { return Ok(()) };
        if attrs.is_empty() {
            return Ok(());
        }
        let save = self.tagbuf.clone();
        if !self.tagbuf.is_empty() && !self.tagbuf.ends_with(')') {
            self.tagbuf.clear();
        }
        let base_len = self.tagbuf.len();
        let is_array = v.get_attr(s::DIM).is_some();
        let is_factor = v.inherits("factor");
        let is_df = v.inherits("data.frame");
        for (tag, val) in &attrs.0 {
            if use_slots && *tag == s::CLASS {
                continue;
            }
            if is_array && (*tag == s::DIM || *tag == s::DIMNAMES) {
                continue;
            }
            if is_factor && (*tag == s::LEVELS || *tag == s::CLASS) {
                continue;
            }
            if is_df && *tag == s::ROW_NAMES {
                continue;
            }
            if !is_array && *tag == s::NAMES {
                continue;
            }
            if *tag == s::COMMENT || *tag == s::SRCREF {
                continue;
            }
            let name = tag.name();
            if matches!(&*name, "wholeSrcref" | "srcfile") {
                continue;
            }
            self.tagbuf.push_str(&format!("attr(,\"{}\")", name));
            let tb = self.tagbuf.clone();
            self.p(&tb);
            self.p("\n");
            self.dispatch(interp, val)?;
            self.tagbuf.truncate(base_len);
        }
        self.tagbuf = save;
        Ok(())
    }
}

pub fn encode_environment(e: &Env) -> String {
    match e.special_name() {
        Some(n) if &*n == "R_GlobalEnv" || &*n == "R_EmptyEnv" || &*n == "base" => format!("<environment: {}>", n),
        Some(n) if n.starts_with("namespace:") => format!("<environment: {}>", n),
        _ => format!("<environment: 0x{:012x}>", 0x55d5_c000_0000usize + e.id() * 0x38),
    }
}

/// Row/column dimnames of a matrix and their names.
#[allow(clippy::type_complexity)]
pub fn matrix_dimnames(v: &Value) -> (Option<Vec<RStr>>, Option<Vec<RStr>>, Option<String>, Option<String>) {
    let dn = v.get_attr(s::DIMNAMES);
    let Some(Value::List(l)) = &dn else { return (None, None, None, None) };
    let strs = |x: Option<&Value>| -> Option<Vec<RStr>> {
        match x {
            Some(Value::Str(s)) => Some(s.data.clone()),
            _ => None,
        }
    };
    let rl = strs(l.data.first());
    let cl = strs(l.data.get(1));
    let names = dn.as_ref().unwrap().get_attr(s::NAMES);
    let (rn, cn) = match names {
        Some(Value::Str(n)) => (
            n.data.first().cloned().flatten().map(|s| s.to_string()),
            n.data.get(1).cloned().flatten().map(|s| s.to_string()).filter(|s| !s.is_empty()),
        ),
        _ => (None, None),
    };
    (rl, cl, rn, cn)
}

fn slice_value(v: &Value, start: usize, len: usize) -> Value {
    match v {
        Value::Lgl(x) => Value::lgls(x.data[start..start + len].to_vec()),
        Value::Int(x) => Value::ints(x.data[start..start + len].to_vec()),
        Value::Real(x) => Value::reals(x.data[start..start + len].to_vec()),
        Value::Cplx(x) => Value::cplxs(x.data[start..start + len].to_vec()),
        Value::Str(x) => Value::strs(x.data[start..start + len].to_vec()),
        Value::Raw(x) => Value::raws(x.data[start..start + len].to_vec()),
        Value::List(x) => Value::list(x.data[start..start + len].to_vec()),
        _ => Value::Null,
    }
}

fn list_elt_summary(x: &Value) -> String {
    match x {
        Value::Null => "NULL".into(),
        Value::Lgl(v) if v.data.len() == 1 => encode_logical(v.data[0], 0, "NA"),
        Value::Lgl(v) => format!("logical,{}", v.data.len()),
        Value::Int(v) if v.data.len() == 1 && !x.is_object() => encode_integer(v.data[0], 0, "NA"),
        Value::Int(v) => {
            if x.inherits("factor") {
                format!("factor,{}", v.data.len())
            } else {
                format!("integer,{}", v.data.len())
            }
        }
        Value::Real(v) if v.data.len() == 1 => {
            let (w, d, e) = format_real(&v.data, 7, 0, 0, 2);
            encode_real0(v.data[0], w, d, e, "NA")
        }
        Value::Real(v) => format!("numeric,{}", v.data.len()),
        Value::Cplx(v) => format!("complex,{}", v.data.len()),
        Value::Str(v) if v.data.len() == 1 => match &v.data[0] {
            Some(s) => format!("\"{}\"", s),
            None => "NA".into(),
        },
        Value::Str(v) => format!("character,{}", v.data.len()),
        Value::Raw(v) => format!("raw,{}", v.data.len()),
        Value::List(v) => format!("list,{}", v.data.len()),
        Value::Lang(_) => "expression".into(),
        _ => "?".into(),
    }
}
