//! Native routines of the utils package: reading and writing tables
//! (port of GNU R's src/library/utils/src/io.c).

use super::NativeFn;
use crate::builtins::scan::{strtoi, Scan, EOF, NO_COMCHAR};
use crate::coerce::{as_integer, as_logical, is_blank, r_strtod5};
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("readtablehead", |i, _, a, _| readtablehead(i, &a)),
    ("countfields", |i, _, a, _| countfields(i, &a)),
    ("typeconvert", |i, _, a, _| typeconvert(i, &a)),
    ("writetable", |i, _, a, _| writetable(i, &a)),
    ("objectSize", |_, _, a, _| Ok(Value::real(objectsize(&arg(&a, 0)) as f64))),
    ("flushconsole", |i, _, _, _| {
        i.flush();
        Ok(Value::Null)
    }),
    // rr's time-zone code is a port of glibc's (see builtins/datetime).
    ("tzcode_type", |_, _, _, _| Ok(Value::str("system (glibc)"))),
];

/// GNU R's memory footprint of an object on 64-bit platforms: node and
/// vector-cell sizes as R allocates them.
// R: utils/src/size.c:objectsize
fn objectsize(v: &Value) -> usize {
    const SEXPREC: usize = 56;
    const SEXPREC_ALIGN: usize = 48;
    fn vec_cells(vcnt: usize) -> usize {
        SEXPREC_ALIGN
            + if vcnt > 16 {
                8 * vcnt
            } else if vcnt > 8 {
                128
            } else if vcnt > 6 {
                64
            } else if vcnt > 4 {
                48
            } else if vcnt > 2 {
                32
            } else if vcnt > 1 {
                16
            } else if vcnt > 0 {
                8
            } else {
                0
            }
    }
    let byte2vec = |n: usize| if n > 0 { (n - 1) / 8 + 1 } else { 0 };
    let int2vec = |n: usize| if n > 0 { (n * 4 - 1) / 8 + 1 } else { 0 };
    let charsxp = |s: &str| vec_cells(byte2vec(s.len() + 1));
    /// A pairlist node chain: TAG + CAR + node per element.
    fn pairlist(items: &[Arg]) -> usize {
        items
            .iter()
            .map(|a| a.tag.map(|_| SEXPREC).unwrap_or(0) + objectsize(&a.value) + SEXPREC)
            .sum()
    }
    let attrs = |v: &Value| -> usize {
        match v.attrs() {
            Some(a) => a.0.iter().map(|(_, val)| SEXPREC + objectsize(val) + SEXPREC).sum(),
            None => 0,
        }
    };
    match v {
        Value::Null => 0,
        Value::Sym(_) => SEXPREC + attrs(v),
        Value::Lgl(x) | Value::Int(x) => vec_cells(int2vec(x.data.len())) + attrs(v),
        Value::Real(x) => vec_cells(x.data.len()) + attrs(v),
        Value::Cplx(x) => vec_cells(2 * x.data.len()) + attrs(v),
        Value::Raw(x) => vec_cells(byte2vec(x.data.len())) + attrs(v),
        Value::Str(x) => {
            let mut seen: Vec<&str> = Vec::new();
            let mut cnt = 0;
            for e in x.data.iter().flatten() {
                if !seen.contains(&&**e) {
                    seen.push(e);
                    cnt += charsxp(e);
                }
            }
            cnt + vec_cells(x.data.len()) + attrs(v)
        }
        Value::List(x) | Value::Expr(x) => x.data.iter().map(objectsize).sum::<usize>() + vec_cells(x.data.len()) + attrs(v),
        Value::Pairlist(p) => pairlist(&p.items),
        Value::Lang(l) => objectsize(&l.fun) + SEXPREC + pairlist(&l.args) + attrs(v),
        Value::Closure(c) => {
            let f: usize = c.formals.iter().map(|a| SEXPREC + objectsize(&a.value) + SEXPREC).sum();
            f + objectsize(&c.body) + SEXPREC + attrs(v)
        }
        _ => SEXPREC + attrs(v),
    }
}

fn arg(a: &[Arg], k: usize) -> Value {
    a.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null)
}

