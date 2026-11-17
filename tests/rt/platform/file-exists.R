# platform.c's file.exists (do_fileexists, R_FileExists).
d <- if (dir.exists("tests/rt/platform/listfiles")) "tests/rt/platform/listfiles" else "listfiles"
fe <- function(x) .Internal(file.exists(x))
fe(c(d, file.path(d, "a.txt"), file.path(d, ".hidden"), file.path(d, "sub", "deeper", "e.Rd")))
fe(c(file.path(d, "nosuch"), NA, "", file.path(d, "a.txt", "x")))
fe(character(0))
fe("~")
fe(strrep("a", 5000))
fe(paste0(d, "/", strrep(".", 1)))
tryCatch(fe(1), error = function(e) conditionMessage(e))
tryCatch(fe(NULL), error = function(e) conditionMessage(e))
tryCatch(.Internal(file.exists()), error = function(e) conditionMessage(e))
s <- "é"
Encoding(s) <- "bytes"
tryCatch(fe(s), error = function(e) conditionMessage(e))
l <- "\x81"
Encoding(l) <- "latin1"
withCallingHandlers(fe(l), warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
# ~user is that user's home (the password database)
fe(c("~root", "~root/", "~nosuchuserxyz"))
