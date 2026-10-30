d <- tempfile(); dir.create(d)
cfile <- file.path(d, 'weak.c'); so <- file.path(d, 'weak.so')
writeLines('typedef void *SEXP;
extern SEXP R_NilValue;
extern SEXP R_MakeWeakRef(SEXP, SEXP, SEXP, int);
extern SEXP R_MakeExternalPtr(void *, SEXP, SEXP);
SEXP weak(SEXP e) { return R_MakeWeakRef(e, R_NilValue, R_NilValue, 0); }
SEXP pointer(void) { return R_MakeExternalPtr(0, R_NilValue, R_NilValue); }
', cfile)
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
p <- .Call('pointer')
reg.finalizer(p, function(e) cat('external done\n'))
rm(p)
invisible(gc())