fn str1(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) if !s.data.is_empty() => Some(s.data[0].as_deref().unwrap_or("NA").to_string()),
        _ => None,
    }
}

fn comment_char(v: &Value) -> RResult<i32> {
    match v {
        Value::Str(c) if c.data.len() == 1 => {
            let p = c.data[0].as_deref().unwrap_or("NA").as_bytes();
            if p.len() > 1 {
                return Err(Flow::error("invalid 'comment.char' argument"));
            }
            Ok(p.first().map(|&b| b as i32).unwrap_or(NO_COMCHAR))
        }
        _ => Err(Flow::error("invalid 'comment.char' argument")),
    }
}

fn sep_char(v: &Value) -> RResult<i32> {
    match v {
        Value::Null => Ok(0),
        Value::Str(s) if s.data.is_empty() => Ok(0),
        Value::Str(s) => Ok(s.data[0].as_deref().unwrap_or("NA").as_bytes().first().map(|&b| b as i32).unwrap_or(0)),
        _ => Err(Flow::error("invalid 'sep' argument")),
    }
}

fn quote_set(v: &Value) -> RResult<Vec<u8>> {
    match v {
        Value::Str(q) => Ok(q.data.first().and_then(|x| x.as_deref()).unwrap_or("").as_bytes().to_vec()),
        Value::Null => Ok(vec![]),
        _ => Err(Flow::error("invalid quote symbol set")),
    }
}

/// Open connection `con` for reading unless it is; returns whether it was.
fn open_for_read(i: &mut Interp, con: i32, check_read: bool) -> RResult<bool> {
    use crate::builtins::io;
    let (wasopen, ..) = io::conn_state(i, con)?;
    if !wasopen {
        if !io::open_in_mode(i, con, "r")? {
            return Err(Flow::error("cannot open the connection"));
        }
        if check_read && !io::conn_state(i, con)?.1 {
            io::close_conn(i, con)?;
            return Err(Flow::error("cannot read from this connection"));
        }
    } else if check_read && !io::conn_state(i, con)?.1 {
        return Err(Flow::error("cannot read from this connection"));
    }
    Ok(wasopen)
}

fn rspace(c: i32) -> bool {
    c == ' ' as i32 || c == '\t' as i32 || c == '\n' as i32 || c == '\r' as i32
}

// R: utils/src/io.c:readtablehead
pub fn readtablehead(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    use crate::builtins::io;
    let con = as_integer(&arg(a, 0));
    let nlines = as_integer(&arg(a, 1));
    let comchar = comment_char(&arg(a, 2))?;
    let blskip = as_logical(&arg(a, 3));
    let blskip = blskip == NA_LOGICAL || blskip != 0;
    let quoteset = quote_set(&arg(a, 4))?;
    let sepchar = sep_char(&arg(a, 5))?;
    let skip_nul = as_logical(&arg(a, 6));
    if nlines <= 0 || nlines == NA_INTEGER {
        return Err(Flow::error("invalid 'nlines' argument"));
    }
    if skip_nul == NA_LOGICAL {
        return Err(Flow::error("invalid 'skipNul' argument"));
    }
    let mut d = Scan::new(con);
    d.comchar = comchar;
    d.quoteset = quoteset;
    d.sepchar = sepchar;
    d.skip_nul = skip_nul != 0;
    let wasopen = open_for_read(i, con, false)?;
    let mut ans: Vec<RStr> = Vec::new();
    let mut quote = 0;
    let mut incomplete = false;
    let mut warnings: Vec<String> = vec![];
    'lines: while (ans.len() as i32) < nlines {
        let mut buf: Vec<u8> = Vec::new();
        let (mut empty, mut skip, mut firstnonwhite) = (true, false, true);
        let mut c;
        loop {
            c = d.scanchar(i, true)?;
            if c == EOF {
                break;
            }
            if quote != 0 {
                if d.sepchar == 0 && c == '\\' as i32 {
                    buf.push(c as u8);
                    c = d.scanchar(i, true)?;
                    if c == EOF {
                        if !wasopen {
                            io::close_conn(i, con)?;
                        }
                        return Err(Flow::error("\\ followed by EOF"));
                    }
                    buf.push(c as u8);
                    continue;
                } else if c == quote {
                    if d.sepchar == 0 {
                        quote = 0;
                    } else {
                        let c2 = d.scanchar(i, true)?;
                        if c2 == quote {
                            buf.push(c as u8);
                        } else {
                            d.unscanchar(c2);
                            quote = 0;
                        }
                    }
                }
            } else if !skip && (firstnonwhite || d.sepchar != 0) && d.is_quote(c) {
                quote = c;
            } else if !skip && d.sepchar == 0 && rspace(c) {
                firstnonwhite = true;
            } else if c != ' ' as i32 && c != '\t' as i32 {
                firstnonwhite = false;
            }
            if empty && !skip && c != '\n' as i32 && c != d.comchar {
                empty = false;
            }
            if quote == 0 && !skip && c == d.comchar {
                skip = true;
            }
            if quote != 0 || c != '\n' as i32 {
                buf.push(c as u8);
            } else {
                break;
            }
        }
        if d.ttyflag && empty {
            incomplete = !buf.is_empty();
            break 'lines;
        }
        if !empty || (c != EOF && !blskip) {
            if buf.contains(&0) {
                warnings.push(format!("line {} appears to contain embedded nulls", ans.len() + 1));
            }
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            ans.push(rstr(&String::from_utf8_lossy(&buf[..end])));
        }
        if c == EOF {
            incomplete = !buf.is_empty();
            break 'lines;
        }
    }
    if !wasopen {
        io::close_conn(i, con)?;
    }
    for w in warnings {
        i.warning(w)?;
    }
    if incomplete {
        let desc = io::connection_description(i, con)?;
        i.warning(format!("incomplete final line found by readTableHeader on '{}'", desc))?;
    }
    Ok(Value::strs(ans))
}

