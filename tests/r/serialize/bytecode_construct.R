if (dir.exists('tests/r/serialize/bytecode-fixtures')) setwd('tests/r/serialize')
print(.Internal(bcVersion()))
b0 <- .Internal(mkCode(structure(c(12L,16L,0L,1L),note='discard'),list(42L)))
print(identical(serialize(b0,NULL),readBin('bytecode-fixtures/constant-code.rds','raw',n=100000L)))
mk <- function(code, constants) .Internal(mkCode(code, constants))
for (v in c(9L, 12L, 13L)) {
  b <- mk(c(v,16L,0L,1L), list(42L))
  print(.Internal(disassemble(b)))
}
b <- mk(c(12L,16L,0L,1L),list(42L))
for (version in 2:3) {
  f <- .Internal(bcClose(pairlist(x=1L),b,globalenv()))
  print(f())
  print(body(f))
  path <- sprintf('bytecode-fixtures/constructed-v%d.rds',version)
  expected <- readBin(path,'raw',n=100000L)
  print(identical(serialize(f,NULL,version=version),expected))
  out <- tempfile()
  saveRDS(f,out,version=version,compress=FALSE)
  print(identical(readBin(out,'raw',n=100000L),expected))
  print(identical(serialize(readRDS(path),NULL,version=version),expected))
  unlink(out)
}
x <- quote(f(a=x, x+1L))
attr(x, 'note') <- 'shared language'
constants <- list(x,x, pairlist(a=x,b=2L), b, TRUE, NA_integer_, 1:3)
b <- mk(c(12L,16L,0L,1L),constants)
for (version in 2:3) {
  expected <- readBin(sprintf('bytecode-fixtures/constructed-code-v%d.rds',version),'raw',n=100000L)
  print(identical(serialize(b,NULL,version=version),expected))
  print(identical(serialize(unserialize(expected),NULL,version=version),expected))
}
print(.Internal(disassemble(b)))
print(.Internal(growconst(structure(list(1L,2L), names=c('a','b')))))
print(.Internal(getconst(list(1L,2L),1L)))
print(.Internal(is.builtin.internal(quote(mean))))
print(.Internal(is.builtin.internal(as.name('if'))))
err <- function(expr) tryCatch(expr,error=function(e) conditionMessage(e))
print(err(.Internal(bcClose(list(x=1),42L,NULL))))
print(err(.Internal(bcClose(pairlist(1),42L,NULL))))
print(err(.Internal(bcClose(NULL,42L,NULL))))
print(err(.Internal(bcClose(NULL,b,NULL))))
print(err(.Internal(bcClose(NULL,b,1))))
print(err(.Internal(disassemble(1))))
print(err(.Internal(is.builtin.internal('mean'))))
print(err(.Internal(growconst(1))))
print(err(.Internal(getconst(1,-1))))
print(err(.Internal(getconst(list(1),-1))))
print(err(mk(c(12L,999L),list(1))))

print(err(.Internal(bcVersion(1L))))
print(err(.Internal(mkCode())))
print(err(.Internal(bcClose(NULL,b))))
print(err(.Internal(disassemble())))
print(err(.Internal(growconst())))
print(err(.Internal(getconst(list()))))
print(err(.Internal(is.builtin.internal())))
print(.Internal(getconst(list(1L,2L),c('1','bad'))))
print(err(.Internal(getconst(list(1L),list(1L)))))
b <- readRDS('bytecode-fixtures/body-code.rds')
b <- mk(c(12L,16L,0L,1L),list(42L,b))
print(identical(serialize(b,NULL),readBin('bytecode-fixtures/constructed-imported-inner.rds','raw',n=100000L)))
compiler::disassemble(.Internal(bcClose(NULL,mk(c(12L,16L,0L,1L),list(42L)),globalenv())))
cf <- readRDS('bytecode-fixtures/cmpfun-simple.rds')
print(cf(5L))
print(body(cf))
print(typeof(.Internal(bodyCode(cf))))
compiler::disassemble(cf)
pool <- list(10L, 20L, quote(x + 1L), quote(x), 1L, structure(c(NA, 1L, 1L, 2L, 2L, 0L, 0L, 0L), class = 'expressionsIndex'))
print(.Internal(putconst(pool, 5L, 20L)))
print(.Internal(putconst(pool, 5L, quote(x))))
print(.Internal(putconst(pool, 5L, 1L)))
print(.Internal(putconst(list(10L, 20L, quote(x + 1L), quote(x), 1L, NULL), 4L, 1L)))
print(err(.Internal(putconst(1L, 0L, 1L))))
print(err(.Internal(putconst(pool, -1L, 1L))))
print(err(.Internal(putconst(pool, 6L, 1L))))
print(err(.Internal(putconst())))
print(err(.Internal(putconst(pool))))
print(err(.Internal(putconst(pool, 0L))))
print(err(.Internal(putconst(pool, 0L, 1L, 2L))))

# Live compiler::cmpfun constant-layout comparisons
cf_live <- compiler::cmpfun(function(x) x + 1L)
print(cf_live(5L))
print(body(cf_live))
print(typeof(.Internal(bodyCode(cf_live))))
print(identical(compiler::disassemble(cf_live), compiler::disassemble(cf)))
compiler::disassemble(cf_live)

cf_math <- compiler::cmpfun(function(x, y = 2L) { z <- 10L; (x + y) * z + 1L })
print(cf_math(3L))
print(cf_math(3L, 4L))
compiler::disassemble(cf_math)

cf_nested <- compiler::cmpfun(function(x = 3L) {
  g <- function(y) y + 1L
  for (i in seq_len(x)) x <- g(x)
  x
})
print(cf_nested(1L))
print(cf_nested(4L))
compiler::disassemble(cf_nested)

# Constant buffer growth and extraction via growconst / putconst / getconst
cbuf <- vector("list", 1L)
ccount <- 0L
test_consts <- list(
  100L, 200L, 100L, quote(a + b), quote(a), 200L, "hello", 1L,
  structure(c(NA, 1L, 1L, 2L, 2L, 0L, 0L, 0L), class = "expressionsIndex")
)
for (c in test_consts) {
  if (ccount == length(cbuf))
    cbuf <- .Internal(growconst(cbuf))
  idx <- .Internal(putconst(cbuf, ccount, c))
  if (idx == ccount)
    ccount <- ccount + 1L
  print(c(idx, ccount, length(cbuf)))
}
print(.Internal(getconst(cbuf, ccount)))

# Aliased buffer mutation in putconst
alias_target <- list(10L, 20L, NULL, NULL)
alias_observer <- alias_target
print(.Internal(putconst(alias_target, 2L, quote(mutated_val))))
print(identical(alias_target, alias_observer))
print(alias_target[[3]])
print(alias_observer[[3]])

