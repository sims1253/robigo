# error() and warning() format into min(BUFSIZE, warning.length (+ 1 for
# warning())) bytes; "object '%s' not found" is the name plus 19 bytes
e <- new.env()
op <- options(warning.length = 100)
err <- function(nm) tryCatch(get(nm, e), error = function(c) conditionMessage(c))
wrn <- function(nm) tryCatch(.Internal(remove(nm, e, FALSE)), warning = function(c) conditionMessage(c))
for (k in 79:83) {
  nm <- strrep("a", k)
  m <- err(nm); w <- wrn(nm)
  cat(k + 19, "bytes: error", nchar(m, "bytes"), "warning", nchar(w, "bytes"), substring(w, nchar(w) - 20), "\n")
}
# a 2-byte character across the cut: its first byte at message byte 97, 98, 99, 100
for (k in 89:92) {
  nm <- paste0(strrep("a", k), "é", "bbbb")
  m <- err(nm); w <- wrn(nm)
  cat(k, ": error", nchar(m, "bytes"), nchar(m), "warning", nchar(w, "bytes"), nchar(w), "\n")
}
options(op)
# the default, 1000
m <- err(strrep("z", 2000)); nchar(m, "bytes")
w <- wrn(strrep("z", 2000)); nchar(w, "bytes"); substring(w, 990)