// R: utils/src/io.c:countfields
pub fn countfields(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    use crate::builtins::io;
    let con = as_integer(&arg(a, 0));
    let sepchar = sep_char(&arg(a, 1))?;
    let quoteset = quote_set(&arg(a, 2))?;
    let mut nskip = as_integer(&arg(a, 3));
    let blskip = as_logical(&arg(a, 4));
    let comchar = comment_char(&arg(a, 5))?;
    if nskip < 0 || nskip == NA_INTEGER {
        nskip = 0;
    }
    let blskip = blskip == NA_LOGICAL || blskip != 0;
    let mut d = Scan::new(con);
    d.comchar = comchar;
    d.quoteset = quoteset;
    d.sepchar = sepchar;
    let wasopen = if con == 0 { true } else { open_for_read(i, con, true)? };
    if con != 0 {
        for _ in 0..nskip {
            loop {
                let c = d.scanchar(i, false)?;
                if c == '\n' as i32 || c == EOF {
                    break;
                }
            }
        }
    }
    let mut ans: Vec<i32> = Vec::new();
    let (mut nfields, mut inquote, mut quote) = (0i32, 0i32, 0i32);
    let mut nlines: i64 = 0;
    let set = |ans: &mut Vec<i32>, k: i64, v: i32| {
        let k = k as usize;
        if ans.len() <= k {
            ans.resize(k + 1, 0);
        }
        ans[k] = v;
    };
    loop {
        let mut c = d.scanchar(i, inquote > 0)?;
        if c == EOF {
            if nfields != 0 {
                set(&mut ans, nlines, nfields);
            } else {
                nlines -= 1;
            }
            break;
        } else if c == '\n' as i32 {
            if inquote != 0 {
                set(&mut ans, nlines, NA_INTEGER);
                nlines += 1;
            } else if nfields != 0 || !blskip {
                set(&mut ans, nlines, nfields);
                nlines += 1;
                nfields = 0;
                inquote = 0;
            }
            continue;
        } else if d.sepchar != 0 {
            if nfields == 0 {
                nfields += 1;
            }
            if inquote != 0 && c == quote {
                inquote = 0;
            } else if d.is_quote(c) {
                inquote = nlines as i32 + 1;
                quote = c;
            }
            if c == d.sepchar && inquote == 0 {
                nfields += 1;
            }
        } else if !rspace(c) {
            if d.is_quote(c) {
                quote = c;
                inquote = nlines as i32 + 1;
                loop {
                    c = d.scanchar(i, inquote > 0)?;
                    if c == quote {
                        break;
                    }
                    if c == EOF {
                        if !wasopen {
                            io::close_conn(i, con)?;
                        }
                        return Err(Flow::error(format!("quoted string on line {} terminated by EOF", inquote)));
                    } else if c == '\n' as i32 {
                        set(&mut ans, nlines, NA_INTEGER);
                        nlines += 1;
                    }
                }
                inquote = 0;
            } else {
                loop {
                    c = d.scanchar(i, false)?;
                    if rspace(c) || c == EOF {
                        break;
                    }
                }
                if c == EOF {
                    c = '\n' as i32;
                }
                d.unscanchar(c);
            }
            nfields += 1;
        }
    }
    if d.save != 0 && !d.ttyflag && wasopen {
        io::conn_pushback(i, con, &[d.save as u8])?;
    }
    if !wasopen {
        io::close_conn(i, con)?;
    }
    if nlines < 0 {
        return Ok(Value::Null);
    }
    ans.resize(nlines as usize + 1, 0);
    Ok(Value::ints(ans))
}

