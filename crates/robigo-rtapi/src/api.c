/* robigo-rtapi's C half (docs/g3.md §3 item 6, §11): the parts of GNU R's
 * C API that are C here, for one of three reasons.
 *
 * - The hot accessors that cannot collect (TYPEOF, LENGTH, INTEGER, CAR,
 *   VECTOR_ELT, SET_VECTOR_ELT, PROTECT, ...): a few loads over robigo's
 *   object layout, as GNU R's own memory.c has them. Their error paths call
 *   an export with a machine entry (lib.rs), from a C frame: no Rust frame
 *   is ever in a C segment. Writes go through the heap's barrier
 *   (robigo_rtapi_store, Rust, which cannot collect).
 * - The variadic functions (Rf_error, Rprintf, ...): the message is
 *   formatted here and handed to a fixed-arity export.
 * - R_UnwindProtect, as GNU R composes it: the protected call and its
 *   context are an export's (robigo_rtapi_unwind_protect), the cleanup is
 *   called from this C frame, so a cleanup that leaves by a longjmp of its
 *   own (cpp11) skips C frames only.
 *
 * GNU R 4.6.2's memory.c, errors.c, printutils.c and context.c are the spec;
 * the declarations here are GNU R's (a test compiles this file with GNU R's
 * Rinternals.h too, which would reject a different one).
 *
 * robigo's object layout (robigo-heap's lib.rs): +0 type, +1 flags, +6 the
 * extra bits, +8 attributes, +16 length or car, +24 cdr, +32 tag or the
 * payload (or EXTDATA's payload pointer). robigo_rtapi_layout lists the
 * offsets and bits for the runtime to check against the heap's. */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <robigo_capi.h>

/* GNU R's types, unless this file is being checked against GNU R's own
 * headers (crates/robigo-rtapi/tests/capi.rs compiles it with robigo's
 * Rinternals.h forced in, so every definition here must match GNU R's
 * declaration). */
#ifndef R_INTERNALS_H_
#define R_NilValue (robigo_cg->R_NilValue)
typedef ptrdiff_t R_xlen_t;
typedef unsigned int SEXPTYPE;
typedef int PROTECT_INDEX;
typedef int Rboolean;
typedef struct { double r, i; } Rcomplex;
typedef unsigned char Rbyte;
typedef void *(*DL_FUNC)(void);
#endif

/* ---- the layout ---------------------------------------------------- */

#define RB_OFF_FLAGS 1
#define RB_OFF_EXTRA 6
#define RB_OFF_ATTRIB 8
#define RB_OFF_LEN 16
#define RB_OFF_DATA 32
#define RB_OFF_CAR 16
#define RB_OFF_CDR 24
#define RB_OFF_TAG 32
#define RB_ALTREP 0x02
#define RB_EXTDATA 0x04
#define RB_SEEN_BY_C (1 << 4)

const size_t robigo_rtapi_layout[] = {
    RB_OFF_FLAGS, RB_OFF_EXTRA, RB_OFF_ATTRIB, RB_OFF_LEN, RB_OFF_DATA, RB_OFF_CAR, RB_OFF_CDR, RB_OFF_TAG,
    RB_ALTREP, RB_EXTDATA, RB_SEEN_BY_C,
};
const size_t robigo_rtapi_layout_len = sizeof robigo_rtapi_layout / sizeof robigo_rtapi_layout[0];

#define RB_TYPE(x) (*(const unsigned char *) (x))
#define RB_FLAGS(x) (((const unsigned char *) (x))[RB_OFF_FLAGS])
#define RB_EXTRA(x) (*(unsigned short *) ((char *) (x) + RB_OFF_EXTRA))
#define RB_WORD(x, off) (*(SEXP *) ((char *) (x) + (off)))
#define RB_LEN(x) (*(R_xlen_t *) ((char *) (x) + RB_OFF_LEN))
#define RB_IS_ALTREP(x) (RB_FLAGS(x) & RB_ALTREP)
#define RB_DATA(x) ((RB_FLAGS(x) & RB_EXTDATA) ? *(void **) ((char *) (x) + RB_OFF_DATA) : (void *) ((char *) (x) + RB_OFF_DATA))

