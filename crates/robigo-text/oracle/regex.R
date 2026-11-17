# Records what GNU R 4.6.2's TRE does: regexec() of every pattern over every
# text, for each combination of fixed, ignore.case and useBytes.
#
#   LC_ALL=C.UTF-8 Rscript regex.R       > ../tests/data/regex-utf8.tsv
#   LC_ALL=C       Rscript regex.R       > ../tests/data/regex-c.tsv
#   LC_ALL=C.UTF-8 Rscript regex.R fuzz  > ../tests/data/regex-fuzz-utf8.tsv
#   LC_ALL=C       Rscript regex.R fuzz  > ../tests/data/regex-fuzz-c.tsv
#   LC_ALL=C.UTF-8 Rscript regex.R afuzz > ../tests/data/regex-afuzz-utf8.tsv
#   LC_ALL=C       Rscript regex.R afuzz > ../tests/data/regex-afuzz-c.tsv
#
# "fuzz" replaces the hand-picked patterns and texts with random ones
# (seeded) and records fewer flag combinations. "afuzz" does the same with
# approximate-matching parameters in the grammar.
#
# record.sh runs both in the oracle image. Never record with a GNU R other
# than 4.6.2.
#
# regexec() is the grep.c entry point that shows TRE's result most
# directly: the match and every subexpression. Each text is matched on its
# own, so the choice between bytes, wide and multibyte matching is made per
# text. Lines, fields separated by tabs, strings percent-encoded (bytes
# 0x20..0x7d other than '%' and '|' as they are, every other byte as %XX):
#
#   T id enc s           a text; enc is native, UTF-8, latin1 or bytes
#   P id enc s           a pattern
#   R pid flags r...     pattern pid against each text in order; flags are
#                        three 0/1 digits: fixed, ignore.case, useBytes.
#                        r~n stands for n copies of r. fixed = TRUE with
#                        ignore.case = TRUE only warns and drops ignore.case,
#                        so it is recorded for the first pattern only.
#
# A result r is NA (an NA text), - (no match), or the matches as
# start:length pairs joined by ','; prefixed by 'c' or 'b' when R sets the
# attributes useBytes = TRUE and index.type = "chars" or "bytes". An error
# is E followed by its message. Warnings come first, each as W message |.

stopifnot(R.version$major == "4", R.version$minor == "6.2")

pct <- function(s) {
  b <- as.integer(charToRaw(s))
  paste0(vapply(b, function(k)
    if (k >= 0x20 && k <= 0x7d && k != 0x25 && k != 0x7c) rawToChar(as.raw(k))
    else sprintf("%%%02X", k), ""), collapse = "")
}
emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")
enc <- function(s) {
  e <- Encoding(s)
  if (e == "unknown") "native" else e
}
mark <- function(s, e) {
  Encoding(s) <- e
  s
}
native <- function(...) rawToChar(as.raw(c(...)))
# In the C locale the parser leaves "\u" escapes as unmarked UTF-8 bytes.
utf8 <- function(s) mark(s, "UTF-8")

texts <- c(
  # ASCII
  "", "a", "b", "e", "ab", "ba", "abc", "aaa", "aab", "abab", "abcd",
  "xabbc", "abcabcabc", "aa a", "zzz", "aAbBcC", "Hello World 42",
  "cat sat on the mat", "foo.bar_baz", "a+b*c?", "a\\b", "ab\ncd",
  "\t x \r\n", "123-456.78e9", "A_b-C d", "[x]{2}", "(a)|b^c$",
  "--__--", "abbbbbbbbbbc", "\001\177~",
  # near misses for approximate matching
  "abd", "xbc", "color", "colour", "acb", "abxc", "aXbYc",
  # marked UTF-8
  utf8(c("\u00e9", "caf\u00e9", "CAF\u00c9", "na\u00efve \u00dcn\u00efcode",
  "\u65e5\u672c\u8a9e\u30c6\u30ad\u30b9\u30c8", "stra\u00dfe STRASSE",
  "a\u20acb", "\U1F600x\U1F600", "\u03b1\u03b2\u03b3 \u0391\u0392\u0393",
  "\u00e9\u00e9a", "\u216b \u217b", "\u0130\u0131 i", "\u00a0x\u2003",
  "\u0660\u0661 12")),
  # latin1, bytes, native non-ASCII
  mark("caf\xe9", "latin1"), mark("\xc0\xe9\xff a", "latin1"),
  # R reads latin1 as CP1252, where 0x81 is undefined
  mark("\x80\x93x", "latin1"), mark("a\x81", "latin1"),
  mark("caf\xe9", "bytes"), mark("\xe9t\xe9", "bytes"),
  native(0x63, 0x61, 0x66, 0xc3, 0xa9),        # valid UTF-8
  native(0x61, 0xe9, 0x62),                    # not UTF-8
  NA)

