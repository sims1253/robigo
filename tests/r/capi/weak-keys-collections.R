# Weak keys die at the first collection after they become unreachable,
# also one that C code's allocations trigger (no gc() call), whatever
# else has weak references: a loaded namespace keyed for the whole
# session (cli's .onLoad does this), C objects, environments in cycles.
d <- tempfile(); dir.create(d)
cfile <- file.path(d, 'wk.c'); so <- file.path(d, 'wk.so')
writeLines('typedef void *SEXP;
extern SEXP R_NilValue;
extern SEXP R_MakeWeakRef(SEXP, SEXP, SEXP, int);
extern SEXP R_WeakRefKey(SEXP);
extern SEXP R_MakeExternalPtr(void *, SEXP, SEXP);
extern void R_RegisterCFinalizer(SEXP, void (*)(SEXP));
extern void R_PreserveObject(SEXP);
extern void R_ReleaseObject(SEXP);
extern SEXP Rf_allocVector(unsigned int, long);
extern int Rf_asInteger(SEXP);
extern void Rprintf(const char *, ...);
static SEXP held;
SEXP weak(SEXP k, SEXP v, SEXP f) { return R_MakeWeakRef(k, v, f, 0); }
SEXP key(SEXP w) { return R_WeakRefKey(w); }
SEXP touch(SEXP x) { return R_NilValue; }
static void fin(SEXP p) { Rprintf("C pointer finalized\\n"); }
SEXP pointer(void) {
    SEXP p = R_MakeExternalPtr(0, R_NilValue, R_NilValue);
    R_RegisterCFinalizer(p, fin);
    return R_NilValue;
}
SEXP hold(SEXP x) { held = x; R_PreserveObject(x); return R_NilValue; }
SEXP release(void) { R_ReleaseObject(held); held = 0; return R_NilValue; }
/* garbage only C allocates: enough for several collections */
SEXP churn(SEXP n) {
    int k = Rf_asInteger(n);
    for (int i = 0; i < k; i++) Rf_allocVector(14, 100000);
    return R_NilValue;
}
', cfile)
stopifnot(system2('cc', c('-shared', '-fPIC', '-o', shQuote(so), shQuote(cfile))) == 0)
dyn.load(so)
churn <- function(label) {
    invisible(.Call('churn', 2000L))
    for (i in 1:2000) NULL  # where GNU R runs pending finalizers
    cat(label, '\n')
}

# a namespace the registry keeps: never finalized before exit
invisible(reg.finalizer(.BaseNamespaceEnv, function(e) cat('base namespace finalized\n'), onexit = TRUE))

# only C objects otherwise
invisible(.Call('pointer'))
churn('after churn 1')

# an environment and its weak value in a cycle, held by nothing else
cycle_done <- function(k) cat('key-value cycle finalized\n')
e <- new.env()
f <- function() NULL
environment(f) <- e
e$f <- f
invisible(.Call('weak', e, f, cycle_done))
rm(e, f)
invisible(gc())
cat('after gc 2\n')

# a self-cycle reachable from a global: alive until the global goes
keep <- new.env()
keep$self <- keep
w <- .Call('weak', keep, NULL, function(k) cat('self-cycle finalized\n'))
churn('after churn 3')
print(identical(.Call('key', w), keep))
rm(keep)
# (it is old now: GNU R's collections of the young generation that C's
# allocations trigger do not look at it, so gc())
invisible(gc())
cat('after gc 4\n')
print(is.null(.Call('key', w)))

# a key only a closure that C preserves reaches
local({
    k <- new.env()
    invisible(.Call('weak', k, NULL, function(e) cat('C-held key finalized\n')))
    invisible(.Call('hold', function() k))
})
churn('after churn 5')
invisible(gc())
cat('after gc 5\n')
invisible(.Call('release'))
invisible(gc())
cat('after gc 6\n')

# a key that only C held for a while (the bridge stops pinning such a
# proxy once many objects passed through C), back in R, in a cycle: dead
back_done <- function(e) cat('cycle of a key back from C finalized\n')
k <- new.env()
invisible(.Call('hold', k))
w <- .Call('weak', k, NULL, back_done)
rm(k)
many <- lapply(1:5000, function(i) new.env())
invisible(.Call('touch', many))
rm(many)
k <- .Call('key', w)
k$self <- k
rm(k)
invisible(.Call('release'))
churn('after churn 8')

# the environments are gone: only C objects again
invisible(.Call('pointer'))
churn('after churn 9')
cat('done\n')
