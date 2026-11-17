/* robigo's Defn.h: the parts of GNU R's private header that robigo-rt
 * implements, for C compiled against robigo (robigo-rtapi's own C, and GNU
 * R's C where docs/g3.md §10 adopts it as a crutch). GNU R 4.6.2's
 * src/include/Defn.h is the spec; each declaration here has GNU R's name,
 * type and field order.
 *
 * Contexts (docs/g3.md §6.5): one chain of RCNTXT records, GNU R's, at
 * stable addresses. The runtime (robigo-rtcore, context.rs) declares the
 * same layout as a #[repr(C)] struct, and the two are checked against each
 * other (context.c's layout table, compared field by field when an isolate
 * is made and in the crate's tests). A record's cjmpbuf is the jump record
 * of the native entry that pushed it ("a trampoline's jump record is a
 * field of its CTXT_CCODE record"); robigo's extension (ROBIGO_CNTXT_EXT)
 * follows GNU R's fields.
 *
 * The object layout: GNU R's sxpinfo.gp masks, from robigo_layout.h, the
 * header robigo-heap's build writes from its layout module (the one owner
 * of every offset and header bit). Their values are GNU R's, which the
 * assertions below hold them to. */
#ifndef ROBIGO_DEFN_H_
#define ROBIGO_DEFN_H_

#include <setjmp.h>
#include <stddef.h>
#include <stdint.h>
#include <robigo_layout.h>

/* (Rinternals.h's, repeated: identical typedefs may repeat in C11) */
typedef struct SEXPREC *SEXP;

/* Defn.h under HAVE_POSIX_SETJMP */
#define SIGJMP_BUF sigjmp_buf
#define JMP_BUF sigjmp_buf
#define SETJMP(x) sigsetjmp(x, 0)
#define LONGJMP(x, i) siglongjmp(x, i)

/* The byte-code interpreter's stack entry (Defn.h). robigo's VM keeps its
 * own state; in a context only returnValue uses it, with tag 0 and sxpval
 * (SEXP_TO_STACKVAL). */
typedef struct {
    int tag;
    int flags;
    union {
	int ival;
	double dval;
	SEXP sxpval;
    } u;
} R_bcstack_t;

typedef struct R_bcFrame R_bcFrame_type;

/* Stack entry for pending promises */
typedef struct RPRSTACK {
    SEXP promise;
    struct RPRSTACK *next;
} RPRSTACK;

/* robigo's part of a context: what GNU R keeps in globals or C locals of
 * the frame that pushed it, and does not need in the record.
 *
 * flags      ROBIGO_CNTXT_LANDING: cjmpbuf is set by a native entry that
 *            is still running (an exit may longjmp to it);
 *            ROBIGO_CNTXT_HIDDEN: GNU R has no context here (the native
 *            entry of an ALTREP method or a cend): R-level walks skip it;
 *            ROBIGO_CNTXT_LAZY_ARGS: promargs is not filled yet (nargs
 *            holds its length).
 * nargs      length(promargs) while it is lazy.
 * fillargs, filldata, fillkind
 *            the runtime's filler of a lazy promargs, its data and its
 *            kind (the owner's tag, 0 for none; opaque to C).
 * parked     the exit waiting in this context while its exit code runs
 *            (GNU R's jumptarget and jumpmask, which endcontext copies to a
 *            local, and R_ReturnedValue): present, kind, mask, target,
 *            value, condition, call.
 * scantop, scanroutine, scanchain
 *            the C segments' state when the record was pushed (the open
 *            segment's top and routine, the innermost closed segment):
 *            an exit restores it at the record as GNU R restores
 *            R_PPStackTop (docs/g3.md §6.5). */
typedef struct {
    unsigned int flags;
    int nargs;
    void *fillargs;
    void *filldata;
    unsigned int parked;
    unsigned int parkedkind;
    int parkedmask;
    unsigned int fillkind;
    struct RCNTXT *parkedtarget;
    SEXP parkedvalue;
    SEXP parkedcondition;
    SEXP parkedcall;
    uintptr_t scantop;
    uintptr_t scanroutine;
    const void *scanchain;
} ROBIGO_CNTXT_EXT;

#define ROBIGO_CNTXT_LANDING 1u
#define ROBIGO_CNTXT_HIDDEN 2u
#define ROBIGO_CNTXT_LAZY_ARGS 4u

/* Evaluation Context Structure (GNU R 4.6.2's fields, in its order) */
typedef struct RCNTXT {
    struct RCNTXT *nextcontext;	/* The next context up the chain */
    int callflag;		/* The context "type" */
    JMP_BUF cjmpbuf;		/* C stack and register information */
    int cstacktop;		/* Top of the pointer protection stack */
    int evaldepth;	        /* evaluation depth at inception */
    SEXP promargs;		/* Promises supplied to closure */
    SEXP callfun;		/* The closure called */
    SEXP sysparent;		/* environment the closure was called from */
    SEXP call;			/* The call that effected this context*/
    SEXP cloenv;		/* The environment */
    SEXP conexit;		/* Interpreted "on.exit" code */
    void (*cend)(void *);	/* C "on.exit" thunk */
    void *cenddata;		/* data for C "on.exit" thunk */
    void *vmax;		        /* top of R_alloc stack */
    int intsusp;                /* interrupts are suspended */
    int gcenabled;		/* R_GCEnabled value */
    int bcintactive;            /* R_BCIntActive value */
    SEXP bcbody;                /* R_BCbody value */
    void* bcpc;                 /* R_BCpc value */
    ptrdiff_t relpc;            /* pc offset when begincontext is called */
    SEXP handlerstack;          /* condition handler stack */
    SEXP restartstack;          /* stack of available restarts */
    struct RPRSTACK *prstack;   /* stack of pending promises */
    R_bcstack_t *nodestack;
    R_bcstack_t *bcprottop;
    R_bcFrame_type *bcframe;
    SEXP srcref;	        /* The source line in effect */
    int browserfinish;          /* should browser finish this context without
                                   stopping */
    R_bcstack_t returnValue;    /* only set during on.exit calls */
    struct RCNTXT *jumptarget;	/* target for a continuing jump */
    int jumpmask;               /* associated LONGJMP argument */
    ROBIGO_CNTXT_EXT robigo;    /* robigo's */
} RCNTXT, *context;

