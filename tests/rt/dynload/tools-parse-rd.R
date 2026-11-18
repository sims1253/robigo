# tools' native code in R_HOME (library/tools/libs/tools.so: GNU R's
# sources, built against robigo's headers by scripts/rt-home.sh; A5): its
# R_init_tools registers the routines and forces native symbols, and
# .External2's parseRd parses an Rd file through a file() connection (the
# connection's struct Rconn, Rconn_fgetc, a C context closing it on error).
d <- dyn.load(file.path(R.home(), "library", "tools", "libs", "tools.so"))
class(d)
d[["name"]]
d[["dynamicLookup"]]
d[["forceSymbols"]]
r <- getDLLRegisteredRoutines(d)
sapply(r, length)
names(r$.External)
names(r$.Call)
s <- getNativeSymbolInfo("parseRd", d)
class(s)
s$numParameters
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-parse-rd-", Sys.getpid(), ".Rd")
writeLines(c("\\name{foo}", "\\alias{foo}", "\\title{A Title}", "\\description{Some \\emph{text}.}"), f)
con <- file(f)
x <- .External2(s, con, new.env(), "UTF-8", FALSE, "foo", FALSE, TRUE, FALSE, FALSE)
summary(con)$opened
close(con)
length(x)
vapply(x, function(e) attr(e, "Rd_tag"), "")
unlist(x[[1]])
vapply(x[[7]], function(e) attr(e, "Rd_tag"), "")
unlist(x[[7]])
unlist(x[[7]][[2]])
tryCatch(.External2("parseRd", PACKAGE = "tools"), error = function(e) conditionMessage(e))
invisible(file.remove(f))
