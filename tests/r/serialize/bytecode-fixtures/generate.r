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
