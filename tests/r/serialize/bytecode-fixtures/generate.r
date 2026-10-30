# Run from the repository root in robigo-oracle:4.5.0.
options(keep.source = TRUE)
f <- eval(parse(text = 'function(x = 3L) {\n  # Preserve source and nested bytecode constants.\n  g <- function(y) y + 1L\n  for (i in seq_len(x)) x <- g(x)\n  x\n}', keep.source = TRUE))
f <- compiler::cmpfun(f)
for (version in 2:3) {
  for (ascii in c(FALSE, TRUE)) {
    suffix <- if (ascii) 'ascii' else 'xdr'
    saveRDS(f, sprintf('tests/r/serialize/bytecode-fixtures/closure-v%d-%s.rds', version, suffix),
            version = version, ascii = ascii, compress = FALSE)
  }
}
saveRDS(.Internal(bodyCode(f)), 'tests/r/serialize/bytecode-fixtures/body-code.rds', compress = FALSE)
for (version in 2:3) {
  path <- sprintf('tests/r/serialize/bytecode-fixtures/closure-v%d-ascii', version)
  saveRDS(readRDS(paste0(path, '.rds')), paste0(path, '-read-xdr.rds'),
          version = version, compress = FALSE)
}
# Portable code starts at byte 40 in this version-3 XDR BCODESXP fixture.
path <- 'tests/r/serialize/bytecode-fixtures/'
bytes <- readBin(paste0(path, 'body-code.rds'), 'raw', n = 100000L)
stopifnot(identical(bytes[40:43], as.raw(c(0, 0, 0, 12))))
for (version in c(9L, 13L)) {
  input <- bytes
  input[40:43] <- as.raw(c(0, 0, 0, version))
  file <- paste0(path, 'code-version-', version)
  writeBin(input, paste0(file, '.rds'))
  saveRDS(readRDS(paste0(file, '.rds')), paste0(file, '-canonical.rds'), compress = FALSE)
}
b <- .Internal(mkCode(c(12L, 16L, 0L, 1L), list(42L)))
saveRDS(b, 'tests/r/serialize/bytecode-fixtures/constant-code.rds', compress = FALSE)
b <- readRDS('tests/r/serialize/bytecode-fixtures/constant-code.rds')
attr(b, 'srcref') <- 1:6
attr(b, 'srcfile') <- new.env(parent = emptyenv())
attr(b, 'note') <- 'portable'
saveRDS(b, 'tests/r/serialize/bytecode-fixtures/attributed-code.rds', compress = FALSE)
path <- 'tests/r/serialize/bytecode-fixtures/'
constants <- list(42L, TRUE, FALSE, NA, NA_integer_, -1L,
                  structure(2L, names = 'x'), c(1L, 2L), quote(x), NULL)
b <- .Internal(mkCode(c(12L, 16L, 0L, 1L), constants))
saveRDS(b, paste0(path, 'scalar-constants.rds'), compress = FALSE)
f <- function(x) environment()
g <- compiler::cmpfun(function() { z <- 40L; f(z + 2L) })
e <- g()
saveRDS(e, paste0(path, 'unforced-promise.rds'), compress = FALSE)
force(e$x)
saveRDS(e, paste0(path, 'forced-promise.rds'), compress = FALSE)
# Scalar real bits include negative zero, R NA and IEEE NaN.
b <- .Internal(mkCode(c(12L, 16L, 0L, 1L), list(42, -0, NA_real_, NaN, Inf, -Inf, 2.5, 1+2i)))
saveRDS(b, 'tests/r/serialize/bytecode-fixtures/real-constants.rds', compress = FALSE)
