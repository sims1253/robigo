# Rdynload.c's getSymbolInfo (getNativeSymbolInfo, `$.DLLInfo`): base forces
# symbols, so a lookup by package name finds nothing; its DllInfo reference
# finds the registered routine.
b <- getLoadedDLLs()[["base"]]
s <- getNativeSymbolInfo("R_addTaskCallback", b)
class(s)
names(s)
s$name
class(s$address)
s$dll[["name"]]
s <- getNativeSymbolInfo("dqrdc2", b, withRegistrationInfo = TRUE)
class(s)
names(s)
s$numParameters
class(s$address)
s <- b$dtrco
class(s)
s$name
x <- getNativeSymbolInfo(c("dqrcf", "dqrxb"), b)
names(x)
vapply(x, function(s) s$name, "")
is.null(.Internal(getSymbolInfo("R_addTaskCallback", "base", FALSE)))
is.null(.Internal(getSymbolInfo("R_addTaskCallback", "", FALSE)))
is.null(.Internal(getSymbolInfo("nosuchsymbol", b[["info"]], TRUE)))
tryCatch(getNativeSymbolInfo("R_addTaskCallback", "base"), error = function(e) conditionMessage(e))
tryCatch(b$nosuchsymbol, error = function(e) conditionMessage(e))
tryCatch(.Internal(getSymbolInfo(1, "", FALSE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(getSymbolInfo(c("a", "b"), "", FALSE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(getSymbolInfo("a", 1, FALSE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(getSymbolInfo("a", b[["handle"]], FALSE)), error = function(e) conditionMessage(e))
# names are translated (translateCharFP): "bytes" is refused
s <- "é"
Encoding(s) <- "bytes"
f <- function(n, p) .Internal(getSymbolInfo(n, p, FALSE))
tryCatch(f(s, b[["info"]]), error = function(e) conditionMessage(e))
tryCatch(f("dtrco", s), error = function(e) conditionMessage(e))
tryCatch(f(s, ""), error = function(e) conditionMessage(e))
# and what iconv cannot convert (latin1 is CP1252, where 0x81 is unmapped)
l <- "\x81"
Encoding(l) <- "latin1"
tryCatch(f(l, b[["info"]]), error = function(e) conditionMessage(e))
tryCatch(f("dtrco", l), error = function(e) conditionMessage(e))
f(s, "")