pats <- list(
  # literals and the basic operators
  "a", "b", "abc", "", ".", "a.c", "..", ".*", ".+", "^a", "c$", "^$", "^",
  "$", "^.", ".$", "a^b", "a$b", "(^a)", "(c$)", "^(a|b)", "(a|b)$",
  "a|b", "ab|cd", "a|", "|a", "|", "a||b", "(a|)", "(|a)", "()",
  "(a)", "(a)(b)", "((a)b)", "(a)|b", "(a|ab)(c|bcd)(d*)",
  "(a*)(b)", "x*", "a*", "a+", "a?", "a*?", "a+?", "a??", "(a*)*",
  "(a*)+", "(a|b)*", "(a|b)*c", "(a+|b)*", "((a)|b)+", "(a?)((ab)?)(b?)",
  "(.*)(.*)", "(.*?)(.*)", "(a*?)(a*)", "(a+?)(b*)", "(a|ab)*?c",
  "a.*b", "a.*?b", "b+c", "(ab)+", "(a.)+",
  # bounds
  "a{2}", "a{2,}", "a{,2}", "a{1,3}", "a{0}", "a{0}b", "a{0,0}",
  "(a){0}", "(a){2}", "(a){2,3}b", "(ab){1,}", "(a{2,1})", "a{2,1}",
  "a{1,2}?", "a{2}{3}", "b{10}", "a{255}", "a{256}", "a{1,1000}",
  "(a|b){3,5}", "(a*){2}", "(){2}", "a{", "a{1", "a{1,", "a{x}", "{",
  "a{,}", "{1}", "a{ 1}",
  # repetition errors
  "*a", "+", "?", "a**", "a*+", "a+*", "(*a)", "(|*)", "^*", "a|*b",
  # parentheses
  "(", ")", "a)", "(a", "((a)", "(a))", "\\(", "\\)",
  # brackets
  "[", "[a", "[]", "[]a]", "[^]a]", "[a-]", "[-a]", "[a-c]", "[c-a]",
  "[^a-c]", "[^a]", "[a-c]+", "[ab][cd]", "[.]", "[*+?]", "[\\]",
  "[\\d]", "[a\\-z]", "[[]", "[]-a]", "[--/]", "[a-a]", "[%--]",
  "[[:alpha:]]", "[[:digit:]]+", "[[:alnum:]_]+", "[[:space:]]",
  "[[:upper:]]+", "[[:lower:]]+", "[[:punct:]]", "[[:xdigit:]]+",
  "[[:cntrl:]]", "[[:print:]]+", "[[:graph:]]+", "[[:blank:]]",
  "[[:foo:]]", "[[:alpha:]", "[[:alpha]]", "[^[:alpha:]]",
  "[^[:alpha:]]{2}", "[^[:alpha:][:digit:]]", "[^[:upper:]a]+",
  "[[:alpha:]-]", "[a-[:digit:]]", "[[.a.]]", "[[=a=]]", "[[.space.]]",
  "[[.-.]]", "[[:alpha:][:digit:]]+", "[^[:space:]]+",
  # escapes
  "\\d", "\\D+", "\\w+", "\\W", "\\s", "\\S+", "\\b", "\\B", "\\<", "\\>",
  "\\bcat\\b", "\\<t", "a\\>", "\\Ba", "\\`", "\\'", "\\`a", "c\\'",
  "\\", "a\\", "\\.", "\\*", "\\[", "\\{", "\\}", "\\|", "\\^", "\\$",
  "\\n", "\\t", "\\r", "\\e", "\\f", "\\v", "\\a", "\\x41", "\\x{e9}",
  "\\x{263A}", "\\x", "\\x{", "\\x{zz}", "\\Q", "\\z", "\\0", "\\w\\W",
  "\\d+\\.\\d+", "\\s+\\S",
  # back references
  "(a)\\1", "(a*)\\1", "(a)\\2", "\\1", "(a)|\\1", "(.)\\1", "([a-c])\\1+",
  "(a).*\\1|z", "(a)\\1|c", "(ab|a)\\1", "(a+)(b+)\\2\\1", "(a*)b\\1",
  "(.+)\\1", "(.*)\\1", "((a)b)\\2", "(a)(b)(c)\\3\\2\\1", "(b*)\\1c",
  "(a|b)\\1*", "(.)(.)\\2\\1",
  # inline options
  "(?i)abc", "(?i)a(?-i)b", "(?r)a*b", "(?U)a+", "(?n)a", "(?:a)", "(?",
  "(?i", "(?x)a",
  # literals that only mean something to fixed = TRUE
  "a.b", "a+b*c?", "a\\b", "[x]{2}", "(a)|b^c$",
  # beyond ASCII
  utf8("\u00e9"), utf8("[\u00e9]"), utf8("[\u00e0-\u00ff]+"),
  utf8("caf\u00e9"), utf8(".\u00e9"), utf8("[[:alpha:]]+\u00e9"),
  utf8("\u00c9"), utf8("\u65e5\u672c"), utf8("[\u65e5\u672c]+"),
  utf8("\u00df"), utf8("\u20ac"), utf8("\U1F600"), utf8("\u03b2"),
  utf8("\u0392"), utf8("[\u03b1-\u03c9]+"), utf8("\u0131"), utf8("\u0130"),
  utf8("(\u00e9)\\1"), utf8("\u00e9{2}"), utf8("[^\u00e9]+"),
  utf8("\u00e9|a"), utf8("[\u20ac-\u00e9]"),
  mark("\xe9", "latin1"), mark("caf\xe9", "latin1"), mark("[\xe0-\xff]", "latin1"),
  mark("\xe9", "bytes"), mark("[\x80-\xff]+", "bytes"),
  native(0xc3, 0xa9), native(0xe9),
  # nesting deep enough to exhaust TRE's stack
  paste0(strrep("(", 1000), "q", strrep(")", 1000)),
  paste0(strrep("(", 3000), "a", strrep(")", 3000)),
  paste0(strrep("a|", 2000), "b"),
  paste0(strrep("(?:", 10), "a", strrep(")", 10)),
  # approximate matching (TRE's {~n}-style parameters)
  "abc{~1}", "(abc){~}", "(abc){~1}", "(abc){~2}", "a{#1}c", "(abc){#1}",
  "(abc){+1}", "(abc){-1}", "(abc){+1-1}", "(abc){~2,1i}", "x{~2,1i}",
  "(ab){~1}c\\1", "colour{~1}", "(colour){~1}", "(colou?r){~1}",
  "(abc){~2,1i+1d}", "(abc){1i+2d+1s<3}", "(abc){2i+1d+1s<4}",
  "(abc){1s<1}", "(abc){~1}(d){~1}", "((ab){~1}c){~1}",
  "((((a){~1}b){~1}c){~1}d){~1}", "(((((a){~1}b){~1}c){~1}d){~1}e){~1}",
  "(a|b){~1}", "^(abc){~1}$", "\\<(abc){~1}\\>", "([[:alpha:]]+){~1}",
  "(a*b){~1}", "(.b){~1}", "(abc){~1}?", "a{~1}{2}", "(abc){~,2i}",
  "(abc){~a}", "(abc){1i}", "(abc){~1,2}", "(abc){~1", "(aXc){~1}",
  "(b){~3}", "(abc){~1}|x", "([ab]c){#1}", "(a\\wc){~1}", "(abc){0,1}",
  utf8("(caf\u00e9){~1}"), utf8("(\u00e9a){~1}"),
  mark("(caf\xe9){~1}", "latin1"), mark("(caf\xe9){~1}", "bytes")
)

