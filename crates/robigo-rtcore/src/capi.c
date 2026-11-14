/* The C side of robigo-rtcore's C API state (capi.rs): the one
 * thread-local of the C API (docs/g3.md §11, L6) and the layout of the
 * per-isolate C globals as C sees it (include/robigo_capi.h), for the
 * runtime to check its #[repr(C)] declaration against. */
#include <stddef.h>
#include <stdalign.h>
#include <robigo_capi.h>

/* L6: the isolate thread-local. Exported from the executable (robigo-rt
 * links with -rdynamic), so packages read it through robigo's headers. */
__thread struct robigo_cglobals *robigo_cg __attribute__((tls_model("initial-exec")));

/* Rust reaches the thread-local through these two (Rust has no C
 * thread-locals). */
struct robigo_cglobals *robigo_rtcore_cg(void)
{
    return robigo_cg;
}

void robigo_rtcore_set_cg(struct robigo_cglobals *g)
{
    robigo_cg = g;
}

#define F(f) offsetof(struct robigo_cglobals, f),
#define SYMF(name, str) offsetof(struct robigo_cglobals, name),

/* sizeof, alignof, then the offset of every field in declaration order;
 * capi.rs lists the same fields in the same order. */
const size_t robigo_cglobals_layout[] = {
    sizeof(struct robigo_cglobals),
    alignof(struct robigo_cglobals),
    F(R_NilValue)
    F(R_GlobalEnv)
    F(R_EmptyEnv)
    F(R_BaseEnv)
    F(R_BaseNamespace)
    F(R_UnboundValue)
    F(R_MissingArg)
    F(R_NaString)
    F(R_BlankString)
    F(R_BlankScalarString)
    ROBIGO_CAPI_SYMBOLS(SYMF)
    F(R_PPStack)
    F(R_PPStackTop)
    F(R_PPStackSize)
    F(R_VStack)
    F(gc_move)
    F(isolate)
};

const size_t robigo_cglobals_layout_len = sizeof robigo_cglobals_layout / sizeof robigo_cglobals_layout[0];

/* The symbols' names, in the struct's order, for the runtime to intern. */
#define SYMNAME(name, str) str,
const char *const robigo_cglobals_symbols[] = { ROBIGO_CAPI_SYMBOLS(SYMNAME) };
const size_t robigo_cglobals_nsymbols = sizeof robigo_cglobals_symbols / sizeof robigo_cglobals_symbols[0];