#ifndef R_INTERNALS_H_
enum { NILSXP = 0, SYMSXP = 1, LISTSXP = 2, CLOSXP = 3, ENVSXP = 4, PROMSXP = 5, LANGSXP = 6, CHARSXP = 9,
       LGLSXP = 10, INTSXP = 13, REALSXP = 14, CPLXSXP = 15, STRSXP = 16, DOTSXP = 17, VECSXP = 19,
       EXPRSXP = 20, EXTPTRSXP = 22, WEAKREFSXP = 23, RAWSXP = 24 };
#endif

/* An object C is handed from here on is C's: under ROBIGO_GC_MOVE=1 it is
 * marked so that the debug collector leaves it in place. */
static inline SEXP seen(SEXP x)
{
    if (__builtin_expect(robigo_cg->gc_move, 0) && x)
        RB_EXTRA(x) |= RB_SEEN_BY_C;
    return x;
}

/* ---- the Rust exports this file calls (lib.rs, memory.rs, ...) ------ */

/* the heap's barrier store into a traced field of x (cannot collect) */
void robigo_rtapi_store(SEXP x, SEXP *slot, SEXP v);
/* errors, with machine entries: they never return */
__attribute__((noreturn)) void robigo_rtapi_error(const char *msg);
__attribute__((noreturn)) void robigo_rtapi_errorcall(SEXP call, const char *msg);
/* "%s() can only be applied to a '%s', not a '%s'" with R_typeToChar(x) */
__attribute__((noreturn)) void robigo_rtapi_type_error(const char *fun, const char *what, SEXP x);
/* "attempt access index %lld/%lld in %s" (set: "attempt to set index") */
__attribute__((noreturn)) void robigo_rtapi_index_error(const char *fun, R_xlen_t i, R_xlen_t n);
__attribute__((noreturn)) void robigo_rtapi_set_index_error(const char *fun, R_xlen_t i, R_xlen_t n);
/* "Value of SET_STRING_ELT() must be a 'CHARSXP' not a '%s'" */
__attribute__((noreturn)) void robigo_rtapi_charsxp_error(SEXP v);
/* "%s: argument of type %s is not an external pointer" */
__attribute__((noreturn)) void robigo_rtapi_extptr_error(const char *fun, SEXP x);
/* the protect stack: overflow, unprotect below its bottom, a bad reprotect
 * index */
__attribute__((noreturn)) void robigo_rtapi_protect_error(void);
__attribute__((noreturn)) void robigo_rtapi_unprotect_error(void);
__attribute__((noreturn)) void robigo_rtapi_reprotect_error(PROTECT_INDEX i);
/* a "not implemented yet" error for an accessor's case robigo lacks */
__attribute__((noreturn)) void robigo_rtapi_not_yet(const char *what);
void robigo_rtapi_warning(const char *msg);
void robigo_rtapi_warningcall(SEXP call, const char *msg);
void robigo_rtapi_print(int to_stderr, const char *buf, size_t n);
int robigo_rtapi_warn_length(void);
R_xlen_t robigo_rtapi_length_slow(SEXP x);
int robigo_rtapi_unwind_protect(SEXP (*fun)(void *), void *data, SEXP cont, SEXP *result);
#ifndef R_INTERNALS_H_
void R_ContinueUnwind(SEXP cont);
SEXP R_MakeUnwindCont(void);
SEXP (CDDR)(SEXP e);
#endif

/* ---- types and lengths --------------------------------------------- */

int (TYPEOF)(SEXP x)
{
    return RB_TYPE(x);
}

static inline int is_vector_type(int t)
{
    switch (t) {
    case LGLSXP: case INTSXP: case REALSXP: case CPLXSXP: case STRSXP: case VECSXP: case EXPRSXP:
    case RAWSXP: case CHARSXP: case WEAKREFSXP:
        return 1;
    default:
        return 0;
    }
}

/* LENGTH of a vector (GNU R's CHK2 accepts vectors only) */
R_xlen_t (XLENGTH)(SEXP x)
{
    if (!is_vector_type(RB_TYPE(x)) || RB_IS_ALTREP(x))
        return robigo_rtapi_length_slow(x);
    return RB_LEN(x);
}

