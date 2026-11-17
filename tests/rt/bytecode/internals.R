# eval.c's byte-code internals (docs/g3.md §6.1): mkCode's encoding checks
# (R_bcEncode: the version installed, a version mismatch, an unknown
# instruction), disassemble, bcVersion, the constant buffer's growconst,
# putconst (which writes its buffer in place, through every reference) and
# getconst, and is.builtin.internal. Closures stay inside local(): the
# base-subset tracer rewrites closures bound in the global environment.
print(.Internal(bcVersion()))
mk <- function(code, constants) .Internal(mkCode(code, constants))
for (v in c(8L, 9L, 12L, 13L)) print(.Internal(disassemble(mk(c(v, 16L, 0L, 1L), list(42L)))))
print(.Internal(disassemble(mk(c(12L, 0L), list(1)))))
print(.Internal(disassemble(mk(integer(0), list(1)))))
print(.Internal(disassemble(mk(c(12, 16, 0, 1), list(1)))))
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
print(err(mk(c(12L, 999L), list(1))))
print(err(mk(c(12L, -1L), list(1))))
print(typeof(mk(c(12L, 16L), list(1))))
b <- mk(c(12L, 16L, 0L, 1L), 42L)
print(typeof(b))
print(err(.Internal(disassemble(b))))
b <- mk(c(12L, 16L, 0L, 1L), list(42L))
print(typeof(b))
print(length(b))
print(deparse(b))
print(identical(b, mk(c(12L, 16L, 0L, 1L), list(42L))))
print(identical(b, mk(c(12L, 16L, 0L, 1L), list(43L))))
print(identical(b, mk(c(12L, 16L, 0L, 0L), list(42L))))
inner <- mk(c(12L, 16L, 0L, 1L), list(quote(x)))
print(.Internal(disassemble(mk(c(12L, 16L, 1L, 1L), list(quote(f(x)), inner, 1:3)))))
print(.Internal(growconst(structure(list(1L, 2L), names = c("a", "b")))))
print(.Internal(getconst(list(1L, 2L), 1L)))
print(.Internal(is.builtin.internal(quote(mean))))
print(.Internal(is.builtin.internal(as.name("if"))))
print(.Internal(is.builtin.internal(as.name("vapply"))))
print(.Internal(is.builtin.internal(as.name("paste0"))))
print(.Internal(is.builtin.internal(as.name("sum"))))
print(err(.Internal(disassemble(1))))
print(err(.Internal(is.builtin.internal("mean"))))
print(err(.Internal(growconst(1))))
print(err(.Internal(getconst(1, -1))))
print(err(.Internal(getconst(list(1), -1))))
print(err(.Internal(getconst(list(1), 2L))))
print(.Internal(getconst(list(1L, 2L), "1")))
pool <- list(10L, 20L, quote(x + 1L), quote(x), 1L,
             structure(c(NA, 1L, 1L, 2L, 2L, 0L, 0L, 0L), class = "expressionsIndex"))
print(.Internal(putconst(pool, 5L, 20L)))
print(.Internal(putconst(pool, 5L, quote(x))))
print(.Internal(putconst(pool, 5L, 1L)))
print(.Internal(putconst(list(10L, 20L, quote(x + 1L), quote(x), 1L, NULL), 4L, 1L)))
print(err(.Internal(putconst(1L, 0L, 1L))))
print(err(.Internal(putconst(pool, -1L, 1L))))
print(err(.Internal(putconst(pool, 6L, 1L))))
alias_target <- list(10L, 20L, NULL, NULL)
alias_observer <- alias_target
print(.Internal(putconst(alias_target, 2L, quote(mutated_val))))
print(identical(alias_target, alias_observer))
print(alias_observer[[3]])
local({
  g <- function(x) x
  print(.Internal(putconst(list(g, NULL), 1L, function(x) x)))
  print(.Internal(putconst(list(g, NULL), 1L, g)))
})
cbuf <- vector("list", 1L)
ccount <- 0L
for (c in list(100L, 200L, 100L, quote(a + b), quote(a), 200L, "hello", 1L)) {
  if (ccount == length(cbuf))
    cbuf <- .Internal(growconst(cbuf))
  idx <- .Internal(putconst(cbuf, ccount, c))
  if (idx == ccount)
    ccount <- ccount + 1L
  print(c(idx, ccount, length(cbuf)))
}
print(.Internal(getconst(cbuf, ccount)))
