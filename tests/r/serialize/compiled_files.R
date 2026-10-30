if (dir.exists('tests/r/serialize/compiled-fixtures')) setwd('tests/r/serialize')
err <- function(e) tryCatch(e,error=function(e) conditionMessage(e))
for (v in 2:3) for (format in c('ascii','xdr','binary')) {
  path <- sprintf('compiled-fixtures/data-%d-%s.Rc',v,format)
  x <- .Internal(load.from.file(path))
  print(x)
  out <- tempfile()
  if (format != 'binary') {
    print(withVisible(.Internal(save.to.file(x,out,format=='ascii',v)))$visible)
    print(identical(readBin(out,'raw',100000L),readBin(path,'raw',100000L)))
    print(identical(.Internal(load.from.file(out)),x))
  }
  unlink(out)
}
p <- tempfile()
for (file in list('',character(),NA_character_,1L)) print(err(.Internal(save.to.file(NULL,file,FALSE,3L))))
print(err(.Internal(save.to.file(NULL,p,1L,3L))))
for (v in list(0L,NA_integer_,-1L,4L)) print(err(.Internal(save.to.file(NULL,p,FALSE,v))))
for (file in list('',character(),NA_character_)) print(err(.Internal(load.from.file(file))))
for (bytes in list(raw(),charToRaw('RD'),charToRaw('XXXXX'),charToRaw('RDX9\n'))) {
 writeBin(bytes,p)
 print(err(.Internal(load.from.file(p))))
}
unlink(p)
