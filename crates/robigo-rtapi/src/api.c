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
 * robigo's object layout is robigo-heap's (its layout module), read from
 * the robigo_layout.h its build writes: the offsets of the header, vector
 * and node words and the header bits. robigo_rtapi_layout is the header's
 * table as this file was compiled with it, which the runtime checks
 * against the heap's (lib.rs, check_layout). */
#include <stdarg.h>
#include <stddef.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <robigo_capi.h>
#include <robigo_layout.h>

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

#define RB_OFF_FLAGS ROBIGO_HDR_OFF_FLAGS
#define RB_OFF_EXTRA ROBIGO_HDR_OFF_EXTRA
#define RB_OFF_ATTRIB ROBIGO_HDR_OFF_ATTRIB
#define RB_OFF_LEN ROBIGO_VEC_OFF_LEN
#define RB_OFF_CAR ROBIGO_NODE_OFF_CAR
#define RB_OFF_CDR ROBIGO_NODE_OFF_CDR
#define RB_OFF_TAG ROBIGO_NODE_OFF_TAG
#define RB_OBJECT ROBIGO_FLAG_OBJECT
#define RB_ALTREP ROBIGO_FLAG_ALTREP
#define RB_EXTDATA ROBIGO_FLAG_EXTDATA
#define RB_SEEN_BY_C ROBIGO_EXTRA_SEEN_BY_C

#define RB_LAYOUT_ENTRY(name) (unsigned long long) (name),
const unsigned long long robigo_rtapi_layout[] = { ROBIGO_LAYOUT_TABLE(RB_LAYOUT_ENTRY) };
const size_t robigo_rtapi_layout_len = sizeof robigo_rtapi_layout / sizeof robigo_rtapi_layout[0];
_Static_assert(sizeof(unsigned long long) == sizeof(uint64_t), "Rust reads the table as u64");
_Static_assert(sizeof robigo_rtapi_layout / sizeof robigo_rtapi_layout[0] == ROBIGO_LAYOUT_COUNT,
               "robigo_layout.h's table and its count differ");

/* The words this file reads, as C sees them: the layout's offsets must be
 * where C's own types put the fields (robigo-heap asserts the same of its
 * #[repr(C)] views). */
struct rb_header { unsigned char type, flags; unsigned short gp, refcnt, extra; SEXP attrib; };
struct rb_vector { struct rb_header h; R_xlen_t length, truelength; };
struct rb_node { struct rb_header h; SEXP car, cdr, tag; };
_Static_assert(offsetof(struct rb_header, flags) == ROBIGO_HDR_OFF_FLAGS, "flags");
_Static_assert(offsetof(struct rb_header, gp) == ROBIGO_HDR_OFF_GP, "gp");
_Static_assert(offsetof(struct rb_header, refcnt) == ROBIGO_HDR_OFF_REFCNT, "refcnt");
_Static_assert(offsetof(struct rb_header, extra) == ROBIGO_HDR_OFF_EXTRA, "extra");
_Static_assert(offsetof(struct rb_header, attrib) == ROBIGO_HDR_OFF_ATTRIB, "ATTRIB");
_Static_assert(sizeof(struct rb_header) == ROBIGO_HDR_SIZE, "the header");
_Static_assert(offsetof(struct rb_vector, length) == ROBIGO_VEC_OFF_LEN, "LENGTH");
_Static_assert(offsetof(struct rb_vector, truelength) == ROBIGO_VEC_OFF_TRUELEN, "TRUELENGTH");
_Static_assert(sizeof(struct rb_vector) <= ROBIGO_VEC_OFF_DATA, "the payload follows the lengths");
_Static_assert(offsetof(struct rb_node, car) == ROBIGO_NODE_OFF_CAR, "CAR");
_Static_assert(offsetof(struct rb_node, cdr) == ROBIGO_NODE_OFF_CDR, "CDR");
_Static_assert(offsetof(struct rb_node, tag) == ROBIGO_NODE_OFF_TAG, "TAG");
_Static_assert(ROBIGO_EXTDATA_OFF_PTR == ROBIGO_VEC_OFF_DATA, "EXTDATA's pointer is where the payload goes");
_Static_assert(ROBIGO_CHAR_OFF_LEN == ROBIGO_VEC_OFF_LEN, "a CHARSXP's LENGTH is a vector's");

