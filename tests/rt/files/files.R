# platform.c's file.info, file.create, file.append, Sys.chmod and
# path.expand, on files made in a fixture directory and removed after
d <- if (dir.exists("tests/rt/files/fixture")) "tests/rt/files/fixture" else "fixture"
fi <- function(f, x = FALSE) .Internal(file.info(f, x))
info <- fi(file.path(d, c("five.txt", "empty.txt", "sub", "sub/three.txt", "nosuch", NA)))
names(info)
info$size[-3]  # (a directory's size is the file system's)
info$isdir
class(info$mode)
is.na(info$mtime)
is.numeric(info$ctime) && is.numeric(info$atime)
x <- fi(file.path(d, c("five.txt", "nosuch")), TRUE)
names(x)
is.na(x$uid)
is.character(x$uname) && is.na(x$uname[2]) && is.na(x$grname[2])
fi(character(0))
f <- function(e) tryCatch(e, error = function(e) cat("Error:", conditionMessage(e), "\n"))
f(fi(1, FALSE))
f(fi("a", NA))
# file.create, file.append
s <- file.path(d, c("s1.txt", "s2.txt"))
.Internal(file.create(c(s, NA), TRUE))
fi(s)$size
.Internal(file.append(s[1], file.path(d, c("five.txt", "sub/three.txt", "nosuch", "sub", NA))))
fi(s[1])$size
.Internal(file.append(s, file.path(d, c("five.txt", "five.txt", "sub/three.txt", "five.txt"))))
fi(s)$size
.Internal(file.append(s, character(0)))
f(.Internal(file.append(character(0), "a")))
f(.Internal(file.append(1, "a")))
f(.Internal(file.append("a", 1)))
# fwrite into stdio's buffer: a short append to /dev/full succeeds
if (file.exists("/dev/full")) .Internal(file.append("/dev/full", file.path(d, "five.txt"))) else TRUE
if (file.exists("/dev/full")) .Internal(file.append(c("/dev/full", "/dev/full"), file.path(d, c("five.txt", "empty.txt")))) else c(TRUE, TRUE)
# a failed flush drops what was buffered (glibc): one FALSE where the
# 4096-byte buffer overflows, then appends succeed again
if (file.exists("/dev/full")) {
    r <- suppressWarnings(.Internal(file.append("/dev/full", rep(file.path(d, "five.txt"), 1000))))
    which(!r)
} else 683L
.Internal(file.create(s[1], FALSE))
fi(s[1])$size
w <- withCallingHandlers(.Internal(file.create(file.path(d, "nosuchdir/x"), TRUE)),
                         warning = function(w) { cat(sub(".*nosuchdir", "<d>/nosuchdir", conditionMessage(w)), "\n")
                                                 invokeRestart("muffleWarning") })
w
.Internal(file.create(file.path(d, "nosuchdir/x"), FALSE))
f(.Internal(file.create(1, TRUE)))
# Sys.chmod
.Internal(Sys.chmod(c(s, file.path(d, "nosuch"), NA), c(384L, 420L), FALSE))
as.integer(fi(s)$mode)
.Internal(Sys.chmod(s, NA_integer_, FALSE))
as.integer(fi(s)$mode)
.Internal(Sys.chmod(s, as.character(c(256, 448)), FALSE))
as.integer(fi(s)$mode)
.Internal(Sys.chmod(character(0), integer(0), FALSE))
f(.Internal(Sys.chmod(s, integer(0), FALSE)))
f(.Internal(Sys.chmod(s, 420L, NA)))
f(.Internal(Sys.chmod(1, 420L, FALSE)))
.Internal(file.remove(s))
# path.expand
pe <- function(p) .Internal(path.expand(p))
identical(pe(c("~", "~/x")), c(Sys.getenv("HOME"), paste0(Sys.getenv("HOME"), "/x")))
pe(c("a/~b", "", NA, "/x", "~robigo-no-such-user/y"))
identical(pe("~root/z"), paste0(.Internal(path.expand("~root")), "/z"))
pe(character(0))
f(pe(1))