/// The types a field could still be converted to.
struct Possible {
    logical: bool,
    integer: bool,
    real: bool,
    complex: bool,
}

/// utils' `strtoc`: (value, end), with NA components for malformed input.
fn strtoc(s: &[u8], dec: u8, na: bool, exact: i32) -> (Complex, usize) {
    let (x, endp) = r_strtod5(s, dec, na, exact);
    let rest = &s[endp..];
    if rest.iter().all(|c| c.is_ascii_whitespace()) {
        (Complex::new(x, 0.0), endp)
    } else if rest.first() == Some(&b'i') {
        if endp == 0 { (Complex::new(NA_REAL, NA_REAL), endp) } else { (Complex::new(0.0, x), endp + 1) }
    } else {
        let (y, e2) = r_strtod5(rest, dec, na, exact);
        if rest.get(e2) == Some(&b'i') {
            (Complex::new(x, y), endp + e2 + 1)
        } else {
            (Complex::new(NA_REAL, NA_REAL), 0)
        }
    }
}

fn blank_from(s: &[u8], k: usize) -> bool {
    s[k.min(s.len())..].iter().all(|c| c.is_ascii_whitespace())
}

// R: utils/src/io.c:ruleout_types
fn ruleout_types(s: &str, p: &mut Possible, dec: u8, exact: i32) {
    if p.logical {
        if matches!(s, "F" | "T" | "FALSE" | "TRUE") {
            p.integer = false;
            p.real = false;
            p.complex = false;
            return;
        }
        p.logical = false;
    }
    if p.integer && strtoi(s) == NA_INTEGER {
        p.integer = false;
    }
    if p.real {
        let (_, e) = r_strtod5(s.as_bytes(), dec, true, exact);
        if !blank_from(s.as_bytes(), e) {
            p.real = false;
        }
    }
    if p.complex {
        let (_, e) = strtoc(s.as_bytes(), dec, true, exact);
        if !blank_from(s.as_bytes(), e) {
            p.complex = false;
        }
    }
}

