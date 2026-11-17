# version.c's Version (R.version), platform.c's R.home and dir.exists, and
# paste.c's file.path.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
v <- .Internal(Version())
print(names(v))
for (n in names(v)) cat(n, ":", v[[n]], "\n")
print(.Internal(dir.exists(c("/", "/etc/passwd", "/nonexistent-dir-c1bc", NA, "~"))))
print(.Internal(dir.exists(character())))
print(err(.Internal(dir.exists(1))))
print(.Internal(file.path(list("a", "b"), "/")))
print(.Internal(file.path(list(c("a", "b"), "c"), "\\")))
print(.Internal(file.path(list(character(), "x"), "/")))
print(.Internal(file.path(list(), "/")))
print(.Internal(file.path(list(1:2, "x"), "/")))
print(.Internal(file.path(list(quote(sym), "x"), "/")))
print(.Internal(file.path(list(factor(c("u", "v")), "w"), "/")))
print(.Internal(file.path(list(NA, "x"), "/")))
print(.Internal(file.path(list("a", NA_character_), "")))
e <- .Internal(file.path(list("é", "x"), "/"))
print(e)
print(Encoding(e))
print(err(.Internal(file.path(1, "/"))))
print(err(.Internal(file.path(list("a"), NA_character_))))
print(err(.Internal(file.path(list("a"), character()))))
print(err(.Internal(file.path(list(sum), "/"))))
b <- "abc\xff"
Encoding(b) <- "bytes"
print(err(.Internal(dir.exists(c("/", b)))))
l <- "a\x81"
Encoding(l) <- "latin1"
print(withCallingHandlers(.Internal(dir.exists(c("/", l))),
  warning = function(w) { cat("Warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }))
