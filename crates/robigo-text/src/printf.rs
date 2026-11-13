//! One C `printf` conversion as glibc performs it: `%[flags][width]
//! [.precision]conv` for `d i o x X` (an `int`), `f F e E g G` (a finite
//! `double`) and `s` (a byte string), what R's `sprintf()` hands to
//! `snprintf`. Doubles are correctly rounded with ties to even, as glibc
//! prints them. `%a`/`%A` (hexadecimal floating point) are not supported:
//! [`Spec::parse`] reports them.

/// A parsed conversion specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub left: bool,
    pub plus: bool,
    pub space: bool,
    pub alt: bool,
    pub zero: bool,
    pub width: Option<usize>,
    pub precision: Option<usize>,
    pub conv: u8,
}

impl Spec {
    /// Parse `%...c`: None if it is not one conversion of the supported
    /// kinds (a `*` must have been replaced by the caller).
    pub fn parse(fmt: &[u8]) -> Option<Spec> {
        let mut p = fmt.strip_prefix(b"%")?;
        let mut s = Spec {
            left: false,
            plus: false,
            space: false,
            alt: false,
            zero: false,
            width: None,
            precision: None,
            conv: 0,
        };
        while let Some((&c, rest)) = p.split_first() {
            match c {
                b'-' => s.left = true,
                b'+' => s.plus = true,
                b' ' => s.space = true,
                b'#' => s.alt = true,
                b'0' => s.zero = true,
                _ => break,
            }
            p = rest;
        }
        let digits = |p: &mut &[u8]| -> Option<usize> {
            let n = p.iter().take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return None;
            }
            let v = std::str::from_utf8(&p[..n]).ok()?.parse().ok();
            *p = &p[n..];
            v
        };
        s.width = digits(&mut p);
        if let Some(rest) = p.strip_prefix(b".") {
            p = rest;
            s.precision = Some(digits(&mut p).unwrap_or(0));
        }
        let (&c, rest) = p.split_first()?;
        if !rest.is_empty() || !b"dioxXfFeEgGs".contains(&c) {
            return None;
        }
        s.conv = c;
        Some(s)
    }

    /// Pad `body` (a sign or prefix of `lead` bytes, then digits) to the
    /// width: spaces on the left or right, or zeros after the lead.
    fn pad(&self, out: &mut Vec<u8>, body: &[u8], lead: usize, zero_ok: bool) {
        let w = self.width.unwrap_or(0);
        let fill = w.saturating_sub(body.len());
        if self.left {
            out.extend_from_slice(body);
            out.resize(out.len() + fill, b' ');
        } else if self.zero && zero_ok {
            out.extend_from_slice(&body[..lead]);
            out.resize(out.len() + fill, b'0');
            out.extend_from_slice(&body[lead..]);
        } else {
            out.resize(out.len() + fill, b' ');
            out.extend_from_slice(body);
        }
    }

    /// The sign of a signed conversion.
    fn sign(&self, neg: bool) -> &'static [u8] {
        if neg {
            b"-"
        } else if self.plus {
            b"+"
        } else if self.space {
            b" "
        } else {
            b""
        }
    }

    /// `d i o x X` of `x`.
    pub fn int(&self, out: &mut Vec<u8>, x: i32) {
        let (neg, mag) = (x < 0, x.unsigned_abs());
        let mut digits = match self.conv {
            b'o' => format!("{:o}", x as u32).into_bytes(),
            b'x' => format!("{:x}", x as u32).into_bytes(),
            b'X' => format!("{:X}", x as u32).into_bytes(),
            _ => mag.to_string().into_bytes(),
        };
        let signed = matches!(self.conv, b'd' | b'i');
        if let Some(p) = self.precision {
            if p == 0 && x == 0 {
                digits.clear();
            }
            if digits.len() < p {
                let mut z = vec![b'0'; p - digits.len()];
                z.extend_from_slice(&digits);
                digits = z;
            }
        }
        let mut body = Vec::new();
        let mut lead = 0;
        if signed {
            body.extend_from_slice(self.sign(neg));
            lead = body.len();
        } else if self.alt {
            match self.conv {
                b'o' if digits.first() != Some(&b'0') => digits.insert(0, b'0'),
                b'x' if x != 0 => {
                    body.extend_from_slice(b"0x");
                    lead = 2;
                }
                b'X' if x != 0 => {
                    body.extend_from_slice(b"0X");
                    lead = 2;
                }
                _ => {}
            }
        }
        body.extend_from_slice(&digits);
        // the 0 flag is ignored when a precision is given
        self.pad(out, &body, lead, self.precision.is_none());
    }

    /// `f F e E g G` of a finite `x`.
    pub fn double(&self, out: &mut Vec<u8>, x: f64) {
        let neg = x.is_sign_negative();
        let a = x.abs();
        let prec = self.precision.unwrap_or(6);
        let mut digits: Vec<u8> = match self.conv {
            b'f' | b'F' => fixed(a, prec, self.alt),
            b'e' | b'E' => exp(a, prec, self.alt, self.conv == b'E'),
            _ => {
                // %g: the style by the exponent %e would show
                let p = if prec == 0 { 1 } else { prec };
                let e = exp(a, p - 1, false, false);
                let xexp = exponent_of(&e);
                let mut d = if (p as i32) > xexp && xexp >= -4 {
                    fixed(a, (p as i32 - 1 - xexp) as usize, self.alt)
                } else {
                    exp(a, p - 1, self.alt, self.conv == b'G')
                };
                if !self.alt {
                    strip_zeros(&mut d);
                }
                d
            }
        };
        let mut body = self.sign(neg).to_vec();
        let lead = body.len();
        body.append(&mut digits);
        self.pad(out, &body, lead, true);
    }

    /// `s` of the bytes `s`: the precision truncates (in bytes, as C).
    pub fn string(&self, out: &mut Vec<u8>, s: &[u8]) {
        let s = match self.precision {
            Some(p) if p < s.len() => &s[..p],
            _ => s,
        };
        self.pad(out, s, 0, false);
    }
}

