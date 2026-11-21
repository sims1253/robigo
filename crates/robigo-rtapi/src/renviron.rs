//! Renviron.c's start-up reading of the `Renviron` files (L5): the system
//! one (`R_HOME/etc/Renviron`), then, unless `--no-environ` (`--vanilla`),
//! the site one (`R_ENVIRON`, else `R_HOME/etc/Renviron.site`) and the
//! user's (`R_ENVIRON_USER`, else `./.Renviron`, else `~/.Renviron`). Each
//! line `NAME=VALUE`, its `${VAR-default}` (`${VAR:-default}`) terms
//! expanded from the environment, is put in the environment. GNU R reads
//! them in `Rf_initialize_R`, before the locale and before R runs: its
//! warnings are `R_ShowMessage`'s, which this early reach stdout (as the
//! oracle shows; the console is not set up yet). robigo-rt reads
//! them in the same place, before the isolate exists, so the writes are
//! the process's own `setenv` (no other thread yet).

use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;

/// fgets' buffer (`BUF_SIZE`): a line longer than this, less one, is read
/// in pieces.
const BUF_SIZE: usize = 100000;
/// The warning's buffer (`MSG_SIZE`).
const MSG_SIZE: usize = 2048;

/// `isspace` in the C locale (the locale is not set up yet).
fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `rmspace`: `s` without leading and trailing white space.
// R: Renviron.c:rmspace
fn rmspace(s: &[u8]) -> &[u8] {
    let end = s.iter().rposition(|&b| !space(b)).map_or(0, |i| i + 1);
    let s = &s[..end];
    let start = s.iter().position(|&b| !space(b)).unwrap_or(s.len());
    &s[start..]
}

/// `getenv(name)`.
fn getenv(name: &[u8]) -> Option<Vec<u8>> {
    if name.contains(&0) || name.contains(&b'=') || name.is_empty() {
        return None;
    }
    std::env::var_os(OsStr::from_bytes(name)).map(|v| v.into_encoded_bytes())
}

/// `subterm(s)`: a `${...}` term's value: the variable's if set (and, for
/// `:-`, not empty), else the default's, itself expanded so.
// R: Renviron.c:subterm
fn subterm(s: &[u8]) -> Vec<u8> {
    if !s.starts_with(b"${") || s.last() != Some(&b'}') {
        return s.to_vec();
    }
    let s = rmspace(&s[2..s.len() - 1]);
    if s.is_empty() {
        return Vec::new();
    }
    let (name, rest, colon) = match s.iter().position(|&b| b == b'-') {
        Some(p) if p > 1 && s[p - 1] == b':' => (&s[..p - 1], Some(&s[p + 1..]), true),
        Some(p) => (&s[..p], Some(&s[p + 1..]), false),
        None => (s, None, false),
    };
    match getenv(name) {
        Some(v) if !colon || !v.is_empty() => v,
        _ => rest.map_or_else(Vec::new, subterm),
    }
}

/// `findRbrace(s)`: the `}` that closes the `{` before `s`.
// R: Renviron.c:findRbrace
fn find_rbrace(s: &[u8]) -> Option<usize> {
    let (mut nl, mut nr, mut p) = (0, 0, 0);
    let mut pr = 0;
    while nr <= nl {
        let pl = s[p..].iter().position(|&b| b == b'{').map(|i| p + i);
        pr = s[p..].iter().position(|&b| b == b'}').map(|i| p + i)?;
        match pl {
            Some(l) if l < pr => {
                p = l + 1;
                nl += 1;
            }
            _ => {
                p = pr + 1;
                nr += 1;
            }
        }
    }
    Some(pr)
}

/// `findterm(s)`: `s` with each `${...}` replaced by its value (`s` as it
/// is when the result would pass the buffer).
// R: Renviron.c:findterm
fn findterm(s: &[u8]) -> Vec<u8> {
    let orig = s;
    let mut s = s;
    let mut ans = Vec::new();
    loop {
        let Some(p) = s.iter().position(|&b| b == b'$') else { break };
        if s.get(p + 1) != Some(&b'{') {
            break;
        }
        let Some(q) = find_rbrace(&s[p + 2..]).map(|q| p + 2 + q) else { break };
        ans.extend_from_slice(&s[..p]);
        let r2 = subterm(&s[p..=q]);
        if ans.len() + r2.len() >= BUF_SIZE {
            return orig.to_vec();
        }
        ans.extend_from_slice(&r2);
        s = &s[q + 1..];
    }
    if ans.len() + s.len() >= BUF_SIZE {
        return orig.to_vec();
    }
    ans.extend_from_slice(s);
    ans
}