int (LENGTH)(SEXP x)
{
    if (x == R_NilValue)
        return 0;
    if (!is_vector_type(RB_TYPE(x)) || RB_IS_ALTREP(x))
        return (int) robigo_rtapi_length_slow(x);
    return (int) RB_LEN(x);
}

R_xlen_t Rf_XLENGTH(SEXP x)
{
    return XLENGTH(x);
}

/* R_len_t length(SEXP) (Rinlinedfuns.h): vectors, pairlists by walking,
 * environments and package ALTREP objects in Rust, 1 for anything else */
int Rf_length(SEXP s)
{
    int t = RB_TYPE(s);
    if (t == NILSXP)
        return 0;
    if (is_vector_type(t) && !RB_IS_ALTREP(s))
        return (int) RB_LEN(s);
    switch (t) {
    case LISTSXP: case LANGSXP: case DOTSXP: {
        int i = 0;
        while (s != NULL && s != R_NilValue) {
            i++;
            s = RB_WORD(s, RB_OFF_CDR);
        }
        return i;
    }
    case ENVSXP:
        return (int) robigo_rtapi_length_slow(s);
    default:
        if (is_vector_type(t))
            return (int) robigo_rtapi_length_slow(s);
        return 1;
    }
}

R_xlen_t Rf_xlength(SEXP s)
{
    int t = RB_TYPE(s);
    if (is_vector_type(t) && !RB_IS_ALTREP(s))
        return RB_LEN(s);
    if (t == LISTSXP || t == LANGSXP || t == DOTSXP || t == NILSXP || t == ENVSXP)
        return Rf_length(s);
    if (is_vector_type(t))
        return robigo_rtapi_length_slow(s);
    return 1;
}

SEXP (ATTRIB)(SEXP x)
{
    if (RB_TYPE(x) == CHARSXP)
        return R_NilValue;
    return seen(RB_WORD(x, RB_OFF_ATTRIB));
}

int (OBJECT)(SEXP x)
{
    return RB_FLAGS(x) & 0x01;
}

/* ---- vectors ------------------------------------------------------- */

/* GNU R builds memory.c with CATCH_ZERO_LENGTH_ACCESS (Rinlinedfuns.h): a
 * zero-length vector's data pointer is (void *) 1, so that reading or
 * writing it faults rather than touching memory (CHKZLN) */
static inline void *dataptr(SEXP x)
{
    if (RB_IS_ALTREP(x))
        robigo_rtapi_not_yet("DATAPTR of an ALTREP object");
    if (RB_LEN(x) == 0)
        return (void *) 1;
    return RB_DATA(x);
}

int *(LOGICAL)(SEXP x)
{
    if (RB_TYPE(x) != LGLSXP)
        robigo_rtapi_type_error("LOGICAL", "logical", x);
    return dataptr(x);
}

const int *(LOGICAL_RO)(SEXP x)
{
    if (RB_TYPE(x) != LGLSXP)
        robigo_rtapi_type_error("LOGICAL", "logical", x);
    return dataptr(x);
}

int *(INTEGER)(SEXP x)
{
    if (RB_TYPE(x) != INTSXP && RB_TYPE(x) != LGLSXP)
        robigo_rtapi_type_error("INTEGER", "integer", x);
    return dataptr(x);
}

const int *(INTEGER_RO)(SEXP x)
{
    if (RB_TYPE(x) != INTSXP && RB_TYPE(x) != LGLSXP)
        robigo_rtapi_type_error("INTEGER", "integer", x);
    return dataptr(x);
}

double *(REAL)(SEXP x)
{
    if (RB_TYPE(x) != REALSXP)
        robigo_rtapi_type_error("REAL", "numeric", x);
    return dataptr(x);
}

const double *(REAL_RO)(SEXP x)
{
    if (RB_TYPE(x) != REALSXP)
        robigo_rtapi_type_error("REAL", "numeric", x);
    return dataptr(x);
}

