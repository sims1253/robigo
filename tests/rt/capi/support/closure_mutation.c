// In-place writes into a closure's body and formals from package C
// (docs/g3.md §9.3, §15.3 E6): the cells R_ClosureExpr and
// R_ClosureFormals reach, written with SETCAR and SET_TAG; and a callback
// into R from C, for writes made while compiled code is suspended.
// ATTRIB, for the attribute cells shared into a call (non-API in 4.6.2,
// as a package that still uses it would declare it)
#define ENABLE_LEGACY_NONAPI_FUNS
#include <R.h>
#include <Rinternals.h>

static SEXP nth_cell(SEXP x, int i) {
    for (int k = 0; k < i; k++)
        x = CDR(x);
    return x;
}

// element i (0-based) of the body's pairlist (`{`'s cell is 0) set to e
SEXP rt_set_body_elt(SEXP f, SEXP i, SEXP e) {
    SETCAR(nth_cell(R_ClosureExpr(f), asInteger(i)), e);
    return R_NilValue;
}

// formal i's default set to e
SEXP rt_set_formal_default(SEXP f, SEXP i, SEXP e) {
    SETCAR(nth_cell(R_ClosureFormals(f), asInteger(i)), e);
    return R_NilValue;
}

// formal i renamed to the symbol s
SEXP rt_set_formal_tag(SEXP f, SEXP i, SEXP s) {
    SET_TAG(nth_cell(R_ClosureFormals(f), asInteger(i)), s);
    return R_NilValue;
}

// element j of formal i's default (a call) set to e: the call written in
// place
SEXP rt_set_default_elt(SEXP f, SEXP i, SEXP j, SEXP e) {
    SEXP d = CAR(nth_cell(R_ClosureFormals(f), asInteger(i)));
    SETCAR(nth_cell(d, asInteger(j)), e);
    return R_NilValue;
}

// fun() evaluated in the global environment: R code called back from C
SEXP rt_callback(SEXP fun) {
    SEXP call = PROTECT(lang1(fun));
    SEXP ans = eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return ans;
}

// what R_ClosureExpr and R_ClosureFormals return
SEXP rt_body(SEXP f) { return R_ClosureExpr(f); }
SEXP rt_formals(SEXP f) { return R_ClosureFormals(f); }

// element i of the body's pairlist, a call `quote(x)`, made to quote a
// pairlist cell that is its own CAR and CDR (data C made cyclic)
SEXP rt_set_quote_cycle(SEXP f, SEXP i) {
    SEXP q = CAR(nth_cell(R_ClosureExpr(f), asInteger(i)));
    SEXP p = PROTECT(CONS(R_NilValue, R_NilValue));
    SETCAR(p, p);
    SETCDR(p, p);
    SETCAR(CDR(q), p);
    UNPROTECT(1);
    return R_NilValue;
}

// the call at element i of the body: its head set to the symbol s
SEXP rt_set_call_head(SEXP f, SEXP i, SEXP s) {
    SETCAR(CAR(nth_cell(R_ClosureExpr(f), asInteger(i))), s);
    return R_NilValue;
}

// element k of the first argument (a call) of the call at element i of
// the body set to e
SEXP rt_set_arg_elt(SEXP f, SEXP i, SEXP k, SEXP e) {
    SEXP c = CAR(nth_cell(R_ClosureExpr(f), asInteger(i)));
    SETCAR(nth_cell(CADR(c), asInteger(k)), e);
    return R_NilValue;
}

// the call at element i of the body: its names removed (setAttrib's
// removeAttrib of a pairlist's names: its tags)
SEXP rt_unname_call(SEXP f, SEXP i) {
    setAttrib(CAR(nth_cell(R_ClosureExpr(f), asInteger(i))), R_NamesSymbol, R_NilValue);
    return R_NilValue;
}

// the arguments of the call at element i of the body made x's attribute
// cells (SETCDR(call, ATTRIB(x))): the call and x share them
SEXP rt_share_attrib(SEXP f, SEXP i, SEXP x) {
    SETCDR(CAR(nth_cell(R_ClosureExpr(f), asInteger(i))), ATTRIB(x));
    return R_NilValue;
}

// setAttrib(x, name, v) from C
SEXP rt_set_attr(SEXP x, SEXP name, SEXP v) {
    setAttrib(x, install(CHAR(STRING_ELT(name, 0))), v);
    return R_NilValue;
}

// the cell at `path` in the body set to e: each index but the last steps
// into that element of the call (`CAR(nthcdr(x, k))`), the last is the
// element written
SEXP rt_set_path(SEXP f, SEXP path, SEXP e) {
    SEXP x = R_ClosureExpr(f);
    int n = LENGTH(path);
    for (int k = 0; k < n - 1; k++)
        x = CAR(nth_cell(x, INTEGER(path)[k]));
    SETCAR(nth_cell(x, INTEGER(path)[n - 1]), e);
    return R_NilValue;
}

// the body's list cut after element i (SETCDR to R_NilValue)
SEXP rt_cut_after(SEXP f, SEXP i) {
    SETCDR(nth_cell(R_ClosureExpr(f), asInteger(i)), R_NilValue);
    return R_NilValue;
}

// the call at `path` in the body (each index steps into that element) given
// the argument list of the call `donor`: SETCDR(call, CDR(donor))
SEXP rt_set_cdr_from(SEXP f, SEXP path, SEXP donor) {
    SEXP x = R_ClosureExpr(f);
    for (int k = 0; k < LENGTH(path); k++)
        x = CAR(nth_cell(x, INTEGER(path)[k]));
    SETCDR(x, CDR(donor));
    return R_NilValue;
}
