objects <- list(data=list(c(NA_integer_, -1L, 0L, 42L), c(NA_real_, NaN, Inf, -Inf, -0, pi, 1e-300), c('a b', 'a\n\t\\?\"', 'é', NA_character_), as.raw(c(0, 10, 255)), quote(f(x=1L))), closure=compiler::cmpfun(function(x=1L) x+42L))
for (name in names(objects)) for (v in 2:3) for (a in c(FALSE,TRUE)) {
 p <- sprintf('tests/r/serialize/compiled-fixtures/%s-%d-%s.Rc', name, v, if(a) 'ascii' else 'xdr')
 .Internal(save.to.file(objects[[name]],p,a,v))
 if (!a) {
   b <- c(charToRaw(sprintf('RDB%d\n',v)),serialize(objects[[name]],NULL,xdr=FALSE,version=v))
   writeBin(b,sub('xdr','binary',p))
 }
}
writeLines('loaded_compiled <- function(x=1L) x+42L', 'tests/r/serialize/compiled-fixtures/source.txt')
compiler::cmpfile('tests/r/serialize/compiled-fixtures/source.txt', 'tests/r/serialize/compiled-fixtures/source.Rc')
