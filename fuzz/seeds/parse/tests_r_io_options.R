getOption("digits")
getOption("width")
getOption("no.such.option")
getOption("no.such.option", "fallback")
op <- options(digits = 3)
op
pi
1/7
options(op)
pi
x <- options("digits", "width")
x
str1 <- options("OutDec")
str1$OutDec
old <- options(my.option = 42)
old
getOption("my.option")
options(my.option = NULL)
getOption("my.option")
o <- options(list(digits = 4, scipen = 2))
o
1e5
123456.789
options(o)
1e5
getOption("scipen")
class(getOption("warn"))
options(warn = 1)
getOption("warn")
options(warn = 0)
is.list(options())
"digits" %in% names(options())
tryCatch(options(digits = 30), error = function(e) conditionMessage(e))
tryCatch(options(width = 5), error = function(e) conditionMessage(e))
tryCatch(options(digits = NULL), error = function(e) conditionMessage(e))
tryCatch(options(warn = "a"), error = function(e) conditionMessage(e))
options(scipen = 10000)
getOption("scipen")
options(scipen = 0)
f <- function() {
  op <- options(digits = 2)
  on.exit(options(op))
  print(pi)
}
f()
pi
