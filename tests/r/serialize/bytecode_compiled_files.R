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

# End-to-end cmpfun -> cmpfile -> loadcmp round-trip:
# 1. Compile source.txt with cmpfile (default binary)
src_file <- 'compiled-fixtures/source.txt'
out_file <- 'compiled-fixtures/test-out.Rc'
compiler::cmpfile(src_file, out_file)
# 2. Compare file bytes against oracle fixture written by GNU R
oracle_file <- 'compiled-fixtures/source.Rc'
print(identical(readBin(out_file, 'raw', 100000L), readBin(oracle_file, 'raw', 100000L)))
# 3. Load generated file in fresh environment with loadcmp
env1 <- new.env(parent = globalenv())
compiler::loadcmp(out_file, envir = env1)
print(env1$loaded_compiled())
print(env1$loaded_compiled(5L))
unlink(out_file)

# Round-trip with ascii = TRUE
out_ascii <- 'compiled-fixtures/test-out-ascii.Rc'
compiler::cmpfile(src_file, out_ascii, ascii = TRUE)
env_ascii <- new.env(parent = globalenv())
compiler::loadcmp(out_ascii, envir = env_ascii)
print(env_ascii$loaded_compiled(7L))
unlink(out_ascii)

# Round-trip with version = 2 and version = 3
for (ver in 2:3) {
  out_ver <- sprintf('compiled-fixtures/test-out-v%d.Rc', ver)
  compiler::cmpfile(src_file, out_ver, version = ver)
  env_ver <- new.env(parent = globalenv())
  compiler::loadcmp(out_ver, envir = env_ver)
  print(env_ver$loaded_compiled(10L * ver))
  unlink(out_ver)
}

# Round-trip starting from cmpfun: compile function, write source, cmpfile, loadcmp
f_sq <- compiler::cmpfun(function(a, b = 3L) a * a + b)
print(f_sq(4L))
print(f_sq(4L, 10L))
sq_src <- 'compiled-fixtures/test-sq.R'
writeLines('sq_func <- function(a, b = 3L) a * a + b', sq_src)
sq_out <- 'compiled-fixtures/test-sq.Rc'
compiler::cmpfile(sq_src, sq_out)
sq_env <- new.env(parent = globalenv())
compiler::loadcmp(sq_out, envir = sq_env)
print(sq_env$sq_func(4L))
print(sq_env$sq_func(4L, 10L))
unlink(sq_src)
unlink(sq_out)


