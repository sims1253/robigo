# Records the expected output of robigo-text's ports from GNU R 4.6.2.
#
#   LC_ALL=C.UTF-8 Rscript cases.R numbers  > ../tests/data/numbers.tsv
#   LC_ALL=C.UTF-8 Rscript cases.R strings  > ../tests/data/strings-utf8.tsv
#   LC_ALL=C       Rscript cases.R strings  > ../tests/data/strings-c.tsv
#
# `crates/robigo-text/oracle/record.sh` runs all three. Never record with a
# GNU R other than 4.6.2.
#
# Every line is one case, fields separated by tabs. Byte strings are
# percent-encoded: bytes 0x20..0x7e other than '%' as they are, every other
# byte as %XX. Doubles are C99 hex floats (`sprintf("%a")`), or NA, NaN,
# Inf, -Inf. The format of each kind is documented in tests/oracle.rs.

stopifnot(R.version$major == "4", R.version$minor == "6.2")

args <- commandArgs(trailingOnly = TRUE)
what <- if (length(args)) args[[1]] else "numbers"

pct <- function(s) {
  b <- as.integer(charToRaw(s))
  paste0(vapply(b, function(k)
    if (k >= 0x20 && k <= 0x7e && k != 0x25) rawToChar(as.raw(k))
    else sprintf("%%%02X", k), ""), collapse = "")
}
hexd <- function(x) {
  vapply(x, function(v) {
    if (is.na(v) && !is.nan(v)) "NA" else if (is.nan(v)) "NaN"
    else if (v == Inf) "Inf" else if (v == -Inf) "-Inf"
    else sprintf("%a", v)
  }, "")
}
emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")

## ---------------------------------------------------------------- numbers

# R  digits scipen nsmall width dec  x1,x2,...  w,d,e  s1  s2 ...
# formatReal via .Internal(format.info()) and EncodeReal0 via
# .Internal(format()), which calls formatReal, takes max(w, width) and
# encodes each element with the decimal mark `dec`.
real_case <- function(x, digits = 7L, scipen = 0L, nsmall = 0L, width = 0L,
                      dec = ".") {
  op <- options(scipen = scipen); on.exit(options(op))
  info <- .Internal(format.info(x, digits, nsmall))
  # (a decimal mark wider than one character warns in EncodeReal0)
  s <- suppressWarnings(
    .Internal(format(x, FALSE, digits, nsmall, width, 3L, TRUE, scipen, dec)))
  emit("R", digits, scipen, nsmall, width, pct(dec),
       paste(hexd(x), collapse = ","), paste(info, collapse = ","),
       paste(vapply(s, pct, ""), collapse = "\t"))
}

# I  width  x1,x2,...  w  s1 ...   (formatInteger, EncodeInteger)
int_case <- function(x, width = 0L) {
  x <- as.integer(x)
  info <- .Internal(format.info(x, NULL, 0L))
  s <- .Internal(format(x, FALSE, NULL, 0L, width, 3L, TRUE, NA, "."))
  emit("I", width, paste(ifelse(is.na(x), "NA", as.character(x)), collapse = ","),
       info[[1]], paste(vapply(s, pct, ""), collapse = "\t"))
}

# L  width  x1,x2,...  w  s1 ...   (formatLogical, EncodeLogical)
lgl_case <- function(x, width = 0L) {
  info <- .Internal(format.info(x, NULL, 0L))
  s <- .Internal(format(x, FALSE, NULL, 0L, width, 3L, TRUE, NA, "."))
  emit("L", width, paste(ifelse(is.na(x), "NA", ifelse(x, "T", "F")), collapse = ","),
       info[[1]], paste(vapply(s, pct, ""), collapse = "\t"))
}

# D  x  s   (EncodeRealDrop0 via as.character(), 15 significant digits)
drop0_case <- function(x) emit("D", hexd(x), pct(as.character(x)))