#define RB_TYPE(x) (((const unsigned char *) (x))[ROBIGO_HDR_OFF_TYPE])
#define RB_FLAGS(x) (((const unsigned char *) (x))[RB_OFF_FLAGS])
#define RB_EXTRA(x) (*(unsigned short *) ((char *) (x) + RB_OFF_EXTRA))
#define RB_WORD(x, off) (*(SEXP *) ((char *) (x) + (off)))
#define RB_LEN(x) (*(R_xlen_t *) ((char *) (x) + RB_OFF_LEN))
#define RB_IS_ALTREP(x) (RB_FLAGS(x) & RB_ALTREP)
/* where a standard vector's payload is: a CHARSXP's bytes have a layout of
 * their own (robigo-heap's layout::data_off) */
#define RB_DATA_OFF(x) (RB_TYPE(x) == CHARSXP ? ROBIGO_CHAR_OFF_DATA : ROBIGO_VEC_OFF_DATA)
#define RB_DATA(x) ((RB_FLAGS(x) & RB_EXTDATA) ? *(void **) ((char *) (x) + ROBIGO_EXTDATA_OFF_PTR) : (void *) ((char *) (x) + RB_DATA_OFF(x)))

#ifndef R_INTERNALS_H_
enum { NILSXP = 0, SYMSXP = 1, LISTSXP = 2, CLOSXP = 3, ENVSXP = 4, PROMSXP = 5, LANGSXP = 6, SPECIALSXP = 7,
       BUILTINSXP = 8, CHARSXP = 9,
       LGLSXP = 10, INTSXP = 13, REALSXP = 14, CPLXSXP = 15, STRSXP = 16, DOTSXP = 17, VECSXP = 19,
       EXPRSXP = 20, EXTPTRSXP = 22, WEAKREFSXP = 23, RAWSXP = 24 };
#endif

/* An object C is handed from here on is C's: under ROBIGO_GC_MOVE=1 it is
 * marked so that the debug collector leaves it in place. An immortal object
 * never moves and is not written (H5: symbols are immutable). */
static inline SEXP seen(SEXP x)
{
    if (__builtin_expect(robigo_cg->gc_move, 0) && x && !(RB_FLAGS(x) & ROBIGO_FLAG_IMMORTAL))
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
/* CHKVEC's "cannot get data pointer of '%s' objects" */
__attribute__((noreturn)) void robigo_rtapi_dataptr_type_error(SEXP x);
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
/* an ALTREP vector's Dataptr, Dataptr_or_null and Elt methods (altrep.rs):
 * robigo's own classes; a package class's are not yet */
void *robigo_rtapi_altvec_dataptr(SEXP x);
void *robigo_rtapi_altvec_dataptr_or_null(SEXP x);
int robigo_rtapi_altinteger_elt(SEXP x, R_xlen_t i);
double robigo_rtapi_altreal_elt(SEXP x, R_xlen_t i);
int robigo_rtapi_unwind_protect(SEXP (*fun)(void *), void *data, SEXP cont, SEXP *result);
#ifndef R_INTERNALS_H_
/* never returns: the exit `cont` owns continues (unwind.rs) */
__attribute__((noreturn)) void R_ContinueUnwind(SEXP cont);
SEXP R_MakeUnwindCont(void);
SEXP Rf_mkString(const char *s);
SEXP Rf_allocVector(SEXPTYPE type, R_xlen_t n);
SEXP R_tryCatch(SEXP (*body)(void *), void *bdata, SEXP conds, SEXP (*handler)(SEXP, void *), void *hdata,
                void (*finally)(void *), void *fdata);
SEXP (CDDR)(SEXP e);
SEXP Rf_cons(SEXP car, SEXP cdr);
SEXP Rf_ScalarInteger(int x);
int *(INTEGER)(SEXP x);
SEXP (CAR)(SEXP e);
SEXP (CDR)(SEXP e);
SEXP (TAG)(SEXP e);
SEXP (SETCAR)(SEXP x, SEXP y);
void (SET_TAG)(SEXP x, SEXP v);
SEXP (VECTOR_ELT)(SEXP x, R_xlen_t i);
SEXP (SET_VECTOR_ELT)(SEXP x, R_xlen_t i, SEXP v);
R_xlen_t (XLENGTH)(SEXP x);
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
    return RB_FLAGS(x) & RB_OBJECT;
}

/* ---- vectors ------------------------------------------------------- */

