/* Every symbol Rinternals.h exports as data (its "Symbol Table
   Shortcuts"), by name: a package that links against one of them must
   load, and each one is the installed symbol. R_AtsignSymbol and
   R_NamespaceEnvSymbol were missing from robigo's libR (issue #340). */
#include <R.h>
#include <Rinternals.h>

SEXP capi_symbol_globals(void)
{
    SEXP syms[] = {
	R_AsCharacterSymbol, R_AtsignSymbol, R_baseSymbol, R_BaseSymbol,
	R_BraceSymbol, R_Bracket2Symbol, R_BracketSymbol, R_ClassSymbol,
	R_DeviceSymbol, R_DimNamesSymbol, R_DimSymbol, R_DollarSymbol,
	R_DotsSymbol, R_DoubleColonSymbol, R_DropSymbol, R_EvalSymbol,
	R_FunctionSymbol, R_LastvalueSymbol, R_LevelsSymbol, R_ModeSymbol,
	R_NaRmSymbol, R_NameSymbol, R_NamesSymbol, R_NamespaceEnvSymbol,
	R_PackageSymbol, R_PreviousSymbol, R_QuoteSymbol, R_RowNamesSymbol,
	R_SeedsSymbol, R_SortListSymbol, R_SourceSymbol, R_SpecSymbol,
	R_TripleColonSymbol, R_TspSymbol, R_dot_defined, R_dot_Method,
	R_dot_packageName, R_dot_target, R_dot_Generic
    };
    int n = (int) (sizeof(syms) / sizeof(syms[0]));
    SEXP ans = PROTECT(allocVector(STRSXP, n));
    for (int i = 0; i < n; i++) {
	if (TYPEOF(syms[i]) != SYMSXP || syms[i] != install(CHAR(PRINTNAME(syms[i]))))
	    error("element %d is not an installed symbol", i + 1);
	SET_STRING_ELT(ans, i, PRINTNAME(syms[i]));
    }
    UNPROTECT(1);
    return ans;
}

/* The issue's repro: the namespace marker binding, found through the
   exported symbol. */
SEXP capi_namespace_name(SEXP ns)
{
    SEXP info = findVarInFrame(ns, R_NamespaceEnvSymbol);
    if (TYPEOF(info) != ENVSXP) error("no namespace information");
    return findVarInFrame(info, install("spec"));
}

/* x@name through a call built with R_AtsignSymbol. */
SEXP capi_atsign(SEXP x, SEXP name, SEXP rho)
{
    SEXP call = PROTECT(lang3(R_AtsignSymbol, x, name));
    SEXP ans = eval(call, rho);
    UNPROTECT(1);
    return ans;
}
