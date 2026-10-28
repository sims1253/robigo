# Recorded by: Rscript --vanilla scripts/oracle-real-arith.R > arith_r_oracle.txt (then gzip)
# Bits of GNU R's double + - * / ^ %% %/% on special values, in each of
# real_binary's recycling shapes (equal lengths, scalar x, scalar y,
# recycled), and with an integer operand (NA_integer_ becomes NA_real_).
hex2d <- function(h) vapply(h, function(s) readBin(as.raw(strtoi(substring(s, seq(15, 1, -2), seq(16, 2, -2)), 16L)), "double"), 0, USE.NAMES = FALSE)
d2hex <- function(x) vapply(x, function(v) paste(rev(as.character(writeBin(v, raw()))), collapse = ""), "")
comp <- c("7ff00000000007a2", "7ff80000000007a2", "fff8000000000000", "7ff8000000000000",
          "7ff8000000000123", "fff4000000000456", "7ff0000000000000", "fff0000000000000",
          "0000000000000000", "8000000000000000", "3ff0000000000000", "c008000000000000",
          "7fefffffffffffff", "0000000000000001", "4000000000000000", "3fe0000000000000")
v <- hex2d(comp); n <- length(v)
ix <- rep(seq_len(n), each = n); iy <- rep(seq_len(n), n)
ops <- c(add = "+", sub = "-", mul = "*", div = "/", pow = "^", mod = "%%", idiv = "%/%")
for (nm in names(ops)) {
  f <- get(ops[[nm]])
  # equal lengths (vector of all pairs)
  r <- f(v[ix], v[iy])
  writeLines(paste(nm, "vv", d2hex(v[ix]), d2hex(v[iy]), d2hex(r)))
  # scalar x / scalar y, one call per pair
  for (k in seq_along(ix)) {
    a <- v[ix[k]]; b <- v[iy[k]]
    writeLines(paste(nm, "ss", d2hex(a), d2hex(b), d2hex(f(a, b))))
    writeLines(paste(nm, "sv", d2hex(a), d2hex(b), d2hex(f(a, c(b, b, b))[2])))
    writeLines(paste(nm, "vs", d2hex(a), d2hex(b), d2hex(f(c(a, a, a), b)[2])))
    writeLines(paste(nm, "rc", d2hex(a), d2hex(b), d2hex(f(c(a, a, a, a), c(b, b))[3])))
  }
}

# --- integer with double (the second half of arith_r_oracle.txt.gz) ---
dv <- hex2d(c("7ff00000000007a2", "fff8000000000000", "7ff8000000000000", "7ff8000000000123", "fff4000000000456", "3ff0000000000000"))
iv <- c(NA_integer_, 3L)
for (op in c("+", "-", "*", "/")) for (a in iv) for (b in dv) {
  f <- get(op); tag <- paste0(if (is.na(a)) "NAi" else "3L")
  r <- c(ss_id = f(a, b), vv_id = f(c(a, a), c(b, b))[2], sv_id = f(a, c(b, b, b))[2], vs_id = f(c(a, a, a), b)[2], rc_id = f(c(a, a, a, a), c(b, b))[3],
         ss_di = f(b, a), vv_di = f(c(b, b), c(a, a))[2], sv_di = f(b, c(a, a, a))[2], vs_di = f(c(b, b, b), a)[2], rc_di = f(c(b, b, b, b), c(a, a))[3])
  writeLines(paste(op, tag, d2hex(b), names(r), d2hex(r)))
}