// R: utils/src/io.c:typeconvert
pub fn typeconvert(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let cvec_v = arg(a, 0);
    let Value::Str(cvec) = &cvec_v else {
        return Err(Flow::error("the first argument must be of mode character"));
    };
    let Value::Str(nas) = arg(a, 1) else { return Err(Flow::error("invalid 'na.strings' argument")) };
    let mut d = Scan::new(-1);
    d.na_strings = nas.data.clone();
    let as_is = as_logical(&arg(a, 2));
    let as_is = as_is != NA_LOGICAL && as_is != 0;
    let dec = match arg(a, 3) {
        Value::Str(s) if !s.data.is_empty() => s.data[0].as_deref().unwrap_or(".").as_bytes().first().copied().unwrap_or(0),
        _ => b'.',
    };
    let (i_exact, exact) = match str1(&arg(a, 4)).as_deref() {
        Some("allow.loss") | None => (0, 0),
        Some("warn.loss") => (NA_INTEGER, 0),
        Some("no.loss") => (1, 1),
        Some(t) => return Err(Flow::error(format!("invalid 'numerals' string: \"{}\"", t))),
    };
    let try_logical = as_logical(&arg(a, 5));
    let mut p = Possible { logical: try_logical != NA_LOGICAL && try_logical != 0, integer: true, real: true, complex: true };
    let is_na = |e: &RStr| match e {
        None => true,
        Some(t) => t.is_empty() || d.is_na_string(t, 1) || is_blank(t),
    };
    if let Some(first) = cvec.data.iter().find(|e| !is_na(e)) {
        ruleout_types(first.as_deref().unwrap(), &mut p, dec, exact);
    }
    let mut rval: Option<Value> = None;
    if p.logical {
        let mut out = Vec::with_capacity(cvec.data.len());
        for e in &cvec.data {
            if is_na(e) {
                out.push(NA_LOGICAL);
                continue;
            }
            match e.as_deref().unwrap() {
                "F" | "FALSE" => out.push(0),
                "T" | "TRUE" => out.push(1),
                t => {
                    p.logical = false;
                    ruleout_types(t, &mut p, dec, exact);
                    break;
                }
            }
        }
        if p.logical {
            rval = Some(Value::lgls(out));
        }
    }
    if rval.is_none() && p.integer {
        let mut out = Vec::with_capacity(cvec.data.len());
        for e in &cvec.data {
            if is_na(e) {
                out.push(NA_INTEGER);
                continue;
            }
            let t = e.as_deref().unwrap();
            let x = strtoi(t);
            if x == NA_INTEGER {
                p.integer = false;
                ruleout_types(t, &mut p, dec, exact);
                break;
            }
            out.push(x);
        }
        if p.integer {
            rval = Some(Value::ints(out));
        }
    }
    if rval.is_none() && p.real {
        let mut out = Vec::with_capacity(cvec.data.len());
        for e in &cvec.data {
            if is_na(e) {
                out.push(NA_REAL);
                continue;
            }
            let t = e.as_deref().unwrap();
            let (x, endp) = r_strtod5(t.as_bytes(), dec, false, i_exact);
            if i_exact == NA_INTEGER && x.abs() > f64::from_bits(0x433f_ffff_ffff_ffff) && x.is_finite() {
                i.warning(format!("accuracy loss in conversion from \"{}\" to numeric", t))?;
            }
            if !blank_from(t.as_bytes(), endp) {
                p.real = false;
                ruleout_types(t, &mut p, dec, exact);
                break;
            }
            out.push(x);
        }
        if p.real {
            rval = Some(Value::reals(out));
        }
    }
    if rval.is_none() && p.complex {
        let mut out = Vec::with_capacity(cvec.data.len());
        for e in &cvec.data {
            if is_na(e) {
                out.push(Complex::new(NA_REAL, NA_REAL));
                continue;
            }
            let t = e.as_deref().unwrap();
            let (z, endp) = strtoc(t.as_bytes(), dec, false, i_exact);
            if !blank_from(t.as_bytes(), endp) {
                p.complex = false;
                break;
            }
            out.push(z);
        }
        if p.complex {
            rval = Some(Value::Cplx(std::rc::Rc::new(Vector::new(out))));
        }
    }
    let mut rval = match rval {
        Some(v) => v,
        None if as_is => Value::strs(
            cvec.data.iter().map(|e| if e.as_deref().is_some_and(|t| d.is_na_string(t, 1)) { None } else { e.clone() }).collect(),
        ),
        None => {
            // A factor with the non-NA values as levels, sorted.
            let mut levs: Vec<RStr> = Vec::new();
            for e in &cvec.data {
                let Some(t) = e else { continue };
                if !d.is_na_string(t, 1) && !levs.contains(e) {
                    levs.push(e.clone());
                }
            }
            let mut sorted_v = Value::strs(levs);
            crate::builtins::sort::sort_vector(&mut sorted_v, false)?;
            let Value::Str(sorted) = &sorted_v else { unreachable!() };
            let sorted = sorted.data.clone();
            let codes: Vec<i32> = cvec
                .data
                .iter()
                .map(|e| sorted.iter().position(|l| l == e).map(|k| k as i32 + 1).unwrap_or(NA_INTEGER))
                .collect();
            let mut f = Value::ints(codes);
            f.set_attr_raw(s::LEVELS, Value::strs(sorted));
            f.set_attr_raw(s::CLASS, Value::str("factor"));
            f
        }
    };
    if let Some(dims) = cvec_v.get_attr(s::DIM) {
        rval.set_attr_raw(s::DIM, dims);
        if let Some(dn) = cvec_v.get_attr(s::DIMNAMES) {
            rval.set_attr_raw(s::DIMNAMES, dn);
        }
    } else if let Some(n) = cvec_v.get_attr(s::NAMES) {
        rval.set_attr_raw(s::NAMES, n);
    }
    Ok(rval)
}

