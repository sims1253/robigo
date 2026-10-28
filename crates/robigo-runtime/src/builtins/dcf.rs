//! `read.dcf` (GNU R's dcf.c): Debian Control File records, the format
//! of DESCRIPTION files.

use super::io::{EOF, con_close_raw, con_open, rconn_fgetc};
use super::{arg, internal};
use crate::coerce::as_integer;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static DEFS: &[Builtin] = &[internal("readDCF", do_readdcf, 3)];

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// The start of the trailing run of blanks (`[ \t]+$`), or the length.
fn trailing_blanks(line: &[u8]) -> usize {
    line.iter().rposition(|&b| !is_blank(b)).map_or(0, |p| p + 1)
}

/// `^[ \f\n\r\t\v]+\.[ \f\n\r\t\v]*$`: a continuation line holding only
/// a dot, which stands for an empty line in a folded field.
fn is_eblankline(line: &[u8]) -> bool {
    let space = |b: &u8| matches!(b, b' ' | b'\x0c' | b'\n' | b'\r' | b'\t' | b'\x0b');
    let lead = line.iter().take_while(|b| space(b)).count();
    lead > 0 && line.get(lead) == Some(&b'.') && line[lead + 1..].iter().all(space)
}

/// `^[^:]+:[ \t]*`: the end of the tag and the blanks after it.
fn regline(line: &[u8]) -> Option<usize> {
    let colon = line.iter().position(|&b| b == b':')?;
    if colon == 0 {
        return None;
    }
    Some(colon + 1 + line[colon + 1..].iter().take_while(|&&b| is_blank(b)).count())
}

/// The value after `offset`, without trailing blanks when folding (GNU R
/// writes the terminator at the start of the trailing blanks; when that
/// lies before `offset` the rest of the line is blank).
fn value_from(line: &[u8], offset: usize, fold: bool) -> Vec<u8> {
    let end = if fold { trailing_blanks(line) } else { line.len() };
    if end <= offset { Vec::new() } else { line[offset..end].to_vec() }
}

fn mk(b: &[u8]) -> RStr {
    rstr(&String::from_utf8_lossy(b))
}

fn first20(line: &[u8]) -> String {
    String::from_utf8_lossy(&line[..line.len().min(20)]).into_owned()
}

// R: dcf.c:do_readDCF
fn do_readdcf(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let ncon = as_integer(&arg(&args, 0));
    let wasopen = i.conns.get(ncon)?.isopen;
    if !wasopen {
        if !con_open(i, ncon)? {
            return Err(Flow::error("cannot open the connection"));
        }
    }
    // the C code's context closes the connection on error
    let res = read_dcf(i, ncon, &args);
    if !wasopen {
        con_close_raw(i, ncon)?;
    }
    res
}

