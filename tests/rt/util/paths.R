# util.c's basename, dirname, normalizePath and getwd (Unix).
b <- function(x) .Internal(basename(x))
d <- function(x) .Internal(dirname(x))
x <- c("a/b/c", "a/b/", "/", "///", "a", "", NA, "/a", "a//b", "//a", "a/b//", "./x", "../y/", "x.tar.gz")
b(x)
d(x)
b(character(0))
d(character(0))
# "~" is expanded
identical(d("~/x"), d(paste0(Sys.getenv("HOME"), "/x")))
f <- function(fun, x) tryCatch(fun(x), error = function(e) conditionMessage(e))
f(b, 1)
f(d, NULL)
s <- "é"
Encoding(s) <- "bytes"
f(b, s)
f(d, s)
b("été/ü")
# normalizePath: relations that hold wherever the script runs
np <- function(x, mustWork = NA) .Internal(normalizePath(x, "\\", mustWork))
w <- .Internal(getwd())
is.character(w) && length(w) == 1L && startsWith(w, "/")
identical(np("."), w)
identical(np(c(".", "./.")), c(w, w))
identical(np(".."), d(w))
np(c("nosuch/x", NA), mustWork = FALSE)
r <- withCallingHandlers(np(c("nosuch/x", NA), mustWork = NA),
                         warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
r
f(function(x) np(x, mustWork = TRUE), "nosuch/x")
f(function(x) np(x, mustWork = TRUE), NA_character_)
f(np, 1)
f(np, s)
tryCatch(.Internal(getwd(1)), error = function(e) conditionMessage(e))
# ~user is that user's home directory
d(c("~root/x", "~root", "~nosuchuserxyz/x"))
b(c("~root/x", "~nosuchuserxyz"))
