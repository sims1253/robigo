# tracemem after C code saw a value: robigo's C API keeps a weak twin of a
# string vector or list that C looked up or received, but R writes the
# value in place when only one variable holds it (no report). A shared one
# is duplicated (one report).
d <- tempfile(); dir.create(d)
cfile <- file.path(d, "bridge.c")
so <- file.path(d, paste0("bridge", .Platform$dynlib.ext))
writeLines('
typedef void *SEXP;
extern SEXP Rf_findVar(SEXP, SEXP), Rf_install(const char *);
extern SEXP R_NilValue;
SEXP look(SEXP env) { Rf_findVar(Rf_install("x"), env); return R_NilValue; }
SEXP pass(SEXP x) { return R_NilValue; }
', cfile)
stopifnot(system2("cc", c("-shared", "-fPIC", "-o", shQuote(so), shQuote(cfile)), stdout = FALSE, stderr = FALSE) == 0)
dyn.load(so)
t <- function(label, expr) {
  lines <- character(0)
  tc <- textConnection("lines", "wr", local = TRUE)
  sink(tc)
  eval.parent(substitute(expr))
  sink()
  close(tc)
  cat(label, ":", length(lines), "lines\n")
}
t("list looked up by C", { x <- list(1, 2); invisible(.Call("look", environment())); invisible(tracemem(x)); x[[1]] <- 5 })
t("list passed to C", { x <- list(1, 2); invisible(.Call("pass", x)); invisible(tracemem(x)); x[[1]] <- 5 })
t("strings looked up by C", { x <- c("a", "b"); invisible(.Call("look", environment())); invisible(tracemem(x)); x[1] <- "z" })
t("shared list looked up by C", { x <- list(1, 2); invisible(.Call("look", environment())); y <- x; invisible(tracemem(x)); x[[1]] <- 5 })
f <- function() { x <- list(1, 2); invisible(.Call("look", environment())); invisible(tracemem(x)); x[[1]] <- 5; x[2] <- list(3) }
t("in function", f())
dyn.unload(so)
unlink(d, recursive = TRUE)