fn read_dcf(i: &mut Interp, ncon: i32, args: &[Arg]) -> RResult<Value> {
    if !i.conns.get(ncon)?.canread {
        return Err(Flow::error("cannot read from this connection"));
    }
    let strs = |i: &mut Interp, v: Value| -> RResult<Vec<RStr>> {
        match i.coerce(&v, VType::Str)? {
            Value::Str(s) => Ok(s.data.to_vec()),
            _ => Ok(Vec::new()),
        }
    };
    let mut what: Vec<Vec<u8>> =
        strs(i, arg(args, 1))?.iter().map(|s| s.as_deref().unwrap_or("NA").as_bytes().to_vec()).collect();
    let dynwhat = what.is_empty();
    let fold_excludes: Vec<Vec<u8>> =
        strs(i, arg(args, 2))?.iter().map(|s| s.as_deref().unwrap_or("NA").as_bytes().to_vec()).collect();
    let foldable = |field: &[u8]| !fold_excludes.iter().any(|e| e == field);

    // records[k][m]: field m of record k (the C code's what x nret matrix)
    let mut records: Vec<Vec<RStr>> = vec![vec![None; what.len()]];
    let mut lastm: Option<usize> = None;
    let mut blank_skip = true;
    let mut field_skip = false;
    let mut field_fold = true;
    let mut n_eblanklines = 0usize;
    loop {
        let mut buf = Vec::new();
        let mut c;
        {
            let con = i.conns.get(ncon)?;
            loop {
                c = rconn_fgetc(con);
                if c == EOF || c == b'\n' as i32 {
                    break;
                }
                buf.push(c as u8);
            }
        }
        if buf.is_empty() && c == EOF {
            break;
        }
        // the C string ends at the first nul
        if let Some(p) = buf.iter().position(|&b| b == 0) {
            buf.truncate(p);
        }
        let line = buf;
        let k = records.len() - 1;
        if line.iter().all(|&b| is_blank(b)) {
            // a blank line: the first one after a record ends it
            if !blank_skip {
                records.push(vec![None; what.len()]);
                blank_skip = true;
                lastm = None;
                field_skip = false;
                field_fold = true;
                n_eblanklines = 0;
            }
            continue;
        }
        blank_skip = false;
        let lead = line.iter().take_while(|&&b| is_blank(b)).count();
        if lead > 0 {
            // a continuation line
            if lastm.is_none() && !field_skip {
                return Err(Flow::error(format!(
                    "Found continuation line starting '{} ...' at begin of record.",
                    first20(&line)
                )));
            }
            if let Some(m) = lastm {
                let mut val: Vec<u8> = records[k][m].as_deref().unwrap_or("NA").as_bytes().to_vec();
                let eblank = is_eblankline(&line);
                if eblank && field_fold {
                    n_eblanklines += 1;
                    continue;
                }
                if !val.is_empty() || !field_fold {
                    val.push(b'\n');
                }
                if !eblank {
                    val.extend(std::iter::repeat_n(b'\n', n_eblanklines));
                    n_eblanklines = 0;
                    let offset = if field_fold { lead } else { 0 };
                    val.extend(value_from(&line, offset, field_fold));
                }
                records[k][m] = mk(&val);
            }
            continue;
        }
        let Some(offset) = regline(&line) else {
            return Err(Flow::error(format!("Line starting '{} ...' is malformed!", first20(&line))));
        };
        for (m, w) in what.iter().enumerate() {
            if line.len() > w.len() && line[w.len()] == b':' && line.starts_with(w) {
                // a known field we are recording
                lastm = Some(m);
                field_skip = false;
                if !fold_excludes.is_empty() {
                    field_fold = foldable(w);
                }
                records[k][m] = mk(&value_from(&line, offset, field_fold));
                break;
            }
            // a field, but not one asked for
            lastm = None;
            field_skip = true;
        }
        if dynwhat && lastm.is_none() {
            // a field not seen before, and all fields are recorded
            field_skip = false;
            let tag = line[..line.iter().position(|&b| b == b':').unwrap()].to_vec();
            what.push(tag);
            for r in records.iter_mut() {
                r.push(None);
            }
            let m = what.len() - 1;
            lastm = Some(m);
            if !fold_excludes.is_empty() {
                field_fold = foldable(&what[m]);
            }
            records[k][m] = mk(&value_from(&line, offset, field_fold));
        }
    }
    if blank_skip {
        records.pop();
    }
    // the transposed matrix: one row per record
    let nr = records.len();
    let nc = what.len();
    let mut out: Vec<RStr> = vec![None; nr * nc];
    for (k, r) in records.iter().enumerate() {
        for (m, v) in r.iter().enumerate() {
            out[k + m * nr] = v.clone();
        }
    }
    let mut ans = Value::strs(out);
    ans.set_attr_raw(s::DIM, Value::ints(vec![nr as i32, nc as i32]));
    ans.set_attr_raw(s::DIMNAMES, Value::list(vec![Value::Null, Value::strs(what.iter().map(|w| mk(w)).collect())]));
    Ok(ans)
}