/// `Putenv(a, b)`'s value: quotes around sections removed (a quote after
/// a backslash kept), outside them a backslash dropped (a doubled one kept
/// once, one before a newline dropped with it), inside them a backslash
/// before the quote character dropped.
// R: Renviron.c:Putenv
fn putenv_value(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let (mut inquote, mut quote) = (false, 0u8);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let prev = if i > 0 { b[i - 1] } else { 0 };
        if !inquote && (c == b'"' || c == b'\'') && (i == 0 || prev != b'\\') {
            inquote = true;
            quote = c;
        } else if inquote && c == quote && prev != b'\\' {
            inquote = false;
        } else if !inquote && c == b'\\' {
            match b.get(i + 1) {
                Some(b'\n') => i += 1,
                Some(b'\\') => out.push(c),
                _ => {}
            }
        } else if !(inquote && c == b'\\' && b.get(i + 1) == Some(&quote)) {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// `setenv(name, value, 1)`; whether it worked.
fn setenv(name: &[u8], value: &[u8]) -> bool {
    let (Ok(n), Ok(v)) = (CString::new(name), CString::new(value)) else { return false };
    // SAFETY: start-up, before any other thread exists (the front end's
    // `run`, before the isolate): nothing reads the environment at the same
    // time; both strings are NUL-terminated.
    #[allow(unsafe_code)]
    unsafe {
        libc::setenv(n.as_ptr(), v.as_ptr(), 1) == 0
    }
}

/// `Renviron_warning` before R runs: `R_ShowMessage`, the message and a
/// newline, on stdout this early.
fn warn(msg: &[u8]) {
    use std::io::Write;
    let mut e = std::io::stdout().lock();
    let _ = e.write_all(msg);
    let _ = e.write_all(b"\n");
    let _ = e.flush();
}

/// `process_Renviron(filename)`: each line of the file put in the
/// environment; whether the file could be opened.
// R: Renviron.c:process_Renviron
pub fn process_renviron(filename: &[u8]) -> bool {
    let Ok(data) = std::fs::read(OsStr::from_bytes(filename)) else { return false };
    let line_prefix: &[u8] = b"\n      ";
    let ignored_msg: &[u8] = b"\n   They were ignored\n";
    let truncated_msg: &[u8] = b"[... truncated]";
    let too_long: &[u8] = b" (too long)";
    let mut msg: Vec<u8> = Vec::new();
    let mut errs = false;
    // fgets' pieces: up to and with a newline, at most BUF_SIZE - 1 bytes
    let mut pos = 0;
    let next = |pos: &mut usize| -> Option<(&[u8], bool)> {
        if *pos >= data.len() {
            return None;
        }
        let rest = &data[*pos..];
        let max = (BUF_SIZE - 1).min(rest.len());
        let n = rest[..max].iter().position(|&b| b == b'\n').map_or(max, |i| i + 1);
        let piece = &rest[..n];
        *pos += n;
        // feof(fp) once the piece reached the end; a piece with a newline
        let complete = *pos >= data.len() || piece.contains(&b'\n');
        Some((piece, complete))
    };
    while let Some((piece, mut complete)) = next(&mut pos) {
        // (embedded NULs are not supported: the C string ends there)
        let piece = &piece[..piece.iter().position(|&b| b == 0).unwrap_or(piece.len())];
        let s = rmspace(piece);
        if s.is_empty() || s[0] == b'#' {
            continue;
        }
        let eq = s.iter().position(|&b| b == b'=');
        if eq.is_none() || !complete {
            if !errs {
                errs = true;
                msg = format!("\n   File {} contains invalid line(s)", String::from_utf8_lossy(filename)).into_bytes();
                msg.truncate(MSG_SIZE - 1);
            }
            if msg.len() + line_prefix.len() + s.len() + ignored_msg.len() < MSG_SIZE {
                msg.extend_from_slice(line_prefix);
                msg.extend_from_slice(s);
            } else if msg.len() + line_prefix.len() + 45 + truncated_msg.len() + ignored_msg.len() < MSG_SIZE {
                msg.extend_from_slice(line_prefix);
                msg.extend_from_slice(&s[..s.len().min(45)]);
                msg.extend_from_slice(truncated_msg);
            }
            if !complete {
                if msg.len() + too_long.len() + ignored_msg.len() < MSG_SIZE {
                    msg.extend_from_slice(too_long);
                }
                // skip the rest of the line
                while !complete {
                    match next(&mut pos) {
                        Some((_, c)) => complete = c,
                        None => break,
                    }
                }
            }
            continue;
        }
        let eq = eq.unwrap();
        let lhs = rmspace(&s[..eq]);
        let rhs = findterm(rmspace(&s[eq + 1..]));
        if !lhs.is_empty() && !rhs.is_empty() && !setenv(lhs, &putenv_value(&rhs)) {
            warn(format!("problem in setting variable '{}' in Renviron", String::from_utf8_lossy(lhs)).as_bytes());
        }
    }
    if errs {
        if msg.len() + ignored_msg.len() < MSG_SIZE {
            msg.extend_from_slice(ignored_msg);
        }
        warn(&msg);
    }
    true
}

/// `process_system_Renviron`: `R_HOME/etc/Renviron`, which must be there.
// R: Renviron.c:process_system_Renviron
pub fn process_system_renviron(home: &std::path::Path) {
    let mut f = home.as_os_str().as_bytes().to_vec();
    f.extend_from_slice(b"/etc/Renviron");
    if f.len() + 1 > libc::PATH_MAX as usize {
        warn(b"path to system Renviron is too long: skipping");
        return;
    }
    if !process_renviron(&f) {
        warn(b"cannot find system Renviron");
    }
}

/// `process_site_Renviron`: `R_ENVIRON` (when set; empty reads nothing),
/// else `R_HOME/etc/Renviron.site`.
// R: Renviron.c:process_site_Renviron
pub fn process_site_renviron(home: &std::path::Path) {
    if let Some(p) = getenv(b"R_ENVIRON") {
        if !p.is_empty() {
            process_renviron(&p);
        }
        return;
    }
    let mut f = home.as_os_str().as_bytes().to_vec();
    f.extend_from_slice(b"/etc/Renviron.site");
    if f.len() + 1 > libc::PATH_MAX as usize {
        warn(b"path to Renviron.site is too long: skipping");
        return;
    }
    process_renviron(&f);
}

/// `process_user_Renviron`: `R_ENVIRON_USER` (when set; empty reads
/// nothing; `~` expanded), else `./.Renviron`, else `~/.Renviron`.
// R: Renviron.c:process_user_Renviron
pub fn process_user_renviron(expand: impl Fn(&[u8]) -> Vec<u8>) {
    if let Some(s) = getenv(b"R_ENVIRON_USER") {
        if !s.is_empty() {
            process_renviron(&expand(&s));
        }
        return;
    }
    if process_renviron(b".Renviron") {
        return;
    }
    process_renviron(&expand(b"~/.Renviron"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_and_values_are_renviron_c_s() {
        assert_eq!(rmspace(b"  a b \t\n"), b"a b");
        assert_eq!(putenv_value(b"'a b'"), b"a b");
        assert_eq!(putenv_value(br#""x\"y""#), br#"x"y"#);
        assert_eq!(putenv_value(br"a\\b\c"), br"a\bc");
        // an unset variable's default, nested; ':-' for an empty one
        assert_eq!(findterm(b"${ROBIGO_RENVIRON_UNSET_X-${ROBIGO_RENVIRON_UNSET_Y-'d'}}/z"), b"'d'/z");
        assert_eq!(findterm(b"${ROBIGO_RENVIRON_UNSET_X:-v}"), b"v");
        assert_eq!(findterm(b"${}x"), b"x");
        assert_eq!(findterm(b"no terms"), b"no terms");
    }
}
