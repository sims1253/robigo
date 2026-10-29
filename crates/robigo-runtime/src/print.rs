//! Printing of R values. Port of GNU R's print.c, printvector.c and
//! printarray.c.

use crate::env::Env;
use crate::error::RResult;
use crate::format::*;
use crate::interp::Interp;
use crate::sym::{Sym, s};
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
    let mut p = Printer {
        pp: interp.print_params(),
        tagbuf: String::new(),
        env: env.clone(),
        out: String::new(),
        call_args: vec![],
    };
    p.value_rec(interp, v)?;
    p.flush(interp);
    Ok(())
}

impl Printer {
    pub fn new(interp: &Interp, env: &Env) -> Printer {
        Printer {
            pp: interp.print_params(),
            tagbuf: String::new(),
            env: env.clone(),
            out: String::new(),
            call_args: vec![],
        }
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
            // R: print.c:PrintObject — S4 objects are shown by show().
            if v.is_s4() && interp.methods_dispatch_on() {
                let env = self.env.clone();
                interp.show_s4(v, &env)?;
                self.tagbuf = save;
                return Ok(());
            }
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
        // R: print.c:PrintValueRec — S4 objects without the methods package.
        if (v.is_s4() || matches!(v, Value::Obj(_))) && !interp.methods_dispatch_on() {
            match v.get_attr(s::CLASS) {
                None => {
                    if matches!(v, Value::Obj(_)) {
                        self.p("<S4 object without a class>\n");
                    } else {
                        self.p(&format!("<Object of type '{}' with S4 bit but without a class>\n", v.type_name()));
                    }
                }
                Some(cl) => {
                    // CHAR(STRING_ELT(x, 0)): "NA" for NA; a zero-length vector
                    // reads as "" in GNU R.
                    let first = |v: &Value| -> String {
                        match crate::coerce::to_str(v).first() {
                            Some(Some(s)) => s.to_string(),
                            Some(None) => "NA".into(),
                            None => String::new(),
                        }
                    };
                    let c = first(&cl);
                    match v.get_attr(Sym::new("package")) {
                        None => self.p(&format!("<S4 object of class \"{}\">\n", c)),
                        Some(pkg) => {
                            let p = first(&pkg);
                            self.p(&format!("<S4 object of class \"{}\" from package '{}'>\n", c, p));
                        }
                    }
                }
            }
            return Ok(());
        }
        match v {
            Value::Null => self.p("NULL\n"),
            Value::Sym(_) => {
                let t = crate::deparse::deparse_lines(v, 60, true, crate::deparse::SIMPLEDEPARSE, None);
                self.p(&t[0]);
                self.p("\n");
            }
            Value::Builtin(b) => self.print_special(interp, b),
            Value::Expr(_) | Value::Lang(_) => {
                let lines = interp.deparse_prepared(v, || {
                    crate::deparse::deparse_lines(v, 60, true, crate::deparse::DEFAULTDEPARSE, None)
                })?;
                for l in lines {
                    self.p(&l);
                    self.p("\n");
                }
            }
            Value::Closure(c) => {
                let lines = interp.deparse_prepared(v, || {
                    crate::deparse::deparse_lines(v, 60, true, crate::deparse::DEFAULTDEPARSE, None)
                })?;
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
            // R: print.c:PrintValueRec (OBJSXP)
            Value::Obj(_) => self.p(if v.is_s4() { "<S4 Type Object>\n" } else { "<object>\n" }),
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
                    lv.attrs = Some(Box::new(Attrs(vec![(s::NAMES, Value::strs(names))], false)));
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
                            let title =
                                dn.as_ref().and_then(|dn| dn.get_attr(s::NAMES)).and_then(|n| n.str1()).flatten();
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
                        self.matrix(
                            v,
                            d.data[0] as usize,
                            d.data[1] as usize,
                            rl,
                            cl,
                            rn,
                            cn,
                            self.pp.quote,
                            self.pp.right,
                        );
                    }
                    Some(Value::Int(d)) if d.data.len() > 2 => {
                        self.array(interp, v, &d.data, self.pp.quote, self.pp.right)?;
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
        let args_env = interp.base_binding(Sym::new(".ArgsEnv"));
        let mut def = None;
        for envname in [".ArgsEnv", ".GenericArgsEnv"] {
            let e = if envname == ".ArgsEnv" { args_env.clone() } else { interp.base_binding(Sym::new(envname)) };
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
            // R: printvector.c printRawVector — the gap is part of the field
            // width for the line-break test, unlike character vectors.
            Value::Raw(x) => (x.data[..n_pr].iter().map(|b| format!("{:02x}", b)).collect(), 2 + pp.gap, false),
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
            self.p(&format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", n - n_pr));
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
        // R: printvector.c printNamedComplexVectorS separates complex items
        // with a gap *before* each item (not after the last one); the other
        // types print the gap after each item.
        let gap_before = matches!(v, Value::Cplx(_));
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
                if gap_before {
                    if j > 0 {
                        self.p(&" ".repeat(pp.gap));
                    }
                    // NA complexes are padded to the full field width (which
                    // may have grown to fit a long name); others are not.
                    let na = matches!(v, Value::Cplx(x) if is_na_real(x.data[k].re) || is_na_real(x.data[k].im));
                    if na {
                        self.p(&" ".repeat(w.saturating_sub(str_width(item))));
                    }
                    self.p(item);
                } else {
                    let pad = w.saturating_sub(str_width(item));
                    self.p(&" ".repeat(pad));
                    self.p(item);
                    self.p(&" ".repeat(pp.gap));
                }
            }
        }
        self.p("\n");
        if n_pr < n {
            self.p(&format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", n - n_pr));
        }
    }

    /// R: printarray.c:printMatrix. `rn`/`cn` are `names(dimnames(x))`
    /// (`None` when there are no names).
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
        if r == 0 && c == 0 {
            self.p("<0 x 0 matrix>\n");
            return;
        }
        let max = self.pp.max;
        let mut r_pr = r;
        let c_pr = if c > max { max } else { c };
        if c > 0 && max / c < r {
            r_pr = max / c;
        }
        if c > c_pr && r_pr < 1 && r > 0 {
            r_pr = 1;
        }
        self.matrix_body(v, 0, r_pr, r, c_pr, &rl, &cl, rn.as_deref(), cn.as_deref(), quote, right, true);
        if r_pr < r || c_pr < c {
            self.p(" [ reached 'max' / getOption(\"max.print\") -- omitted");
            if r_pr < r {
                let k = r - r_pr;
                self.p(&format!(" {} row{}", k, if k == 1 { "" } else { "s" }));
            }
            if c_pr < c {
                if r_pr < r {
                    self.p(" and");
                }
                let k = c - c_pr;
                self.p(&format!(" {} column{}", k, if k == 1 { "" } else { "s" }));
            }
            self.p(" ]\n");
        }
    }

    /// R: printarray.c:MatrixRowLabel
    fn matrix_row_label(&mut self, rl: &Option<Vec<RStr>>, i: usize, rlabw: usize, lbloff: usize) {
        match rl {
            Some(rl) => {
                let (s, l) = match &rl[i] {
                    None => (self.pp.na_string_noquote.clone(), self.pp.na_width_noquote),
                    Some(s) => (escape_string(s, None), rstrlen(s, None)),
                };
                self.p("\n");
                self.p(&" ".repeat(lbloff));
                self.p(&s);
                self.p(&" ".repeat(rlabw.saturating_sub(l + lbloff)));
            }
            None => {
                let lab = format!("[{},]", i + 1);
                self.p("\n");
                self.p(&" ".repeat(rlabw.saturating_sub(3 + index_width(i + 1))));
                self.p(&lab);
            }
        }
    }

    fn col_label(&self, cl: &Option<Vec<RStr>>, j: usize) -> (String, usize) {
        match cl {
            Some(cl) => match &cl[j] {
                None => (self.pp.na_string_noquote.clone(), self.pp.na_width_noquote),
                Some(s) => (escape_string(s, None), rstrlen(s, None)),
            },
            None => (format!("[,{}]", j + 1), index_width(j + 1) + 3),
        }
    }

    /// The print<TYPE>Matrix() functions of printarray.c: print rows
    /// `0..r_pr` and columns `0..c` of the `r`-row matrix stored at
    /// `offset` in `v`.
    #[allow(clippy::too_many_arguments)]
    fn matrix_body(
        &mut self,
        v: &Value,
        offset: usize,
        r_pr: usize,
        r: usize,
        c: usize,
        rl: &Option<Vec<RStr>>,
        cl: &Option<Vec<RStr>>,
        rn: Option<&str>,
        cn: Option<&str>,
        quote: bool,
        right: bool,
        print_ij: bool,
    ) {
        let pp = self.pp.clone();
        // _PRINT_INIT_rl_rn
        let mut rlabw = match rl {
            Some(rl) => format_string(&rl[..r.min(rl.len())], None, &pp),
            None => index_width(r + 1) + 3,
        };
        let mut lbloff = 0;
        if let Some(rn) = rn {
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
        // _COMPUTE_W_: column widths and per-column format info.
        let mut w = vec![0usize; c];
        let mut real_fmt: Vec<(usize, usize, usize)> = vec![(0, 0, 0); c];
        let mut cplx_fmt: Vec<((usize, usize, usize), (usize, usize, usize))> = vec![((0, 0, 0), (0, 0, 0)); c];
        for j in 0..c {
            let range = offset + j * r..offset + j * r + r;
            if print_ij {
                w[j] = match v {
                    Value::Lgl(x) => format_logical(&x.data[range], pp.na_width),
                    Value::Int(x) => format_integer(&x.data[range], pp.na_width),
                    Value::Real(x) => {
                        let f = format_real(&x.data[range], pp.digits, pp.scipen, 0, pp.na_width);
                        real_fmt[j] = f;
                        f.0
                    }
                    Value::Cplx(x) => {
                        let f = format_complex(&x.data[range], pp.digits, pp.scipen, 0, pp.na_width);
                        cplx_fmt[j] = f;
                        f.0.0 + f.1.0 + 2
                    }
                    Value::Str(x) => format_string(&x.data[range], qc, &pp),
                    Value::Raw(_) => 2,
                    _ => 0,
                };
            }
            let clabw = match cl {
                Some(cl) => match &cl[j] {
                    None => pp.na_width_noquote,
                    Some(s) => str_width(s),
                },
                None => index_width(j + 1) + 3,
            };
            if w[j] < clabw {
                w[j] = clabw;
            }
            if !is_str {
                w[j] += pp.gap;
            }
        }
        let extra = if is_str { pp.gap } else { 0 };
        let print_row_lab = |this: &mut Printer| {
            if let Some(cn) = cn {
                this.p(&" ".repeat(rlabw));
                this.p(cn);
                this.p("\n");
            }
            match rn {
                Some(rn) => {
                    this.p(rn);
                    this.p(&" ".repeat(rlabw.saturating_sub(str_width(rn))));
                }
                None => this.p(&" ".repeat(rlabw)),
            }
        };
        if c == 0 {
            print_row_lab(self);
            for i in 0..r {
                self.matrix_row_label(rl, i, rlabw, lbloff);
            }
            self.p("\n");
            return;
        }
        let mut jmin = 0;
        let mut jmax = 0;
        while jmin < c {
            let mut width = rlabw;
            loop {
                width += w[jmax] + extra;
                jmax += 1;
                if !(jmax < c && width + w[jmax] + extra < pp.width) {
                    break;
                }
            }
            print_row_lab(self);
            for j in jmin..jmax {
                let (lab, l) = self.col_label(cl, j);
                if is_str {
                    if right {
                        // R: RightMatrixColumnLabel
                        self.p(&" ".repeat((pp.gap + w[j]).saturating_sub(l)));
                        self.p(&lab);
                    } else {
                        // R: LeftMatrixColumnLabel
                        self.p(&" ".repeat(pp.gap));
                        self.p(&lab);
                        self.p(&" ".repeat(w[j].saturating_sub(l)));
                    }
                } else {
                    // R: MatrixColumnLabel
                    self.p(&" ".repeat(w[j].saturating_sub(l)));
                    self.p(&lab);
                }
            }
            for i in 0..r_pr {
                self.matrix_row_label(rl, i, rlabw, lbloff);
                if !print_ij {
                    continue;
                }
                for j in jmin..jmax {
                    let k = offset + i + j * r;
                    let s = match v {
                        Value::Lgl(x) => encode_logical(x.data[k], w[j], &pp.na_string),
                        Value::Int(x) => encode_integer(x.data[k], w[j], &pp.na_string),
                        Value::Real(x) => {
                            let (_, d, e) = real_fmt[j];
                            encode_real0(x.data[k], w[j], d, e, &pp.na_string)
                        }
                        Value::Cplx(x) => {
                            let z = x.data[k];
                            if is_na_real(z.re) || is_na_real(z.im) {
                                encode_real0(NA_REAL, w[j], 0, 0, &pp.na_string)
                            } else {
                                let (rf, imf) = cplx_fmt[j];
                                encode_complex(z, (w[j] - imf.0 - 2, rf.1, rf.2), imf, &pp.na_string)
                            }
                        }
                        Value::Str(x) => {
                            let just = if right { Justify::Right } else { Justify::Left };
                            format!("{}{}", " ".repeat(pp.gap), encode_string(&x.data[k], w[j], qc, just, &pp))
                        }
                        Value::Raw(x) => format!("{}{:02x}", " ".repeat(w[j].saturating_sub(2)), x.data[k]),
                        _ => String::new(),
                    };
                    self.p(&s);
                }
            }
            self.p("\n");
            jmin = jmax;
        }
    }

    /// R: printarray.c:printArray for arrays of three or more dimensions.
    fn array(&mut self, interp: &mut Interp, v: &Value, dims: &[i32], quote: bool, right: bool) -> RResult<()> {
        let _ = interp;
        let ndim = dims.len();
        let nr = dims[0] as usize;
        let nc = dims[1] as usize;
        let b = nr * nc;
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
        let dn0 = dnl.as_ref().and_then(|l| strs(l.first()));
        let dn1 = dnl.as_ref().and_then(|l| strs(l.get(1)));
        let nm = |k: usize| -> Option<String> {
            dnn.as_ref().map(|n| n.get(k).cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into()))
        };
        let (rn, cn) = (nm(0), nm(1));
        let nb: usize = dims[2..].iter().map(|&d| d as usize).product();
        let max = self.pp.max;
        let max_reached = b > 0 && max / b < nb;
        let (nb_pr, nc_last, nr_last);
        if max_reached {
            let mut nbp = max.div_ceil(b);
            let ne_last = max - b * (nbp - 1);
            let mut ncl = if ne_last < nc { ne_last } else { nc };
            let mut nrl = if ne_last < nc { 1 } else { ne_last / nc };
            if nrl == 0 {
                nbp -= 1;
                ncl = nc;
                nrl = nr;
            }
            nb_pr = nbp;
            nc_last = ncl;
            nr_last = nrl;
        } else {
            nb_pr = if nb > 0 { nb } else { 1 };
            nc_last = nc;
            nr_last = nr;
        }
        for i in 0..nb_pr {
            let do_ij = nb > 0;
            let i_last = i == nb_pr - 1;
            let use_nc = if i_last { nc_last } else { nc };
            let use_nr = if i_last { nr_last } else { nr };
            if do_ij {
                let mut k = 1;
                let mut header = String::from(", ");
                for j in 2..ndim {
                    let l = (i / k) % dims[j] as usize + 1;
                    match dnl.as_ref().and_then(|d| strs(d.get(j))) {
                        Some(lab) => {
                            let lab_s = lab[l - 1].as_deref().unwrap_or("NA").to_string();
                            match &dnn {
                                Some(n) => {
                                    header.push_str(&format!(", {} = {}", n[j].as_deref().unwrap_or("NA"), lab_s))
                                }
                                None => header.push_str(&format!(", {}", lab_s)),
                            }
                        }
                        None => header.push_str(&format!(", {}", l)),
                    }
                    k *= dims[j] as usize;
                }
                self.p(&header);
                self.p("\n\n");
            } else {
                let mut s = String::new();
                for (k, d) in dims.iter().enumerate() {
                    s.push_str(&format!("{}{}", if k == 0 { "<" } else { " x " }, d));
                }
                let tn = match v {
                    Value::Real(_) => "double",
                    other => other.type_name(),
                };
                s.push_str(&format!(" array of {}>\n", tn));
                self.p(&s);
            }
            self.matrix_body(
                v,
                i * b,
                use_nr,
                nr,
                use_nc,
                &dn0,
                &dn1,
                rn.as_deref(),
                cn.as_deref(),
                quote,
                right,
                do_ij,
            );
            self.p("\n");
        }
        if max_reached {
            self.p(" [ reached 'max' / getOption(\"max.print\") -- omitted");
            if nb_pr < nb {
                let k = nb - nb_pr;
                self.p(&format!(" {} slice{}", k, if k == 1 { "" } else { "s" }));
            } else if nb_pr == nb {
                let nr2 = nr - nr_last;
                if nr2 > 0 {
                    self.p(&format!(" {} row{}", nr2, if nr2 == 1 { "" } else { "s" }));
                }
                let nc2 = nc - nc_last;
                if nc2 > 0 {
                    self.p(&format!(" {} column{}", nc2, if nc2 == 1 { "" } else { "s" }));
                }
            }
            self.p(" ] \n");
        }
        Ok(())
    }

    fn generic_vector(&mut self, interp: &mut Interp, v: &Value) -> RResult<()> {
        let Value::List(l) = v else { return Ok(()) };
        let ns = l.data.len();
        if let Some(Value::Int(d)) = v.get_attr(s::DIM) {
            if d.data.len() > 1 {
                // Array-like list: print summaries.
                let t: Vec<RStr> = l
                    .data
                    .iter()
                    .map(|x| match object_summary(interp, x, &self.env) {
                        Some(s) => Some(std::rc::Rc::from(s.as_str())),
                        None => rstr(&list_elt_summary(x)),
                    })
                    .collect();
                let mut tv = Value::strs(t);
                if d.data.len() == 2 {
                    let (rl, cl, rn, cn) = matrix_dimnames(v);
                    self.matrix(&tv, d.data[0] as usize, d.data[1] as usize, rl, cl, rn, cn, false, self.pp.right);
                } else {
                    // print.c PrintGenericVector: print the summary vector as
                    // an array, keeping the original's dimnames.
                    tv.set_attr_raw(s::DIM, Value::ints(d.data.clone()));
                    if let Some(dn) = v.get_attr(s::DIMNAMES) {
                        tv.set_attr_raw(s::DIMNAMES, dn);
                    }
                    self.array(interp, &tv, &d.data, false, false)?;
                }
                return self.attributes(interp, v, false);
            }
        }
        // getAttrib(s, R_NamesSymbol): dimnames[[1]] for a 1-d array.
        let names: Option<Vec<RStr>> = match crate::attrib::get_attrib(v, s::NAMES) {
            Value::Str(n) => Some(n.data.clone()),
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
                self.p(&format!(" [ reached 'max' / getOption(\"max.print\") -- omitted {} entries ]\n", ns - n_pr));
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

// R: printutils.c:EncodeEnvironment
pub fn encode_environment(e: &Env) -> String {
    if let Some(n) = e.special_name().filter(|n| matches!(&**n, "R_GlobalEnv" | "R_EmptyEnv" | "base")) {
        return format!("<environment: {}>", n);
    }
    if let Some(n) = e.package_env_name() {
        return format!("<environment: {}>", n);
    }
    if let Some(n) = e.namespace_name() {
        return format!("<environment: namespace:{}>", n);
    }
    format!("<environment: 0x{:012x}>", 0x55d5_c000_0000usize + e.id() * 0x38)
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
    // R: printutils.c:GetMatrixDimnames
    let names = dn.as_ref().unwrap().get_attr(s::NAMES);
    let (rn, cn) = match names {
        Some(Value::Str(n)) => (
            Some(n.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into())),
            Some(n.data.get(1).cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into())),
        ),
        _ => (None, None),
    };
    (rl, cl, rn, cn)
}

/// R: print.c PrintGenericVector — an `isObject()` element is summarised by
/// `format()`: a single short string is used verbatim, otherwise the element
/// prints as `"<class>,<length>"`.
fn object_summary(interp: &mut Interp, x: &Value, env: &Env) -> Option<String> {
    if !x.is_object() {
        return None;
    }
    let class_name =
        crate::dispatch::dispatch_class(x).first().map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
    let base = interp.base.clone();
    if let Ok(f) = interp.find_fun(Sym::new("format"), &base) {
        if let Ok(Value::Str(s)) = interp.call_value(&f, vec![Arg::pos(x.clone())], env) {
            if s.data.len() == 1 {
                if let Some(Some(t)) = s.data.first() {
                    if t.len() < 100 {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    Some(format!("{},{}", class_name, x.len()))
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
        // R: print.c PrintGenericVector: a length-one complex is printed like
        // the scalar itself (formatComplex on just that element, no gap).
        Value::Cplx(v) if v.data.len() == 1 => {
            let z = v.data[0];
            if is_na_real(z.re) || is_na_real(z.im) {
                encode_real0(NA_REAL, 2, 0, 0, "NA")
            } else {
                let (r, i) = format_complex(&v.data, 7, 0, 0, 2);
                encode_complex(z, r, i, "NA")
            }
        }
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
