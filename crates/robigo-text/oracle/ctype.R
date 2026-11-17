# Prints glibc's wide-character classes and case mappings in the running
# locale (record.sh: C.UTF-8), as GNU R 4.6.2's TRE sees them through
# iswctype() and R's toupper()/tolower() through towctrans():
#   "class NAME FIRST LAST"  inclusive hex ranges of code points in the class
#   "upper FROM TO"          towupper(FROM) == TO != FROM
#   "lower FROM TO"          towlower(FROM) == TO != FROM
# Code point 0 cannot be in a string, and grepRaw(re, as.raw(0L)) crashes
# GNU R 4.6.2; glibc puts it in cntrl only, which gen-ctype.py adds.
stopifnot(R.version$major == "4", R.version$minor == "6.2", l10n_info()$`UTF-8`)
classes <- c("alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower",
             "print", "punct", "space", "upper", "xdigit", "combining",
             "combining_level3")
cp <- c(1:0xd7ff, 0xe000:0x10ffff)
s <- intToUtf8(cp, multiple = TRUE)
for (k in classes) {
  re <- sprintf("[[:%s:]]", k)
  m <- grepl(re, s)
  inc <- cp[m]
  if (!length(inc)) next
  brk <- c(1L, which(diff(inc) != 1L) + 1L)
  end <- c(brk[-1L] - 1L, length(inc))
  cat(sprintf("class %s %x %x\n", k, inc[brk], inc[end]), sep = "")
}
# R's utf8towcs rejects the noncharacters, which glibc does not map anyway.
nonchar <- (cp >= 0xfdd0 & cp <= 0xfdef) | bitwAnd(cp, 0xfffe) == 0xfffe
cpc <- cp[!nonchar]
sc <- s[!nonchar]
for (f in c("upper", "lower")) {
  t <- if (f == "upper") toupper(sc) else tolower(sc)
  d <- which(t != sc)
  to <- vapply(t[d], utf8ToInt, 0L, USE.NAMES = FALSE)
  cat(sprintf("%s %x %x\n", f, cpc[d], to), sep = "")
}