numbers <- function() {
  special <- c(0, -0, 1, -1, 0.1, 0.5, 1/3, 2/3, pi, -pi, exp(1), 100, 1e5,
               123456, 1234567, 0.0001234, 1e-5, 1e15, 1e16, 1e22, 1e23,
               1e-300, 1e300, .Machine$double.xmax, .Machine$double.xmin,
               5e-324, 2.225073858507201e-308, 4.9406564584124654e-324 * 3,
               9.9999999, 99999.5, 9995, 9996, 0.15, 2.675, 1.005, 0.95,
               123.456, -0.000012345, 1/7, 2^52, 2^53, 2^53 + 2, 2^63, 2^-30,
               1e100, 1.5e-100, 7.0e-5, 99.99, 0.099999, 999999.9)
  ## each special value alone, at every digits
  for (d in 1:22) for (x in special) real_case(x, digits = d)
  ## non-finite and mixed vectors
  nf <- list(NA_real_, NaN, Inf, -Inf, c(NA, 1.5), c(NaN, -1), c(Inf, -Inf),
             c(NA, NaN, Inf, -Inf), c(1, NA, 100.25), c(-Inf, 1e-20),
             c(1e10, 1, NA), c(0, -0), numeric(0) + 1)
  for (x in nf) for (d in c(1L, 3L, 7L, 15L, 22L)) real_case(x, digits = d)
  ## vectors that mix magnitudes (F vs E decision, roundingwidens)
  vecs <- list(c(1, 10, 100), c(0.1, 0.12, 0.123), c(1e-10, 1), c(1e5, 1e-5),
               c(123456789, 0.1), c(1.5, 2.25, 1234.5678), c(9996, 1),
               c(0.00099999, 123), c(-1, 1e-3, 1e3), c(1e-15, 1e15),
               c(1e200, 1e-200), c(1e-99, 1), c(1e-100, 1), c(1e101, 1),
               c(1e100, 1), seq(0, 1, by = 0.1), (1:10)^5 / 7, -(1:5) * 1e-8,
               c(3.14159, 2.71828, 1.41421), c(99999, 100000, 100001))
  for (x in vecs) for (d in c(1L, 2L, 4L, 7L, 12L, 15L, 16L, 17L, 22L))
    real_case(x, digits = d)
  ## scipen in both directions
  ## (options() clamps scipen to -9, which format.info reads)
  for (sp in c(-9L, -5L, -3L, -1L, 1L, 3L, 10L, 100L, 300L))
    for (x in list(1e5, 123456, 1e-5, 0.000123, c(1, 1e10), 1e300, 3.5e-7,
                   c(1.5, NA, Inf)))
      real_case(x, scipen = sp)
  ## nsmall and width
  for (ns in c(1L, 2L, 5L, 20L))
    for (x in list(1, 3.14159, c(1.5, 10), 1e10, 1e-10, 123456789))
      real_case(x, nsmall = ns)
  for (wd in c(1L, 10L, 25L))
    for (x in list(1.5, c(NA, 2), -Inf, 1e-20))
      real_case(x, width = wd)
  ## decimal marks
  for (dec in c(",", intToUtf8(0xb7), "dec"))
    for (x in list(1.5, c(1, 2.25), 1e-20, NA_real_, Inf))
      real_case(x, dec = dec)
  ## a seeded sweep over magnitudes and digit counts
  set.seed(4062)
  for (i in 1:150) {
    m <- sample(c(-320, -310, -300, -100, -20, -8, -5, -2, -1, 0, 1, 2, 5,
                  8, 14, 15, 16, 20, 100, 300, 307), 1)
    x <- signif(runif(1, 1, 10), sample(1:17, 1)) * 10^m
    if (runif(1) < 0.3) x <- -x
    real_case(x, digits = sample(1:22, 1))
  }
  for (i in 1:60) {
    x <- signif(rnorm(sample(2:5, 1)) * 10^sample(-6:8, 1), sample(1:9, 1))
    real_case(x, digits = sample(c(1:9, 15:17), 1), scipen = sample(-5:5, 1))
  }

  ## integers
  ints <- list(0, 1, -1, 9, 10, -10, 99, 100, 12345, -12345,
               .Machine$integer.max, -.Machine$integer.max, NA, c(NA, 1),
               c(NA, -1000), c(1, 22, 333), c(-5, 5), integer(0) + 7L)
  for (x in ints) int_case(x)
  for (wd in c(3L, 12L)) int_case(c(1, NA, -20), width = wd)

  ## logicals
  for (x in list(TRUE, FALSE, NA, c(TRUE, NA), c(TRUE, FALSE), c(NA, FALSE),
                 c(TRUE, TRUE, NA)))
    lgl_case(x)
  for (wd in c(2L, 4L, 7L)) lgl_case(c(TRUE, NA), width = wd)

  ## as.character(<double>)
  for (x in c(special, NaN, Inf, -Inf, 1e-4, 1234567.1, 0.30000000000000004,
              100000, 1e15 + 0.3, 123456789012, 1/998001))
    drop0_case(x)
}

## ---------------------------------------------------------------- strings