Rcomplex *(COMPLEX)(SEXP x)
{
    if (RB_TYPE(x) != CPLXSXP)
        robigo_rtapi_type_error("COMPLEX", "complex", x);
    return dataptr(x);
}

const Rcomplex *(COMPLEX_RO)(SEXP x)
{
    if (RB_TYPE(x) != CPLXSXP)
        robigo_rtapi_type_error("COMPLEX", "complex", x);
    return dataptr(x);
}

Rbyte *(RAW)(SEXP x)
{
    if (RB_TYPE(x) != RAWSXP)
        robigo_rtapi_type_error("RAW", "raw", x);
    return dataptr(x);
}

const Rbyte *(RAW_RO)(SEXP x)
{
    if (RB_TYPE(x) != RAWSXP)
        robigo_rtapi_type_error("RAW", "raw", x);
    return dataptr(x);
}

const char *(R_CHAR)(SEXP x)
{
    if (RB_TYPE(x) != CHARSXP)
        robigo_rtapi_type_error("CHAR", "CHARSXP", x);
    return (const char *) ((char *) x + RB_OFF_DATA);
}

SEXP (STRING_ELT)(SEXP x, R_xlen_t i)
{
    if (RB_TYPE(x) != STRSXP)
        robigo_rtapi_type_error("STRING_ELT", "character vector", x);
    if (i < 0 || i >= RB_LEN(x))
        robigo_rtapi_index_error("STRING_ELT", i, RB_LEN(x));
    if (RB_IS_ALTREP(x))
        robigo_rtapi_not_yet("STRING_ELT of an ALTREP object");
    return seen(((SEXP *) RB_DATA(x))[i]);
}

SEXP (VECTOR_ELT)(SEXP x, R_xlen_t i)
{
    int t = RB_TYPE(x);
    if (t != VECSXP && t != EXPRSXP && t != WEAKREFSXP)
        robigo_rtapi_type_error("VECTOR_ELT", "list", x);
    if (i < 0 || i >= RB_LEN(x))
        robigo_rtapi_index_error("VECTOR_ELT", i, RB_LEN(x));
    if (RB_IS_ALTREP(x))
        robigo_rtapi_not_yet("VECTOR_ELT of an ALTREP object");
    return seen(((SEXP *) RB_DATA(x))[i]);
}

void (SET_STRING_ELT)(SEXP x, R_xlen_t i, SEXP v)
{
    if (RB_TYPE(x) != STRSXP)
        robigo_rtapi_type_error("SET_STRING_ELT", "character vector", x);
    if (RB_TYPE(v) != CHARSXP)
        robigo_rtapi_charsxp_error(v);
    if (i < 0 || i >= RB_LEN(x))
        robigo_rtapi_set_index_error("SET_STRING_ELT", i, RB_LEN(x));
    if (RB_IS_ALTREP(x))
        robigo_rtapi_not_yet("SET_STRING_ELT of an ALTREP object");
    robigo_rtapi_store(x, (SEXP *) RB_DATA(x) + i, v);
}

SEXP (SET_VECTOR_ELT)(SEXP x, R_xlen_t i, SEXP v)
{
    int t = RB_TYPE(x);
    if (t != VECSXP && t != EXPRSXP && t != WEAKREFSXP)
        robigo_rtapi_type_error("SET_VECTOR_ELT", "list", x);
    if (i < 0 || i >= RB_LEN(x))
        robigo_rtapi_set_index_error("SET_VECTOR_ELT", i, RB_LEN(x));
    if (RB_IS_ALTREP(x))
        robigo_rtapi_not_yet("SET_VECTOR_ELT of an ALTREP object");
    robigo_rtapi_store(x, (SEXP *) RB_DATA(x) + i, v);
    return v;
}

/* ---- pairlists ------------------------------------------------------ */

/* GNU R's accessors check nothing (CHKCONS is TESTING_WRITE_BARRIER's) */
#define CAR_(e) RB_WORD(e, RB_OFF_CAR)
#define CDR_(e) RB_WORD(e, RB_OFF_CDR)
#define TAG_(e) RB_WORD(e, RB_OFF_TAG)

