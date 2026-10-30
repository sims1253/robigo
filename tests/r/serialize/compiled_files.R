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
print(readBin(p,'raw',100L))
for (file in list('',character(),NA_character_)) print(err(.Internal(load.from.file(file))))
for (bytes in list(raw(),charToRaw('RD'),charToRaw('XXXXX'),charToRaw('RDX9\n'))) {
 writeBin(bytes,p)
 print(err(.Internal(load.from.file(p))))
}
unlink(p)

p <- tempfile()
for (bytes in list(charToRaw('RDX3\n'),charToRaw('RDX3\nX'),charToRaw('RDX3\nX\n'),charToRaw('RDX3\nA\n'),charToRaw('RDA3\nB\n'),charToRaw('RDX3\n?\n'))) {
 writeBin(bytes,p)
 print(err(.Internal(load.from.file(p))))
}
unlink(p)

Sys.setenv(R_DEFAULT_SAVE_VERSION='2suffix')
p <- tempfile()
.Internal(save.to.file(42L,p,FALSE,NULL))
print(rawToChar(readBin(p,'raw',5L)))
Sys.setenv(R_DEFAULT_SAVE_VERSION='3')
.Internal(save.to.file(42L,p,FALSE,NULL))
print(rawToChar(readBin(p,'raw',5L)))
unlink(p)
Sys.unsetenv('R_DEFAULT_SAVE_VERSION')
p <- tempfile()
b <- readBin('compiled-fixtures/data-3-ascii.Rc','raw',100000L)
writeBin(c(b[1:5],charToRaw('\n'),b[-(1:5)]),p)
print(identical(.Internal(load.from.file(p)),.Internal(load.from.file('compiled-fixtures/data-3-ascii.Rc'))))
b[7] <- charToRaw('?')
writeBin(b,p)
print(identical(.Internal(load.from.file(p)),.Internal(load.from.file('compiled-fixtures/data-3-ascii.Rc'))))
unlink(p)

p <- tempfile()
dir.create(p)
print(err(.Internal(load.from.file(p))))
unlink(p,recursive=TRUE)
print(err(.Internal(load.from.file(p))))
print(err(.Internal(save.to.file(NULL,file.path(p,'missing'),FALSE,3L))))
