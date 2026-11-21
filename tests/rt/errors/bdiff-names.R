# zero-length names: installTrChar refuses them
e <- new.env()
tryCatch(as.name(""), error = function(e) conditionMessage(e))
tryCatch(lockBinding("", e), error = function(e) conditionMessage(e))
tryCatch(.Internal(remove("", e, FALSE)), error = function(e) conditionMessage(e))
tryCatch(get("", e), error = function(e) conditionMessage(e))
tryCatch(exists("", e), error = function(e) conditionMessage(e))
tryCatch(assign("", 1, e), error = function(e) conditionMessage(e))
# latin1 and bytes names in messages
l <- "caf\xe9"; Encoding(l) <- "latin1"
b <- "caf\xe9"; Encoding(b) <- "bytes"
tryCatch(.Internal(get("a", e, l, TRUE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(get("a", e, b, TRUE)), error = function(e) conditionMessage(e))
tryCatch(.Internal(remove(l, e, FALSE)), warning = function(e) conditionMessage(e))
tryCatch(.Internal(remove(b, e, FALSE)), error = function(e) conditionMessage(e), warning = function(e) conditionMessage(e))
tryCatch(lockBinding(l, e), error = function(e) conditionMessage(e))
tryCatch(lockBinding(b, e), error = function(e) conditionMessage(e))
tryCatch(get(l, e), error = function(e) conditionMessage(e))
# .Internal(lapply/vapply) outside a closure with '...'
f <- function(x, ...) x
tryCatch(.Internal(lapply(TRUE, f)), error = function(e) conditionMessage(e))
tryCatch(.Internal(vapply(TRUE, f, logical(1), TRUE)), error = function(e) conditionMessage(e))
g <- function(X, FUN, ...) .Internal(lapply(X, FUN)); g(1:2, function(x, ...) x + 1)
# Sys.getenv of a bytes name
tryCatch(.Internal(Sys.getenv(b, "")), error = function(e) conditionMessage(e))
tryCatch(.Internal(Sys.getenv(l, "")), error = function(e) conditionMessage(e))
# a long name in "object '...' not found"
nm <- strrep("x", 1200)
m <- tryCatch(get(nm, e), error = function(e) conditionMessage(e)); nchar(m); substr(m, 990, 1010)
m <- tryCatch(eval(as.name(strrep("y", 9000))), error = function(e) conditionMessage(e)); nchar(m)
# Sys.getenv: NA looks up "NA", "" and names with '=' find nothing set
do.call(Sys.setenv, structure(list("na-value"), names = "NA")); .Internal(Sys.getenv(c(NA, "", "NA"), "<unset>"))
Sys.setenv(ROBIGO_EQ = "a=b"); .Internal(Sys.getenv(c("ROBIGO_EQ=a", "ROBIGO_EQ"), "<unset>"))
Sys.setenv(café = "latin1-name"); .Internal(Sys.getenv(l, "<unset>"))
# names<- of a pairlist and a call installs each name
p <- pairlist(1, 2); tryCatch(names(p) <- c("a", b), error = function(e) conditionMessage(e))
names(p) <- c("a", l); names(p)
cl <- quote(f(1, 2)); names(cl) <- c("", "x", l); cl
# error()'s buffer cuts a multibyte name short of the character it splits
m <- tryCatch(get(strrep("é", 600), e), error = function(e) conditionMessage(e)); nchar(m, "bytes"); nchar(m)
op <- options(warning.length = 200)
m <- tryCatch(get(nm, e), error = function(e) conditionMessage(e)); nchar(m)
options(op)
m <- tryCatch(stop(strrep("z", 1500)), error = function(e) conditionMessage(e)); nchar(m)