SEXP (CAR)(SEXP e) { return seen(CAR_(e)); }
SEXP (CDR)(SEXP e) { return seen(CDR_(e)); }
SEXP (TAG)(SEXP e) { return seen(TAG_(e)); }
SEXP (CAAR)(SEXP e) { return seen(CAR_(CAR_(e))); }
SEXP (CDAR)(SEXP e) { return seen(CDR_(CAR_(e))); }
SEXP (CADR)(SEXP e) { return seen(CAR_(CDR_(e))); }
SEXP (CDDR)(SEXP e) { return seen(CDR_(CDR_(e))); }
SEXP (CDDDR)(SEXP e) { return seen(CDR_(CDR_(CDR_(e)))); }
SEXP (CADDR)(SEXP e) { return seen(CAR_(CDR_(CDR_(e)))); }
SEXP (CADDDR)(SEXP e) { return seen(CAR_(CDR_(CDR_(CDR_(e))))); }
SEXP (CAD4R)(SEXP e) { return seen(CAR_(CDR_(CDR_(CDR_(CDR_(e)))))); }
SEXP (CAD5R)(SEXP e) { return seen(CAR_(CDR_(CDR_(CDR_(CDR_(CDR_(e))))))); }

/* GNU R's CHKCONS(x) == NULL || x == R_NilValue: "bad value" */
static inline void check_cons(SEXP x)
{
    if (x == NULL || x == R_NilValue)
        robigo_rtapi_error("bad value");
}

/* robigo's nodes whose car is not an object: an external pointer's
 * address, a primitive's offset (the barrier would read them as one) */
static inline void check_car_object(SEXP x)
{
    int t = RB_TYPE(x);
    if (t == EXTPTRSXP || t == 7 /* SPECIALSXP */ || t == 8 /* BUILTINSXP */ || t == ENVSXP)
        robigo_rtapi_not_yet("SETCAR of this type");
}

SEXP (SETCAR)(SEXP x, SEXP y)
{
    check_cons(x);
    check_car_object(x);
    if (y == CAR_(x))
        return y;
    robigo_rtapi_store(x, &CAR_(x), y);
    return y;
}

SEXP (SETCDR)(SEXP x, SEXP y)
{
    check_cons(x);
    if (RB_TYPE(x) == ENVSXP)
        robigo_rtapi_not_yet("SETCDR of an environment");
    robigo_rtapi_store(x, &CDR_(x), y);
    return y;
}

void (SET_TAG)(SEXP x, SEXP v)
{
    check_cons(x);
    if (RB_TYPE(x) == ENVSXP)
        robigo_rtapi_not_yet("SET_TAG of an environment");
    robigo_rtapi_store(x, &TAG_(x), v);
}

SEXP (SETCADR)(SEXP x, SEXP y)
{
    if (x == NULL || x == R_NilValue || CDR_(x) == NULL || CDR_(x) == R_NilValue)
        robigo_rtapi_error("bad value");
    SEXP cell = CDR_(x);
    check_car_object(cell);
    robigo_rtapi_store(cell, &CAR_(cell), y);
    return y;
}

SEXP (SETCADDR)(SEXP x, SEXP y)
{
    if (x == NULL || x == R_NilValue || CDR_(x) == NULL || CDR_(x) == R_NilValue ||
        CDDR(x) == NULL || CDDR(x) == R_NilValue)
        robigo_rtapi_error("bad value");
    SEXP cell = CDR_(CDR_(x));
    check_car_object(cell);
    robigo_rtapi_store(cell, &CAR_(cell), y);
    return y;
}

/* ---- symbols -------------------------------------------------------- */

SEXP (PRINTNAME)(SEXP x)
{
    return seen(CAR_(x));
}

/* ---- external pointers and weak references ---------------------------- */

void *R_ExternalPtrAddr(SEXP s)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_ExternalPtrAddr", s);
    return (void *) CAR_(s);
}

SEXP R_ExternalPtrTag(SEXP s)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_ExternalPtrTag", s);
    return seen(TAG_(s));
}

SEXP R_ExternalPtrProtected(SEXP s)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_ExternalPtrProtected", s);
    return seen(CDR_(s));
}