/* The Various Context Types (GNU R's). */
enum {
    CTXT_TOPLEVEL = 0,
    CTXT_NEXT	  = 1,
    CTXT_BREAK	  = 2,
    CTXT_LOOP	  = 3,	/* break OR next target */
    CTXT_FUNCTION = 4,
    CTXT_CCODE	  = 8,
    CTXT_RETURN	  = 12,
    CTXT_BROWSER  = 16,
    CTXT_GENERIC  = 20,
    CTXT_RESTART  = 32,
    CTXT_BUILTIN  = 64, /* used in profiling */
    CTXT_UNWIND   = 128
};

/* sxpinfo.gp masks (GNU R 4.6.2's Defn.h; the line of each in a comment) */
#define ASSIGNMENT_PENDING_MASK ROBIGO_GP_CELL_ASSIGNMENT_PENDING /* 320 */
#define S4_OBJECT_MASK ((unsigned short) ROBIGO_GP_S4) /* 367 */
#define NOJIT_MASK ((unsigned short) ROBIGO_GP_CLOSURE_NOJIT) /* 373 */
#define MAYBEJIT_MASK ((unsigned short) ROBIGO_GP_CLOSURE_MAYBEJIT) /* 376 */
#define GROWABLE_MASK ((unsigned short) ROBIGO_GP_VEC_GROWABLE) /* 382 */
#define MISSING_MASK ROBIGO_GP_CELL_MISSING /* 456 */
#define DDVAL_MASK ROBIGO_GP_SYM_DDVAL /* 526 */
#define BYTES_MASK ROBIGO_GP_CHAR_BYTES /* 934 */
#define LATIN1_MASK ROBIGO_GP_CHAR_LATIN1 /* 935 */
#define UTF8_MASK ROBIGO_GP_CHAR_UTF8 /* 936 */
#define CACHED_MASK ROBIGO_GP_CHAR_CACHED /* 938 */
#define ASCII_MASK ROBIGO_GP_CHAR_ASCII /* 939 */
#define HASHASH_MASK ROBIGO_GP_CHAR_HASHASH /* 940 */
#define ACTIVE_BINDING_MASK ROBIGO_GP_CELL_ACTIVE_BINDING /* 1308 */
#define BINDING_LOCK_MASK ROBIGO_GP_CELL_BINDING_LOCK /* 1309 */
#define SPECIAL_BINDING_MASK (ACTIVE_BINDING_MASK | BINDING_LOCK_MASK) /* 1310 */
#define SPECIAL_SYMBOL_MASK ROBIGO_GP_SYM_SPECIAL_SYMBOL /* 1331 */

/* robigo's layout keeps GNU R's bits (C1x _Static_assert) */
_Static_assert(ASSIGNMENT_PENDING_MASK == (1 << 11), "ASSIGNMENT_PENDING_MASK");
_Static_assert(S4_OBJECT_MASK == (1 << 4), "S4_OBJECT_MASK");
_Static_assert(NOJIT_MASK == (1 << 5) && MAYBEJIT_MASK == (1 << 6), "NOJIT_MASK, MAYBEJIT_MASK");
_Static_assert(GROWABLE_MASK == (1 << 5), "GROWABLE_MASK");
_Static_assert(MISSING_MASK == 15 && DDVAL_MASK == 1, "MISSING_MASK, DDVAL_MASK");
_Static_assert(BYTES_MASK == (1 << 1) && LATIN1_MASK == (1 << 2) && UTF8_MASK == (1 << 3), "encoding masks");
_Static_assert(CACHED_MASK == (1 << 5) && ASCII_MASK == (1 << 6) && HASHASH_MASK == 1, "CHARSXP masks");
_Static_assert(ACTIVE_BINDING_MASK == (1 << 15) && BINDING_LOCK_MASK == (1 << 14), "binding masks");
_Static_assert(ACTIVE_BINDING_MASK == ROBIGO_GP_SYM_ACTIVE_BINDING && BINDING_LOCK_MASK == ROBIGO_GP_SYM_BINDING_LOCK,
               "the same bits (15 and 14) in symbols and bindings");
_Static_assert(SPECIAL_SYMBOL_MASK == (1 << 12) && SPECIAL_SYMBOL_MASK == ROBIGO_GP_ENV_NO_SPECIAL_SYMBOLS,
               "SPECIAL_SYMBOL_MASK");
/* envir.c's own FRAME_LOCK_MASK and GLOBAL_FRAME_MASK */
_Static_assert(ROBIGO_GP_ENV_FRAME_LOCK == (1 << 14) && ROBIGO_GP_ENV_GLOBAL_FRAME == (1 << 15), "ENVFLAGS");

#define IS_RESTART_BIT_SET(flags) ((flags) & CTXT_RESTART)
#define SET_RESTART_BIT_ON(flags) (flags |= CTXT_RESTART)
#define SET_RESTART_BIT_OFF(flags) (flags &= ~CTXT_RESTART)

#endif /* ROBIGO_DEFN_H_ */