/// `%.*f` of a non-negative finite `a`; `alt` keeps the point at precision 0.
fn fixed(a: f64, prec: usize, alt: bool) -> Vec<u8> {
    let mut s = format!("{a:.prec$}").into_bytes();
    if alt && prec == 0 {
        s.push(b'.');
    }
    s
}

/// `%.*e` of a non-negative finite `a`, exponent of at least two digits.
fn exp(a: f64, prec: usize, alt: bool, upper: bool) -> Vec<u8> {
    let s = format!("{a:.prec$e}");
    let (mant, e) = s.split_once('e').expect("an exponent");
    let (sign, digits) = match e.strip_prefix('-') {
        Some(d) => (b'-', d),
        None => (b'+', e),
    };
    let mut body = mant.as_bytes().to_vec();
    if alt && prec == 0 {
        body.push(b'.');
    }
    body.push(if upper { b'E' } else { b'e' });
    body.push(sign);
    if digits.len() < 2 {
        body.push(b'0');
    }
    body.extend_from_slice(digits.as_bytes());
    body
}

/// The exponent of a `%e` rendering.
fn exponent_of(e: &[u8]) -> i32 {
    let pos = e.iter().position(|&c| c == b'e' || c == b'E').expect("an exponent");
    std::str::from_utf8(&e[pos + 1..]).expect("ASCII").parse().expect("digits")
}

/// `%g`'s removal of trailing zeros (and a trailing point) from the
/// fraction, before any exponent.
fn strip_zeros(d: &mut Vec<u8>) {
    let epos = d.iter().position(|&c| c == b'e' || c == b'E').unwrap_or(d.len());
    if !d[..epos].contains(&b'.') {
        return;
    }
    let mut end = epos;
    while end > 0 && d[end - 1] == b'0' {
        end -= 1;
    }
    if end > 0 && d[end - 1] == b'.' {
        end -= 1;
    }
    d.drain(end..epos);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fmt: &str, x: f64) -> String {
        let mut o = Vec::new();
        Spec::parse(fmt.as_bytes()).unwrap().double(&mut o, x);
        String::from_utf8(o).unwrap()
    }

    fn i(fmt: &str, x: i32) -> String {
        let mut o = Vec::new();
        Spec::parse(fmt.as_bytes()).unwrap().int(&mut o, x);
        String::from_utf8(o).unwrap()
    }

    // Expected values: glibc's printf.
    #[test]
    fn like_glibc() {
        assert_eq!(f("%f", 3.14159), "3.141590");
        assert_eq!(f("%.2f", 2.675), "2.67");
        assert_eq!(f("%8.3f", -1.5), "  -1.500");
        assert_eq!(f("%-8.1f", 1.0), "1.0     ");
        assert_eq!(f("%08.2f", -3.14159), "-0003.14");
        assert_eq!(f("%+.1e", 12345.0), "+1.2e+04");
        assert_eq!(f("%E", 0.000123), "1.230000E-04");
        assert_eq!(f("%g", 100000.0), "100000");
        assert_eq!(f("%g", 1000000.0), "1e+06");
        assert_eq!(f("%g", 0.0001), "0.0001");
        assert_eq!(f("%g", 0.00001), "1e-05");
        assert_eq!(f("%.3g", 3.14159), "3.14");
        assert_eq!(f("%#.3g", 1.0), "1.00");
        assert_eq!(f("%g", 0.0), "0");
        assert_eq!(f("%5.1f", 9.96), " 10.0");
        assert_eq!(i("%d", -42), "-42");
        assert_eq!(i("%5d", 42), "   42");
        assert_eq!(i("%-5d", 42), "42   ");
        assert_eq!(i("%05d", -42), "-0042");
        assert_eq!(i("%.3d", 7), "007");
        assert_eq!(i("%x", 255), "ff");
        assert_eq!(i("%#X", 255), "0XFF");
        assert_eq!(i("%#o", 8), "010");
        assert_eq!(i("%+d", 5), "+5");
        assert_eq!(i("% d", 5), " 5");
        assert_eq!(i("%.0d", 0), "");
    }
}