/// One element for write.table: strings quoted with doubled or escaped
/// quotes, numbers at 15 significant digits.
// R: utils/src/io.c:EncodeElement2, printutils.c:EncodeElement0
fn encode_element2(i: &Interp, x: &Value, k: usize, quote: bool, qmethod: bool, dec: &str) -> String {
    use crate::format::*;
    match x {
        Value::Str(v) => {
            let p0 = v.data[k].as_deref().unwrap_or("NA");
            if !quote {
                return p0.to_string();
            }
            let mut out = String::with_capacity(p0.len() + 2);
            out.push('"');
            for c in p0.chars() {
                if c == '"' {
                    out.push(if qmethod { '\\' } else { '"' });
                }
                out.push(c);
            }
            out.push('"');
            out
        }
        Value::Lgl(v) => {
            let w = format_logical(&v.data[k..=k], 2);
            encode_logical(v.data[k], w, "NA")
        }
        Value::Int(v) => {
            let w = format_integer(&v.data[k..=k], 2);
            encode_integer(v.data[k], w, "NA")
        }
        Value::Real(v) => {
            let scipen = i.option_int("scipen", 0);
            let (w, d, e) = format_real(&v.data[k..=k], 15, scipen, 0, 2);
            let s = encode_real0(v.data[k], w, d, e, "NA");
            if dec != "." { s.replacen('.', dec, 1) } else { s }
        }
        Value::Cplx(v) => {
            let scipen = i.option_int("scipen", 0);
            let (wr, wi) = format_complex(&v.data[k..=k], 15, scipen, 0, 2);
            let s = encode_complex(v.data[k], wr, wi, "NA");
            if dec != "." { s.replace('.', dec) } else { s }
        }
        Value::Raw(v) => format!("{:02x}", v.data[k]),
        _ => String::new(),
    }
}

fn is_na_at(x: &Value, k: usize) -> bool {
    match x {
        Value::Lgl(v) | Value::Int(v) => v.data[k] == NA_INTEGER,
        Value::Real(v) => v.data[k].is_nan(),
        Value::Str(v) => v.data[k].is_none(),
        Value::Cplx(v) => v.data[k].re.is_nan() || v.data[k].im.is_nan(),
        _ => false,
    }
}

