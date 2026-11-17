# Byte-code translation cases: GNU R's own installed byte code of base
# functions (their code is base's, compiled in base's namespace: no guards),
# rebuilt and called (scripts/bc-fixtures.R).
b <- function(name, calls) list(name = paste0("b_", gsub("[^A-Za-z0-9]", "_", name)), fun = get(name, baseenv()),
                                calls = gsub("@", paste0("b_", gsub("[^A-Za-z0-9]", "_", name)), calls, fixed = TRUE),
                                env = ".BaseNamespaceEnv")
cases <- list(
  b("paste0", c("print(@('a', 1:3))", "print(@('x', c('p', 'q'), collapse = '+'))")),
  b("lapply", "print(@(1:3, function(i) i * 2))"),
  b("sapply", c("print(@(1:3, function(i) i^2))", "print(@(list(), length))")),
  b("vapply", "print(@(c(a = 1, b = 4), sqrt, 0))"),
  b("Filter", "print(@(function(x) x > 2, c(1, 5, 2, 7)))"),
  b("match.arg", c("f <- function(type = c('linear', 'quad')) b_match_arg(type)", "print(f())", "print(f('quad'))")),
  b("rev.default", "print(@(c(1, 9, 3)))"),
  b("ifelse", "print(@(c(TRUE, FALSE, NA), 'y', 'n'))"),
  b("do.call", "print(@('paste', list('a', 'b', sep = '-')))"),
  b("nchar", "print(@(c('abc', 'de')))"),
  b("startsWith", "print(@(c('apple', 'banana'), 'a'))"),
  b("trimws", c("print(@('  hi  '))", "print(@('xx--', 'right', '[-]'))")),
  b("rle", "print(unclass(@(c(1, 1, 2, 2, 2, 3))))"),
  b("which", c("print(@(c(FALSE, TRUE, NA, TRUE)))", "print(@(c(a = TRUE, b = FALSE, c = TRUE)))")),
  b("unique.default", "print(@(c(3, 1, 3, 2, 1)))"),
  b("setdiff", "print(@(1:6, c(2, 4)))"),
  b("union", "print(@(c(1, 2), c(2, 3)))"),
  b("vapply", "print(@(list(1:3, 4:6), function(v) sum(v), numeric(1)))"),
  b("rep_len", "print(@(1:3, 7))"),
  b("Negate", "print(@(is.null)(NULL))"),
  b("isTRUE", c("print(@(TRUE))", "print(@(c(TRUE, TRUE)))")),
  b("xor", "print(@(TRUE, c(TRUE, FALSE)))"),
  b("append", "print(@(1:3, 99, after = 1))"),
  b("seq.default", "print(@(1, 10, by = 3))")
)
# (base's closures only: a primitive has no byte code)
cases <- Filter(function(cs) typeof(cs$fun) == "closure", cases)
