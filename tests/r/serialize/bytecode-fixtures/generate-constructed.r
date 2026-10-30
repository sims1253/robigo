p <- 'tests/r/serialize/bytecode-fixtures/'
b <- .Internal(mkCode(c(12L,16L,0L,1L),list(42L)))
f <- .Internal(bcClose(pairlist(x=1L),b,globalenv()))
for (v in 2:3) saveRDS(f,paste0(p,'constructed-v',v,'.rds'),version=v,compress=FALSE)
x <- quote(f(a=x,x+1L))
attr(x, 'note') <- 'shared language'
constants <- list(x,x,pairlist(a=x,b=2L),b,TRUE,NA_integer_,1:3)
b <- .Internal(mkCode(c(12L,16L,0L,1L),constants))
for (v in 2:3) saveRDS(b,paste0(p,'constructed-code-v',v,'.rds'),version=v,compress=FALSE)
b <- readRDS(paste0(p,'body-code.rds'))
b <- .Internal(mkCode(c(12L,16L,0L,1L),list(42L,b)))
saveRDS(b,paste0(p,'constructed-imported-inner.rds'),compress=FALSE)
fcmp <- compiler::cmpfun(function(x) x + 1L)
saveRDS(fcmp,paste0(p,'cmpfun-simple.rds'),compress=FALSE)
