# gram.y's parseError condition on each path that parses: parse(text = ),
# parse(file = ), str2lang() and str2expression() (source() parses through
# parse(file = ); robigo-rt's source() needs the utils namespace first,
# INTERIM(L5)). Without source references the filename is getFilename()'s
# "<input>" on every path (gram.y:4426). Handlers find it by its classes;
# the value, line and column are the file's.
show <- function(e) cat(class(e)[1], "|", deparse(e$value), "|", e$filename, e$lineno, e$colno, "|", conditionMessage(e), "\n")
f <- tempfile(fileext = ".R")
writeLines(c("x <- 1", "y <- function(a, b, a) 2"), f)
tryCatch(parse(file = f), error = show)
tryCatch(parse(text = "1 |> f(_)"), error = show)
tryCatch(str2lang("1 |> f(_)"), error = show)
tryCatch(str2expression(c("1", "\"\\q\"")), error = show)
# by subclass, by "parseError", and a calling handler before the default
r <- tryCatch(parse(text = "1 |> f"), RHSnotFnCall = function(e) "the subclass")
print(r)
r <- tryCatch(parse(text = "\"\\0\""), parseError = function(e) "parseError")
print(r)
r <- try(withCallingHandlers(parse(text = "function(a, a) 1"),
  parseError = function(e) cat("calling:", class(e)[1], "\n")), silent = TRUE)
cat(r)
# a grammar error stays a simpleError, reported against parse()'s call
r <- tryCatch(parse(text = "x y"), error = function(e) class(e))
print(r)