args <- commandArgs(trailingOnly = TRUE)
afuzz <- length(args) && args[[1]] == "afuzz"
fuzz <- afuzz || length(args) && args[[1]] == "fuzz"
flagsets <- function(pi) if (pi == 1) 0:7 else 0:5
if (fuzz) {
  set.seed(if (afuzz) 4063 else 4062)
  u <- utf8("\u00e9")
  pick <- function(x, ...) x[[sample.int(length(x), 1, ...)]]
  unif <- function() sample.int(1000, 1) / 1000  # (stats is not attached)
  lits <- list("a", "b", "c", "A", " ", u, "x")
  classes <- list("[ab]", "[^a]", "[a-c]", "[[:alpha:]]", "[^[:space:]]",
                  paste0("[", u, " a]"), "[[:upper:]]", "[^[:alpha:]]")
  escapes <- list("\\w", "\\W", "\\s", "\\d", "\\b", "\\B", "\\<", "\\>",
                  "^", "$", "\\1", "\\2", ".")
  quants <- list("*", "+", "?", "{2}", "{1,2}", "{0,1}", "*?", "+?", "??",
                 "{2,}", "{0}")
  approx <- list("{~1}", "{~}", "{~2}", "{#1}", "{+1}", "{-1}", "{+1-1}",
                 "{~2,1i}", "{1i+2d+1s<3}", "{2s+1d<2}", "{~1,2i+1d}")
  if (afuzz) quants <- c(quants, approx, approx)
  gen <- function(depth) {
    alt <- function() {
      n <- sample.int(3, 1)
      paste0(vapply(seq_len(n), function(i) {
        r <- unif()
        a <- if (r < 0.4) pick(lits)
        else if (r < 0.55) pick(classes)
        else if (r < 0.7) pick(escapes)
        else if (depth > 0) paste0("(", gen(depth - 1), ")",
                                   if (afuzz && unif() < 0.5) pick(approx) else "")
        else pick(lits)
        if (unif() < 0.35) paste0(a, pick(quants)) else a
      }, ""), collapse = "")
    }
    if (unif() < 0.25) paste0(alt(), "|", alt()) else alt()
  }
  pats <- lapply(1:1200, function(i) {
    p <- gen(2)
    if (grepl("[^ -~]", p, useBytes = TRUE)) utf8(p) else p
  })
  texts <- c(lapply(1:23, function(i) {
    k <- sample.int(8, 1) - 1
    t <- paste0(vapply(seq_len(k), function(j)
      pick(list("a", "b", "c", "A", u, " ", "ab", "x")), ""), collapse = "")
    if (grepl("[^ -~]", t, useBytes = TRUE)) utf8(t) else t
  }), NA)
  # wide matching in the UTF-8 locale; bytes in the C locale
  flagsets <- function(pi) if (l10n_info()$MBCS) c(0, 2) else c(0, 1)
}