void R_ClearExternalPtr(SEXP s)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_ClearExternalPtr", s);
    CAR_(s) = NULL;
}

void R_SetExternalPtrAddr(SEXP s, void *p)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_SetExternalPtrAddr", s);
    CAR_(s) = (SEXP) p;
}

void R_SetExternalPtrTag(SEXP s, SEXP tag)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_SetExternalPtrTag", s);
    robigo_rtapi_store(s, &TAG_(s), tag);
}

void R_SetExternalPtrProtected(SEXP s, SEXP p)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_SetExternalPtrProtected", s);
    robigo_rtapi_store(s, &CDR_(s), p);
}

DL_FUNC R_ExternalPtrAddrFn(SEXP s)
{
    if (RB_TYPE(s) != EXTPTRSXP)
        robigo_rtapi_extptr_error("R_ExternalPtrAddrFn", s);
    union { void *p; DL_FUNC fn; } tmp;
    tmp.p = (void *) CAR_(s);
    return tmp.fn;
}

/* (WEAKREF_KEY, WEAKREF_VALUE: elements 0 and 1) */
SEXP R_WeakRefKey(SEXP w)
{
    if (RB_TYPE(w) != WEAKREFSXP)
        robigo_rtapi_error("not a weak reference");
    return seen(((SEXP *) RB_DATA(w))[0]);
}

SEXP R_WeakRefValue(SEXP w)
{
    if (RB_TYPE(w) != WEAKREFSXP)
        robigo_rtapi_error("not a weak reference");
    SEXP v = ((SEXP *) RB_DATA(w))[1];
    /* ENSURE_NAMEDMAX(v): GNU R's REFCNT protocol has no NAMED to set */
    return seen(v);
}

/* ---- the protect stack ---------------------------------------------- */

SEXP Rf_protect(SEXP s)
{
    struct robigo_cglobals *g = robigo_cg;
    if (g->R_PPStackTop >= g->R_PPStackSize)
        robigo_rtapi_protect_error();
    g->R_PPStack[g->R_PPStackTop++] = s;
    return s;
}

void Rf_unprotect(int l)
{
    struct robigo_cglobals *g = robigo_cg;
    if (g->R_PPStackTop >= l)
        g->R_PPStackTop -= l;
    else
        robigo_rtapi_unprotect_error();
}

void R_ProtectWithIndex(SEXP s, PROTECT_INDEX *pi)
{
    Rf_protect(s);
    *pi = robigo_cg->R_PPStackTop - 1;
}

void R_Reprotect(SEXP s, PROTECT_INDEX i)
{
    struct robigo_cglobals *g = robigo_cg;
    if (i >= g->R_PPStackTop || i < 0)
        robigo_rtapi_reprotect_error(i);
    g->R_PPStack[i] = s;
}

void Rf_unprotect_ptr(SEXP s)
{
    struct robigo_cglobals *g = robigo_cg;
    int i = g->R_PPStackTop;
    do {
        if (i == 0)
            robigo_rtapi_error("unprotect_ptr: pointer not found");
    } while (g->R_PPStack[--i] != s);
    while (++i < g->R_PPStackTop)
        g->R_PPStack[i - 1] = g->R_PPStack[i];
    g->R_PPStackTop--;
}

/* ---- messages --------------------------------------------------------- */

/* GNU R's BUFSIZE (errors.c) */
#define BUFSIZE 8192

/* Rvsnprintf_mbcs (errors.c): vsnprintf into a buffer of size, which, if
 * the text did not fit, ends at the last complete character (a UTF-8
 * locale: robigo's). */
static void vsnprintf_mbcs(char *buf, size_t size, const char *format, va_list ap)
{
    int val = vsnprintf(buf, size, format, ap);
    if (size) {
        if (val < 0 || (size_t) val >= size) {
            /* mbcsTruncateToValid: drop a trailing incomplete UTF-8 sequence */
            size_t n = strlen(buf);
            size_t i = n;
            while (i > 0 && (((unsigned char) buf[i - 1]) & 0xC0) == 0x80)
                i--;
            if (i > 0 && ((unsigned char) buf[i - 1]) >= 0xC0) {
                unsigned char c = (unsigned char) buf[i - 1];
                size_t need = c >= 0xF0 ? 4 : c >= 0xE0 ? 3 : 2;
                if (n - (i - 1) < need)
                    buf[i - 1] = '\0';
            }
        }
    }
}

