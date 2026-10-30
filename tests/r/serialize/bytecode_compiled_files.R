if (dir.exists('tests/r/serialize/compiled-fixtures')) setwd('tests/r/serialize')
for (v in 2:3) for (format in c('ascii','xdr','binary')) {
 path <- sprintf('compiled-fixtures/closure-%d-%s.Rc',v,format)
 f <- .Internal(load.from.file(path))
 print(f())
 print(f(10L))
 if (format != 'binary') {
  out <- tempfile()
  .Internal(save.to.file(f,out,format=='ascii',v))
  print(identical(readBin(out,'raw',100000L),readBin(path,'raw',100000L)))
  print(.Internal(load.from.file(out))(2L))
  unlink(out)
 }
}
compiler::loadcmp('compiled-fixtures/source.Rc')
print(loaded_compiled())
print(loaded_compiled(10L))
