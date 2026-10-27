library(tools)
### Name: package_native_routine_registration_skeleton
### Title: Write Skeleton for Adding Native Routine Registration to a
###   Package
### Aliases: package_native_routine_registration_skeleton

### ** Examples
## Not run: 
##D ## with a completed splines/DESCRIPTION file,
##D tools::package_native_routine_registration_skeleton('splines',,,FALSE)
##D ## produces
##D #include <R.h>
##D #include <Rinternals.h>
##D #include <stdlib.h> // for NULL
##D #include <R_ext/Rdynload.h>
##D 
##D /* FIXME: 
##D    Check these declarations against the C/Fortran source code.
##D */
##D 
##D /* .Call calls */
##D extern SEXP spline_basis(SEXP, SEXP, SEXP, SEXP);
##D extern SEXP spline_value(SEXP, SEXP, SEXP, SEXP, SEXP);
##D 
##D static const R_CallMethodDef CallEntries[] = {
##D     {"spline_basis", (DL_FUNC) &spline_basis, 4},
##D     {"spline_value", (DL_FUNC) &spline_value, 5},
##D     {NULL, NULL, 0}
##D };
##D 
##D void R_init_splines(DllInfo *dll)
##D {
##D     R_registerRoutines(dll, NULL, CallEntries, NULL, NULL);
##D     R_useDynamicSymbols(dll, FALSE);
##D }
## End(Not run)