/* GNU R builds memory.c with CATCH_ZERO_LENGTH_ACCESS (Rinlinedfuns.h): a
 * zero-length vector's data pointer is (void *) 1, so that reading or
 * writing it faults rather than touching memory (CHKZLN) */
static inline void *dataptr(SEXP x)
{
    if (RB_IS_ALTREP(x))
        return robigo_rtapi_altvec_dataptr(x); /* ALTVEC_DATAPTR */
    if (RB_LEN(x) == 0)
        return (void *) 1;
    return RB_DATA(x);
}

/* CHKVEC (Rinlinedfuns.h, STRICT_TYPECHECK) */
static inline void chkvec(SEXP x)
{
    switch (RB_TYPE(x)) {
    case CHARSXP: case LGLSXP: case INTSXP: case REALSXP: case CPLXSXP: case STRSXP: case VECSXP:
    case EXPRSXP: case RAWSXP: case WEAKREFSXP:
        break;
    default:
        robigo_rtapi_dataptr_type_error(x);
    }
}

void *(DATAPTR)(SEXP x)
{
    chkvec(x);
    if (!RB_IS_ALTREP(x) && RB_TYPE(x) == CHARSXP)
        return RB_DATA(x); /* (a CHARSXP's trailing zero byte) */
    return dataptr(x);
}

const void *(DATAPTR_RO)(SEXP x)
{
    chkvec(x);
    if (RB_IS_ALTREP(x))
        return robigo_rtapi_altvec_dataptr(x); /* ALTVEC_DATAPTR_RO */
    return RB_DATA(x);
}

const void *(DATAPTR_OR_NULL)(SEXP x)
{
    chkvec(x);
    if (RB_IS_ALTREP(x))
        return robigo_rtapi_altvec_dataptr_or_null(x);
    return RB_DATA(x);
}

/* CHECK_BOUNDS_ELT: GNU R's test is i > XLENGTH(x) */
static inline void check_bounds_elt(SEXP x, R_xlen_t i)
{
    if (i < 0 || i > XLENGTH(x))
        robigo_rtapi_error("subscript out of bounds");
}

int (INTEGER_ELT)(SEXP x, R_xlen_t i)
{
    if (RB_TYPE(x) != INTSXP && RB_TYPE(x) != LGLSXP)
        robigo_rtapi_error("bad INTSXP vector");
    check_bounds_elt(x, i);
    return RB_IS_ALTREP(x) ? robigo_rtapi_altinteger_elt(x, i) : ((int *) RB_DATA(x))[i];
}