// R: utils/src/io.c:writetable
pub fn writetable(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    use crate::builtins::io;
    let x = arg(a, 0);
    let file = arg(a, 1);
    if !file.inherits("connection") {
        return Err(Flow::error("'file' is not a connection"));
    }
    let con = as_integer(&file);
    let (wasopen, _, canwrite, _) = io::conn_state(i, con)?;
    if !canwrite {
        return Err(Flow::error("cannot write to this connection"));
    }
    if !wasopen && !io::open_in_mode(i, con, "wt")? {
        return Err(Flow::error("cannot open the connection"));
    }
    let nr = as_integer(&arg(a, 2));
    let nc = as_integer(&arg(a, 3));
    let rnames = arg(a, 4);
    let (sep, eol, na, dec) = (arg(a, 5), arg(a, 6), arg(a, 7), arg(a, 8));
    let quote = arg(a, 9);
    let qmethod = as_logical(&arg(a, 10));
    if nr == NA_INTEGER {
        return Err(Flow::error("invalid 'nr' argument"));
    }
    if nc == NA_INTEGER {
        return Err(Flow::error("invalid 'nc' argument"));
    }
    if !rnames.is_null() && !matches!(rnames, Value::Str(_)) {
        return Err(Flow::error("invalid 'rnames' argument"));
    }
    let (Some(csep), Some(ceol), Some(cna), Some(sdec)) = (str1(&sep), str1(&eol), str1(&na), str1(&dec)) else {
        let which = if str1(&sep).is_none() {
            "sep"
        } else if str1(&eol).is_none() {
            "eol"
        } else if str1(&na).is_none() {
            "na"
        } else {
            "dec"
        };
        return Err(Flow::error(format!("invalid '{}' argument", which)));
    };
    if qmethod == NA_LOGICAL {
        return Err(Flow::error("invalid 'qmethod' argument"));
    }
    let qmethod = qmethod != 0;
    if sdec.len() != 1 {
        return Err(Flow::error("'dec' must be a single character"));
    }
    let (nr, nc) = (nr as usize, nc as usize);
    let mut quote_col = vec![false; nc];
    let mut quote_rn = false;
    for q in crate::coerce::to_int(&quote, &mut Default::default()) {
        if q == 0 {
            quote_rn = true;
        }
        if q > 0 {
            quote_col[q as usize - 1] = true;
        }
    }
    let mut out = String::new();
    let res = (|| -> RResult<()> {
        if let Value::List(cols) = &x {
            let mut levels: Vec<Option<Value>> = Vec::with_capacity(nc);
            for (j, xj) in cols.data.iter().enumerate().take(nc) {
                if xj.len() != nr {
                    return Err(Flow::error(format!(
                        "corrupt data frame -- length of column {} does not match nrows",
                        j + 1
                    )));
                }
                levels.push(if xj.inherits("factor") { xj.get_attr(s::LEVELS) } else { None });
            }
            for r in 0..nr {
                if !rnames.is_null() {
                    out.push_str(&encode_element2(i, &rnames, r, quote_rn, qmethod, &sdec));
                    out.push_str(&csep);
                }
                for j in 0..nc {
                    let xj = &cols.data[j];
                    if j > 0 {
                        out.push_str(&csep);
                    }
                    if is_na_at(xj, r) {
                        out.push_str(&cna);
                    } else if let Some(levs) = &levels[j] {
                        let code = match xj {
                            Value::Int(v) => v.data[r] as i64 - 1,
                            Value::Real(v) => (v.data[r] - 1.0) as i64,
                            _ => {
                                return Err(Flow::error(format!(
                                    "column {} claims to be a factor but does not have numeric codes",
                                    j + 1
                                )));
                            }
                        };
                        if code < 0 || code as usize >= levs.len() {
                            return Err(Flow::error("index out of range"));
                        }
                        out.push_str(&encode_element2(i, levs, code as usize, quote_col[j], qmethod, &sdec));
                    } else {
                        out.push_str(&encode_element2(i, xj, r, quote_col[j], qmethod, &sdec));
                    }
                }
                out.push_str(&ceol);
            }
        } else {
            if !x.is_atomic() {
                return Err(Flow::error(format!("unimplemented type '{}' in 'write.table, matrix method'\n", x.type_name())));
            }
            if x.len() != nr * nc {
                return Err(Flow::error("corrupt matrix -- dims do not match length"));
            }
            for r in 0..nr {
                if !rnames.is_null() {
                    out.push_str(&encode_element2(i, &rnames, r, quote_rn, qmethod, &sdec));
                    out.push_str(&csep);
                }
                for j in 0..nc {
                    if j > 0 {
                        out.push_str(&csep);
                    }
                    let k = r + j * nr;
                    if is_na_at(&x, k) {
                        out.push_str(&cna);
                    } else {
                        out.push_str(&encode_element2(i, &x, k, quote_col[j], qmethod, &sdec));
                    }
                }
                out.push_str(&ceol);
            }
        }
        Ok(())
    })();
    io::conn_write(i, con, &out)?;
    if !wasopen {
        io::close_conn(i, con)?;
    }
    res.map(|_| Value::Null)
}