for (i in seq_along(texts)) {
  t <- texts[[i]]
  if (is.na(t)) emit("T", i, "NA", "NA") else emit("T", i, enc(t), pct(t))
}
for (i in seq_along(pats)) emit("P", i, enc(pats[[i]]), pct(pats[[i]]))

one <- function(p, t, fixed, icase, bytes) {
  warns <- character()
  r <- withCallingHandlers(
    tryCatch(regexec(p, t, ignore.case = icase, fixed = fixed, useBytes = bytes)[[1]],
             error = function(e) structure(conditionMessage(e), class = "err")),
    warning = function(w) {
      warns <<- c(warns, conditionMessage(w))
      invokeRestart("muffleWarning")
    })
  out <- if (inherits(r, "err")) paste0("E", pct(unclass(r)))
  else if (is.na(r[1])) "NA"
  else {
    m <- if (r[1] == -1) "-"
    else paste(r, attr(r, "match.length"), sep = ":", collapse = ",")
    it <- attr(r, "index.type")
    if (is.null(it)) m
    else {
      stopifnot(isTRUE(attr(r, "useBytes")))
      paste0(if (it == "bytes") "b" else "c", m)
    }
  }
  w <- if (length(warns)) paste0("W", vapply(warns, pct, ""), "|", collapse = "") else ""
  paste0(w, out)
}

for (pi in seq_along(pats)) {
  for (flags in flagsets(pi)) {
    fixed <- bitwAnd(flags, 4) > 0
    icase <- bitwAnd(flags, 2) > 0
    bytes <- bitwAnd(flags, 1) > 0
    rs <- vapply(texts, function(t) one(pats[[pi]], t, fixed, icase, bytes), "",
                 USE.NAMES = FALSE)
    r <- rle(rs)
    rs <- ifelse(r$lengths > 1, paste0(r$values, "~", r$lengths), r$values)
    emit("R", pi, paste0(as.integer(fixed), as.integer(icase), as.integer(bytes)),
         paste(rs, collapse = "\t"))
  }
}
