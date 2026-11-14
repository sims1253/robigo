/* robigo's per-isolate C globals (docs/g3.md §11, L6): what GNU R keeps in
 * process globals that C code reads (R_NilValue, R_GlobalEnv, the symbol
 * shortcuts, R_PPStack and R_PPStackTop, R_VStack), one block per isolate,
 * reached through the one thread-local the C API has, robigo_cg: the block
 * of the isolate whose native code runs on this thread. robigo's
 * Rinternals.h (robigo-rtapi's include/) makes GNU R's names macros over
 * it, after GNU R 4.6.2's own declarations.
 *
 * The runtime (robigo-rtcore, capi.rs) declares the same layout as a
 * #[repr(C)] struct, checked against this one (capi.c's layout table) when
 * an isolate is made and in the crate's tests. */
#ifndef ROBIGO_CAPI_H_
#define ROBIGO_CAPI_H_

/* (Rinternals.h's, repeated: identical typedefs may repeat in C11) */
typedef struct SEXPREC *SEXP;

/* The symbol shortcuts of Rinternals.h, in its order: R_<name>, the
 * symbol's name. */
#define ROBIGO_CAPI_SYMBOLS(X) \
    X(R_AsCharacterSymbol, "as.character") \
    X(R_AtsignSymbol, "@") \
    X(R_baseSymbol, "base") \
    X(R_BaseSymbol, "base") \
    X(R_BraceSymbol, "{") \
    X(R_Bracket2Symbol, "[[") \
    X(R_BracketSymbol, "[") \
    X(R_ClassSymbol, "class") \
    X(R_DeviceSymbol, ".Device") \
    X(R_DimNamesSymbol, "dimnames") \
    X(R_DimSymbol, "dim") \
    X(R_DollarSymbol, "$") \
    X(R_DotsSymbol, "...") \
    X(R_DoubleColonSymbol, "::") \
    X(R_DropSymbol, "drop") \
    X(R_EvalSymbol, "eval") \
    X(R_FunctionSymbol, "function") \
    X(R_LastvalueSymbol, ".Last.value") \
    X(R_LevelsSymbol, "levels") \
    X(R_ModeSymbol, "mode") \
    X(R_NaRmSymbol, "na.rm") \
    X(R_NameSymbol, "name") \
    X(R_NamesSymbol, "names") \
    X(R_NamespaceEnvSymbol, ".__NAMESPACE__.") \
    X(R_PackageSymbol, "package") \
    X(R_PreviousSymbol, "previous") \
    X(R_QuoteSymbol, "quote") \
    X(R_RowNamesSymbol, "row.names") \
    X(R_SeedsSymbol, ".Random.seed") \
    X(R_SortListSymbol, "sort.list") \
    X(R_SourceSymbol, "source") \
    X(R_SpecSymbol, "spec") \
    X(R_TripleColonSymbol, ":::") \
    X(R_TspSymbol, "tsp") \
    X(R_dot_defined, ".defined") \
    X(R_dot_Method, ".Method") \
    X(R_dot_packageName, ".packageName") \
    X(R_dot_target, ".target") \
    X(R_dot_Generic, ".Generic")

#define ROBIGO_CAPI_FIELD(name, str) SEXP name;

struct robigo_cglobals {
    /* the constants and fixed environments (the objects never move: they
     * are marked as seen by C when the isolate is made) */
    SEXP R_NilValue;
    SEXP R_GlobalEnv;
    SEXP R_EmptyEnv;
    SEXP R_BaseEnv;
    SEXP R_BaseNamespace;
    SEXP R_UnboundValue;
    SEXP R_MissingArg;
    SEXP R_NaString;
    SEXP R_BlankString;
    SEXP R_BlankScalarString;
    ROBIGO_CAPI_SYMBOLS(ROBIGO_CAPI_FIELD)
    /* the protect stack: R_PPStack[0 .. R_PPStackTop) are roots */
    SEXP *R_PPStack;
    int R_PPStackTop;
    int R_PPStackSize;
    /* R_alloc's stack (memory.c's R_VStack): the newest block, linked
     * through ATTRIB; vmaxget() is this word */
    SEXP R_VStack;
    /* ROBIGO_GC_MOVE is on: every object C is handed is marked as seen by
     * C, so the debug collector leaves it in place */
    int gc_move;
    /* the isolate (opaque to C) */
    void *isolate;
};

#undef ROBIGO_CAPI_FIELD

/* L6: the isolate thread-local. The block of the isolate whose native code
 * runs on this thread; set by every native entry and restored when it
 * returns or is left. Defined in robigo-rtcore's capi.c, initial-exec: the
 * executable's static TLS block, which a dlopen'ed package reaches with one
 * load. */
extern __thread struct robigo_cglobals *robigo_cg __attribute__((tls_model("initial-exec")));

#endif /* ROBIGO_CAPI_H_ */
