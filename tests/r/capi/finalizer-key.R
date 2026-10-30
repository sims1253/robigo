d <- tempfile(); dir.create(d)
cfile <- file.path(d, 'weak.c'); so <- file.path(d, 'weak.so')
writeLines('typedef void *SEXP; extern SEXP R_NilValue; extern SEXP R_MakeWeakRef(SEXP, SEXP, SEXP, int); SEXP weak(SEXP e) { return R_MakeWeakRef(e, R_NilValue, R_NilValue, 0); }', cfile)
stopifnot(system2('cc', c('-shared', '-fPIC', '-o', shQuote(so), shQuote(cfile))) == 0)
dyn.load(so)
w <- .Call('weak', new.env())
probe <- function(fun, onexit=FALSE) {
    tryCatch(reg.finalizer(w, fun, onexit), error=function(e) print(conditionMessage(e)))
}
probe(function(e) {})
probe(NULL, NA)
probe(function(e) {}, NA)
probe(1, TRUE)
