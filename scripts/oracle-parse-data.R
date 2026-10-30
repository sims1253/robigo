# GNU R 4.5.0 raw parseData records, including token and reduction IDs.
# docker run --rm -v "$PWD:/work" -w /work robigo-oracle:4.5.0 \
#   Rscript --vanilla scripts/oracle-parse-data.R > tests/parse/data.tsv
hex <- function(x) paste(sprintf('%02x', as.integer(charToRaw(x))), collapse='')
sources <- c(
  'x + 1', 'x = 1', 'x     = 1', 'x = x + 1', 'x <- x + 1',
  '{ symbol }', 'foo(1,g,b=a)', 'attr(x,"mya") <- bar()',
  'foo() %myop% {}', 'foo %myop% 42', 'foo[[bar=xyz %myop% 42]]',
  '1 + 2 * 3', 'a <- b <- 1', '1 -> x ->> y', 'x := 1',
  '(x)', '((x))', '-x^2', '!x && y || z', '?x', 'x ? y',
  'pkg::fun(x)', 'pkg:::fun()', 'x$name', 'x@slot',
  'f(,x,,a=,NULL=1,"b"=2)', 'x[1, , drop=FALSE]', 'x[[1]]',
  'function(x, y=1, ...) x+y', 'function() {}',
  '{x; y;}', '{\nx\ny\n}', 'if (x) y else z',
  'for (x in 1:3) x', 'while (x) x <- x-1', 'repeat break',
  'f(\n1,\n2\n)', 'x +\n 1', '# before\nx # after\n# tail',
  '{ # inner\nx\n# between\ny\n}', 'x |> f()',
  'TRUE; FALSE; NA; NULL; NA_integer_; 1L; 2i; "a"',
  '`a b` + 1', 'x\t+\ty', '"é" + café',
  '{ if(x) y\nelse z }', '{ if(x) y\nz }', '#line 20 \"x.R\"\nx', '#line 2\nx', '#line not-a-number\nx', '#line20\nx', ' #line 20\nx', '{;}', '{;;}', '{\n;\n}', 'f()', 'f(,)', '{}', '# comment;'
)
for (src in sources) {
  src <- paste0(src, '\n')
  cat('S', hex(src), '\n', sep='\t')
  parsed <- parse(text=src, keep.source=TRUE)
  sf <- attr(parsed, 'srcfile')
  if (!is.null(sf$original)) sf <- sf$original
  data <- sf$parseData
  for (j in seq_len(ncol(data))) {
    cat(paste(data[,j], collapse='\t'), attr(data,'tokens')[j],
        hex(attr(data,'text')[j]), '\n', sep='\t')
  }
}
