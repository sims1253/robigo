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