# S  enc  bytes  quote  justify  width  rstrlen0  rstrlenq  out
#   enc: native, UTF-8, latin1 or bytes (the CHARSXP's mark).
#   quote: the quote byte (percent-encoded) or empty.
#   justify: 0 left, 1 right, 2 centre, 3 none.
#   width: the w argument (NA: encodeString's own, the widest element).
#   rstrlen0: format.info(s) = Rstrlen(s, 0); NA for bytes strings, where
#     formatinfo is not consulted.
#   rstrlenq: Rstrlen(s, quote) read off the padding of "" next to s.
#   out: encodeString(s, width, quote, justify): EncodeString, with
#     strings marked UTF-8 kept in UTF-8 (w - 1000000).
mk <- function(bytes, enc) {
  s <- rawToChar(as.raw(bytes))
  Encoding(s) <- enc
  s
}
str_case <- function(bytes, enc, quote = "", justify = 0L, width = NA) {
  s <- mk(bytes, enc)
  r0 <- if (enc == "bytes") NA else .Internal(format.info(s, NULL, 0L))[[1]]
  rq <- nchar(encodeString(c(s, ""), quote = quote, width = NA)[[2]], "bytes") -
    if (nzchar(quote)) 2L else 0L
  out <- .Internal(encodeString(s, if (is.na(width)) NULL else width,
                                quote, justify, TRUE))
  emit("S", if (enc == "unknown") "native" else enc,
       pct(rawToChar(as.raw(bytes))), pct(quote), justify,
       if (is.na(width)) "NA" else width, r0, rq, pct(out))
}

strings <- function() {
  # UTF-8 bytes of code points (intToUtf8 does not depend on the locale)
  u <- function(...) as.integer(charToRaw(intToUtf8(c(...))))
  b <- function(...) c(...)
  texts <- list(
    b(0x61, 0x62, 0x63),                       # abc
    b(0x61, 0x22, 0x62),                       # a"b
    b(0x61, 0x27, 0x62),                       # a'b
    b(0x61, 0x60, 0x62),                       # a`b
    b(0x5c),                                   # backslash
    b(0x07, 0x08, 0x0c, 0x0a, 0x0d, 0x09, 0x0b), # \a\b\f\n\r\t\v
    b(0x01, 0x1b, 0x7f, 0x1f),                 # other controls: octal
    u(0x63, 0x61, 0x66, 0xe9), u(0xfc, 0xdf), u(0x4e2d, 0x6587), u(0xff21),
    u(0x65, 0x301),                            # combining acute: width 0
    u(0x200b), u(0xad),                        # zero width, soft hyphen
    u(0x2028), u(0x85), u(0xa0),               # separators, NEL, nbsp
    u(0xe00), u(0x2072), u(0xfffd),            # unassigned, replacement
    u(0x1f600), u(0x1f43b, 0x200d, 0x2744), u(0x10fffd),
    u(0xe0001), u(0x1d11e), u(0x20000),
    b(0xff), b(0xc3), b(0x61, 0xe2, 0x82),     # invalid / truncated UTF-8
    b(0xe2, 0x82, 0x61), b(0x80, 0x61),
    b(0xc0, 0x80), b(0xc1, 0x81),              # overlong
    b(0xed, 0xa0, 0x80),                       # surrogate
    b(0xef, 0xbf, 0xbe),                       # U+FFFE
    b(0xf4, 0x90, 0x80, 0x80),                 # above U+10FFFF
    b(0xf8, 0x88, 0x80, 0x80, 0x80),           # five bytes
    b(0xe9, 0x74, 0xe9), b(0x80, 0x81, 0x8d, 0x9f, 0xa0, 0xff), # latin-1/cp1252
    b(0x61, 0x22, 0xff, 0x0a))
  for (t in texts) for (enc in c("unknown", "UTF-8", "latin1", "bytes")) {
    if (enc == "UTF-8" && !validUTF8(rawToChar(as.raw(t))) &&
        !(t[[1]] %in% c(0xc0, 0xc1, 0xed, 0xef, 0xf4, 0xf8))) next
    for (q in c("", "\"", "'")) str_case(t, enc, quote = q)
  }
  ## quoting with a quote character of its own, justification, widths
  for (enc in c("unknown", "UTF-8"))
    for (t in list(b(0x61, 0x60, 0x62), u(0x78, 0xe9)))
      str_case(t, enc, quote = "`")
  for (j in 0:3) for (w in c(NA, 0L, 3L, 8L, 11L)) {
    str_case(u(0x61, 0x62), "unknown", quote = "\"", justify = j, width = w)
    str_case(u(0x4e2d), "UTF-8", quote = "", justify = j, width = w)
    str_case(b(0xe9), "latin1", quote = "'", justify = j, width = w)
  }
  ## NA_character_ (na.encode = TRUE)
  for (q in c("", "\"")) for (j in 0:3) for (w in c(NA, 1L, 6L)) {
    out <- .Internal(encodeString(NA_character_, if (is.na(w)) NULL else w,
                                  q, j, TRUE))
    emit("N", pct(q), j, if (is.na(w)) "NA" else w, pct(out))
  }
}

switch(what, numbers = numbers(), strings = strings(),
       stop("unknown case set: ", what))
