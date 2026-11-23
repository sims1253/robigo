# Text writes to a full device (/dev/full, Linux), GNU R 4.6.2 against robigo-rt:
# whether a write fails depends on stdio's buffer (L5 saveload ledger).
#   docker run --rm -i robigo-oracle:4.6.2 R --vanilla --no-echo -f /dev/stdin < devfull-probe.R
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e), warning = function(w) paste("warning:", conditionMessage(w)))
for (n in c(10, 5000, 100000)) {
  con <- file("/dev/full", "w")
  print(msg(writeLines(strrep("a", n), con)))
  print(msg(close(con)))
  con <- file("/dev/full", "w")
  print(msg(serialize(strrep("a", n), con, ascii = TRUE)))
  print(msg(close(con)))
  con <- file("/dev/full", "w")
  x <- strrep("a", n)
  print(msg(save(x, file = con, ascii = TRUE)))
  print(msg(close(con)))
}
