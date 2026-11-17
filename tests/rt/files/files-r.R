# the R-level wrappers of file.create, file.append, Sys.chmod, path.expand
# and file.info's columns
d <- if (dir.exists("tests/rt/files/fixture")) "tests/rt/files/fixture" else "fixture"
s <- file.path(d, "r1.txt")
file.create(s)
file.append(s, file.path(d, "five.txt"))
Sys.chmod(s, "640")
as.integer(file.info(s, extra_cols = FALSE)$mode)
file.size(s)
file.info(s, extra_cols = FALSE)$isdir
Sys.chmod(s, "600", use_umask = FALSE)
as.integer(file.mode(s))
file.mtime(s) > 0
identical(path.expand("~/x"), paste0(Sys.getenv("HOME"), "/x"))
path.expand(c("a", NA))
file.remove(s)
