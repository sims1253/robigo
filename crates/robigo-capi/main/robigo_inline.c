/*
 * Out-of-line copies of the two Rinlinedfuns.h functions our C sources use.
 * Those headers define them gnu_inline, which emits no symbol, and GNU R's
 * inlined.c (which would) defines the whole set, most of it in Rust here.
 * Any unoptimized build calls these.
 */
#include <Rinternals.h>

int *INTEGER0(SEXP x) { return INTEGER(x); }
double *REAL0(SEXP x) { return REAL(x); }