static size_t min_size(size_t a, size_t b)
{
    return a < b ? a : b;
}

void Rf_error(const char *format, ...)
{
    char buf[BUFSIZE];
    va_list ap;
    va_start(ap, format);
    /* R: errors.c:error (min(BUFSIZE, R_WarnLength)) */
    vsnprintf_mbcs(buf, min_size(BUFSIZE, (size_t) robigo_rtapi_warn_length()), format, ap);
    va_end(ap);
    robigo_rtapi_error(buf);
}

void Rf_errorcall(SEXP call, const char *format, ...)
{
    char buf[BUFSIZE];
    va_list ap;
    va_start(ap, format);
    /* R: errors.c:errorcall -> verrorcall_dflt formats into errbuf */
    vsnprintf_mbcs(buf, BUFSIZE, format, ap);
    va_end(ap);
    robigo_rtapi_errorcall(call, buf);
}

void Rf_warning(const char *format, ...)
{
    char buf[BUFSIZE];
    va_list ap;
    va_start(ap, format);
    /* R: errors.c:warning (psize = min(BUFSIZE, R_WarnLength + 1)) */
    vsnprintf_mbcs(buf, min_size(BUFSIZE, (size_t) robigo_rtapi_warn_length() + 1), format, ap);
    va_end(ap);
    size_t n = strlen(buf);
    if (n > 0 && buf[n - 1] == '\n')
        buf[n - 1] = '\0';
    robigo_rtapi_warning(buf);
}

void Rf_warningcall(SEXP call, const char *format, ...)
{
    char buf[BUFSIZE];
    va_list ap;
    va_start(ap, format);
    vsnprintf_mbcs(buf, BUFSIZE, format, ap);
    va_end(ap);
    robigo_rtapi_warningcall(call, buf);
}

/* R: printutils.c:Rvprintf (R_WriteConsole of what vsnprintf made, a
 * larger buffer when the text does not fit) */
static void vprint(int to_stderr, const char *format, va_list ap)
{
    char buf[BUFSIZE];
    va_list aq;
    va_copy(aq, ap);
    int n = vsnprintf(buf, sizeof buf, format, aq);
    va_end(aq);
    if (n < 0)
        return;
    if ((size_t) n < sizeof buf) {
        robigo_rtapi_print(to_stderr, buf, (size_t) n);
        return;
    }
    char *big = __builtin_alloca((size_t) n + 1);
    vsnprintf(big, (size_t) n + 1, format, ap);
    robigo_rtapi_print(to_stderr, big, (size_t) n);
}

void Rprintf(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    vprint(0, format, ap);
    va_end(ap);
}

void REprintf(const char *format, ...)
{
    va_list ap;
    va_start(ap, format);
    vprint(1, format, ap);
    va_end(ap);
}

void Rvprintf(const char *format, va_list arg)
{
    vprint(0, format, arg);
}

void REvprintf(const char *format, va_list arg)
{
    vprint(1, format, arg);
}

/* ---- R_UnwindProtect -------------------------------------------------- */

/* R: context.c:R_UnwindProtect. The context and fun's call are the
 * export's (robigo_rtapi_unwind_protect, which leaves the continuation
 * owning the exit's record if one stopped there); the cleanup runs here. */
SEXP R_UnwindProtect(SEXP (*fun)(void *data), void *data, void (*cleanfun)(void *data, Rboolean jump),
                     void *cleandata, SEXP cont)
{
    SEXP result;
    if (cont == NULL) {
        Rf_protect(cont = R_MakeUnwindCont());
        result = R_UnwindProtect(fun, data, cleanfun, cleandata, cont);
        Rf_unprotect(1);
        return result;
    }
    Rboolean jump = robigo_rtapi_unwind_protect(fun, data, cont, &result);
    cleanfun(cleandata, jump);
    if (jump)
        R_ContinueUnwind(cont);
    return seen(result);
}