double (REAL_ELT)(SEXP x, R_xlen_t i)
{
    if (RB_TYPE(x) != REALSXP)
        robigo_rtapi_error("bad REALSXP vector");
    check_bounds_elt(x, i);
    return RB_IS_ALTREP(x) ? robigo_rtapi_altreal_elt(x, i) : ((double *) RB_DATA(x))[i];
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
    return (const char *) ((char *) x + ROBIGO_CHAR_OFF_DATA);
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

/* memory.c's R_ClosureFormals, R_ClosureBody, R_ClosureEnv and eval.c's
 * R_ClosureExpr: a closure's FORMALS (car), BODY (cdr) and CLOENV (tag).
 * robigo never stores byte code in BODY (its code hangs off the closure's
 * 4th word), so BODY is always the expression bytecodeExpr would return.
 * Writing into the cells they reach is tracked (robigo_rtapi_store). */
SEXP (R_ClosureFormals)(SEXP x) { return seen(CAR_(x)); }
SEXP (R_ClosureBody)(SEXP x) { return seen(CDR_(x)); }
SEXP (R_ClosureEnv)(SEXP x) { return seen(TAG_(x)); }
SEXP (R_ClosureExpr)(SEXP x) { return seen(CDR_(x)); }

/* Rinlinedfuns.h's ALTREP accessors: an ALTREP object's data1 is its car,
 * data2 its cdr, its class its tag (robigo-heap's layout too) */
int (ALTREP)(SEXP x) { return RB_IS_ALTREP(x) != 0; }
SEXP (ALTREP_CLASS)(SEXP x) { return seen(TAG_(x)); }
SEXP (R_altrep_data1)(SEXP x) { return seen(CAR_(x)); }
SEXP (R_altrep_data2)(SEXP x) { return seen(CDR_(x)); }
void (R_set_altrep_data1)(SEXP x, SEXP v) { robigo_rtapi_store(x, &CAR_(x), v); }
void (R_set_altrep_data2)(SEXP x, SEXP v) { robigo_rtapi_store(x, &CDR_(x), v); }

SEXP robigo_rtapi_printname(SEXP x);
/* GNU R's CAR read: a symbol's CAR is its PRINTNAME (one field there),
 * which is the isolate's (H5): the symbol's own CAR is the arena's copy of
 * the name, which C must never see. Every accessor's last CAR is this. */
static inline SEXP car_of(SEXP e)
{
    return RB_TYPE(e) == SYMSXP ? robigo_rtapi_printname(e) : CAR_(e);
}
SEXP (CAR)(SEXP e) { return seen(car_of(e)); }
SEXP (CDR)(SEXP e) { return seen(CDR_(e)); }
SEXP (TAG)(SEXP e) { return seen(TAG_(e)); }
SEXP (CAAR)(SEXP e) { return seen(car_of(CAR_(e))); }
SEXP (CDAR)(SEXP e) { return seen(CDR_(CAR_(e))); }
SEXP (CADR)(SEXP e) { return seen(car_of(CDR_(e))); }
SEXP (CDDR)(SEXP e) { return seen(CDR_(CDR_(e))); }
SEXP (CDDDR)(SEXP e) { return seen(CDR_(CDR_(CDR_(e)))); }
SEXP (CADDR)(SEXP e) { return seen(car_of(CDR_(CDR_(e)))); }
SEXP (CADDDR)(SEXP e) { return seen(car_of(CDR_(CDR_(CDR_(e))))); }
SEXP (CAD4R)(SEXP e) { return seen(car_of(CDR_(CDR_(CDR_(CDR_(e)))))); }
SEXP (CAD5R)(SEXP e) { return seen(car_of(CDR_(CDR_(CDR_(CDR_(CDR_(e))))))); }

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
    if (t == EXTPTRSXP || t == SPECIALSXP || t == BUILTINSXP || t == ENVSXP)
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
    return seen(car_of(x));
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
    /* (its trailing newline is removed by Cx::warning) */
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

/* R: printutils.c:Rvprintf and REvprintf, as Rscript has them: every byte
 * vsnprintf makes reaches the console (GNU R's vfprintf to R_Outputfile or
 * R_Consolefile). Text longer than the buffer is formatted again into a
 * heap block, and if that cannot be had the truncated text is written, as
 * REvprintf does without a console file. */
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
    /* (the console write is a leaf: it neither collects nor jumps, so the
     * block is always freed) */
    char *big = malloc((size_t) n + 1);
    if (big != NULL && vsnprintf(big, (size_t) n + 1, format, ap) == n)
        robigo_rtapi_print(to_stderr, big, (size_t) n);
    else
        robigo_rtapi_print(to_stderr, buf, strlen(buf));
    free(big);
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

/* ---- C-level tryCatch (errors.c) --------------------------------------- */

/* errors.c's tryCatchData_t; trycatch.rs reads it (TryCatchData) */
typedef struct {
    SEXP (*body)(void *);
    void *bdata;
    SEXP (*handler)(SEXP, void *);
    void *hdata;
    void (*finally)(void *);
    void *fdata;
    int suspended;
} robigo_trycatch_t;

SEXP robigo_rtapi_try_catch(robigo_trycatch_t *tcd, SEXP conds, int fin);
SEXP robigo_rtapi_push_error_handler(robigo_trycatch_t *tcd);
void robigo_rtapi_set_handler_stack(SEXP old);

static SEXP default_tryCatch_handler(SEXP cond, void *data)
{
    (void)cond;
    (void)data;
    return R_NilValue;
}

static void default_tryCatch_finally(void *data)
{
    (void)data;
}

/* R: errors.c:R_tryCatchError */
SEXP R_tryCatchError(SEXP (*body)(void *), void *bdata, SEXP (*handler)(SEXP, void *), void *hdata)
{
    SEXP val;
    SEXP cond = Rf_mkString("error");

    Rf_protect(cond);
    val = R_tryCatch(body, bdata, cond, handler, hdata, NULL, NULL);
    Rf_unprotect(1);
    return val;
}

/* R: errors.c:R_tryCatch. The R part (the interrupts' state, the call of
 * trycatch_callback) is the entry's (trycatch.rs). */
SEXP R_tryCatch(SEXP (*body)(void *), void *bdata, SEXP conds, SEXP (*handler)(SEXP, void *), void *hdata,
                void (*finally)(void *), void *fdata)
{
    if (body == NULL)
        Rf_error("must supply a body function");
    robigo_trycatch_t tcd = {
        .body = body,
        .bdata = bdata,
        .handler = handler != NULL ? handler : default_tryCatch_handler,
        .hdata = hdata,
        .finally = finally != NULL ? finally : default_tryCatch_finally,
        .fdata = fdata,
        .suspended = 0,
    };
    if (conds == NULL)
        conds = Rf_allocVector(STRSXP, 0);
    Rf_protect(conds);
    SEXP val = robigo_rtapi_try_catch(&tcd, conds, finally != NULL);
    Rf_unprotect(1);
    return val;
}

/* R: errors.c:R_withCallingErrorHandler. The handler's closure and entry
 * are the entry's (trycatch.rs); the body is called here, from C, as GNU R
 * calls it. */
SEXP R_withCallingErrorHandler(SEXP (*body)(void *), void *bdata, SEXP (*handler)(SEXP, void *), void *hdata)
{
    if (body == NULL)
        Rf_error("must supply a body function");
    robigo_trycatch_t tcd = {
        .handler = handler != NULL ? handler : default_tryCatch_handler,
        .hdata = hdata,
    };
    SEXP oldstack = robigo_rtapi_push_error_handler(&tcd);
    Rf_protect(oldstack);
    SEXP val = body(bdata);
    robigo_rtapi_set_handler_stack(oldstack);
    Rf_unprotect(1);
    return val;
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
    if (cont == R_NilValue) {
        /* not a token (NULL asks for one): GNU R calls fun, then its
         * SETCAR(cont, result) fails on R_NilValue, inside the unwind
         * context, whose call is R_NilValue (memory.c:SETCAR) */
        fun(data);
        robigo_rtapi_errorcall(R_NilValue, "bad value");
    }
    Rboolean jump = robigo_rtapi_unwind_protect(fun, data, cont, &result);
    cleanfun(cleandata, jump);
    if (jump)
        R_ContinueUnwind(cont);
    return seen(result);
}

/* ---- what GNU R's base-package natives use (the A5 note) ------------- */

/* R: memory.c:R_chk_calloc, R_chk_realloc, R_chk_free (glibc's calloc
 * works for 0 elements: HAVE_WORKING_CALLOC) */
void *R_chk_calloc(size_t nelem, size_t elsize)
{
    void *p = calloc(nelem, elsize);
    if (!p) {
        char msg[128];
        snprintf(msg, sizeof msg, "'R_Calloc' could not allocate memory (%llu of %llu bytes)",
                 (unsigned long long) nelem, (unsigned long long) elsize);
        robigo_rtapi_error(msg);
    }
    return p;
}

void *R_chk_realloc(void *ptr, size_t size)
{
    void *p;
    /* Protect against broken realloc */
    if (ptr) p = realloc(ptr, size); else p = malloc(size);
    if (!p) {
        char msg[128];
        snprintf(msg, sizeof msg, "'R_Realloc' could not re-allocate memory (%llu bytes)", (unsigned long long) size);
        robigo_rtapi_error(msg);
    }
    return p;
}

void R_chk_free(void *ptr)
{
    if (ptr) free(ptr);
}

/* R: Rinlinedfuns.h's predicates, as libR exports them */
Rboolean (Rf_isNull)(SEXP s) { return RB_TYPE(s) == NILSXP; }
Rboolean (Rf_isLogical)(SEXP s) { return RB_TYPE(s) == LGLSXP; }
Rboolean (Rf_isEnvironment)(SEXP s) { return RB_TYPE(s) == ENVSXP; }
Rboolean (Rf_isString)(SEXP s) { return RB_TYPE(s) == STRSXP; }

/* R: arithmetic.c:imax2 */
int Rf_imax2(int x, int y) { return (x < y) ? y : x; }

/* R: util.c:Rf_strchr, Rf_strchr_const. GNU R walks multibyte characters
 * only in a multibyte locale other than UTF-8; robigo models UTF-8 and C,
 * where it is strchr. */
char *Rf_strchr(char *s, int c) { return strchr(s, c); }
const char *Rf_strchr_const(const char *s, int c) { return strchr(s, c); }

/* R: platform.c's dirent wrappers (Unix) */
#include <dirent.h>
#include <errno.h>
struct R_dirent { char *d_name; };
struct R_DIR_INTERNAL { DIR *dirp; struct R_dirent de; };
typedef struct R_DIR_INTERNAL R_DIR;
R_DIR *R_opendir(const char *name);
struct R_dirent *R_readdir(R_DIR *rdir);
int R_closedir(R_DIR *rdir);

R_DIR *R_opendir(const char *name)
{
    R_DIR *rdir = malloc(sizeof(R_DIR));
    if (!rdir) {
        errno = ENOMEM;
        return NULL;
    }
    rdir->dirp = opendir(name);
    if (!rdir->dirp) {
        free(rdir);
        return NULL;
    }
    return rdir;
}

struct R_dirent *R_readdir(R_DIR *rdir)
{
    if (!rdir) {
        errno = EFAULT;
        return NULL;
    }
    struct dirent *de = readdir(rdir->dirp);
    if (de) {
        rdir->de.d_name = de->d_name;
        return &rdir->de;
    } else
        return NULL;
}

int R_closedir(R_DIR *rdir)
{
    if (!rdir) {
        errno = EFAULT;
        return -1;
    }
    int res = closedir(rdir->dirp);
    free(rdir);
    return res;
}

/* R: memory.c's precious multi-sets, composed of the API as GNU R composes
 * them (the allocations are entries; PROTECT is the protect stack's) */
SEXP R_NewPreciousMSet(int initialSize)
{
    SEXP npreserved, mset, isize;

    /* npreserved is modified in place */
    npreserved = Rf_allocVector(INTSXP, 1);
    INTEGER(npreserved)[0] = 0;
    Rf_protect(mset = Rf_cons(R_NilValue, npreserved));
    /* isize is not modified in place */
    if (initialSize < 0)
        robigo_rtapi_error("'initialSize' must be non-negative");
    isize = Rf_ScalarInteger(initialSize);
    SET_TAG(mset, isize);
    Rf_unprotect(1); /* mset */
    return mset;
}

static void checkMSet(SEXP mset)
{
    SEXP store = CAR(mset);
    SEXP npreserved = CDR(mset);
    SEXP isize = TAG(mset);
    if (((store != R_NilValue) && (RB_TYPE(store) != VECSXP)) ||
        (RB_TYPE(npreserved) != INTSXP || XLENGTH(npreserved) != 1) ||
        (RB_TYPE(isize) != INTSXP || XLENGTH(isize) != 1))
        robigo_rtapi_error("Invalid mset");
}

void R_PreserveInMSet(SEXP x, SEXP mset)
{
    if (x == R_NilValue || RB_TYPE(x) == SYMSXP)
        return; /* no need to preserve */
    Rf_protect(x);
    checkMSet(mset);
    SEXP store = CAR(mset);
    int *n = INTEGER(CDR(mset));
    if (store == R_NilValue) {
        R_xlen_t newsize = INTEGER(TAG(mset))[0];
        if (newsize == 0)
            newsize = 4; /* default minimum size */
        store = Rf_allocVector(VECSXP, newsize);
        SETCAR(mset, store);
    }
    R_xlen_t size = XLENGTH(store);
    if (*n == size) {
        R_xlen_t newsize = 2 * size;
        if (newsize >= INT_MAX || newsize < size)
            robigo_rtapi_error("Multi-set overflow");
        SEXP newstore = Rf_protect(Rf_allocVector(VECSXP, newsize));
        for (R_xlen_t i = 0; i < size; i++)
            SET_VECTOR_ELT(newstore, i, VECTOR_ELT(store, i));
        SETCAR(mset, newstore);
        Rf_unprotect(1); /* newstore */
        store = newstore;
    }
    Rf_unprotect(1); /* x */
    SET_VECTOR_ELT(store, (*n)++, x);
}

void R_ReleaseFromMSet(SEXP x, SEXP mset)
{
    if (x == R_NilValue || RB_TYPE(x) == SYMSXP)
        return; /* not preserved */
    checkMSet(mset);
    SEXP store = CAR(mset);
    if (store == R_NilValue)
        return; /* not preserved */
    int *n = INTEGER(CDR(mset));
    for (R_xlen_t i = (*n) - 1; i >= 0; i--) {
        if (VECTOR_ELT(store, i) == x) {
            for (; i < (*n) - 1; i++)
                SET_VECTOR_ELT(store, i, VECTOR_ELT(store, i + 1));
            SET_VECTOR_ELT(store, i, R_NilValue);
            (*n)--;
            return;
        }
    }
    /* not preserved */
}
