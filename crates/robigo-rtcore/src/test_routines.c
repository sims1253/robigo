/* Package-code stand-ins for robigo-rtcore's tests (tests/contexts.rs):
 * routines, cends and cleanups that use the C API the way packages do, with
 * the API reached through the hooks the test passes in (its stand-ins for
 * robigo-rtapi's exports). Nothing else references this object, so the
 * linker leaves it out of binaries. */
#include <setjmp.h>
#include <Defn.h>

typedef struct {
    /* Rf_error: an exit to the innermost catcher of the test */
    void (*error)(const char *msg);
    void (*log)(const char *msg);
    /* begincontext(c, callflag, R_NilValue, ...), endcontext(c) */
    void (*begin)(RCNTXT *c, int callflag);
    void (*end)(RCNTXT *c);
    /* whether c is on the context chain now */
    int (*on_chain)(RCNTXT *c);
    /* R_ToplevelExec */
    int (*toplevel_exec)(void (*fun)(void *), void *data);
    /* R_UnwindProtect, R_ContinueUnwind */
    SEXP (*unwind_protect)(SEXP (*fun)(void *), void *data, void (*cleanfun)(void *, int), void *cleandata, SEXP cont);
    void (*continue_unwind)(SEXP cont);
    /* error(): reported against the current context's call */
    void (*error_here)(const char *msg);
    /* the test records what R code sees as R_GlobalContext now */
    void (*look)(void);
} rt_hooks;

/* ---- a C context's cend runs before the jump ---- */

typedef struct {
    const rt_hooks *h;
    volatile int *magic;
    RCNTXT *c;
} cend_data;

static void cend_check(void *p)
{
    cend_data *d = p;
    /* before the jump: the frame that holds the record (and magic) is
     * still there, and so is the record on the chain */
    int alive = *d->magic == 0x5eed;
    int linked = d->h->on_chain(d->c);
    d->h->log(alive && linked ? "cend: before the jump" : "cend: after the jump");
}

/* A routine that pushes a CTXT_CCODE record with a cend in its own frame,
 * then raises an error. */
SEXP rt_test_cend_case(const rt_hooks *h)
{
    RCNTXT c;
    volatile int magic = 0x5eed;
    cend_data d = { h, &magic, &c };
    h->begin(&c, CTXT_CCODE);
    c.cend = cend_check;
    c.cenddata = &d;
    h->error("from C");
    h->end(&c);
    magic = 0;
    return 0;
}

/* ---- a cend that raises its own error supersedes the jump ---- */

static void cend_log(void *p)
{
    cend_data *d = p;
    d->h->log("outer cend");
}

static void cend_raise(void *p)
{
    cend_data *d = p;
    d->h->log("inner cend");
    d->h->error("from the cend");
}

/* Two C records: the inner one's cend raises an error while the routine's
 * own error unwinds; the outer one's cend still runs, once. */
SEXP rt_test_cend_raises(const rt_hooks *h)
{
    RCNTXT outer, inner;
    volatile int magic = 0x5eed;
    cend_data d = { h, &magic, 0 };
    h->begin(&outer, CTXT_CCODE);
    outer.cend = cend_log;
    outer.cenddata = &d;
    h->begin(&inner, CTXT_CCODE);
    inner.cend = cend_raise;
    inner.cenddata = &d;
    h->error("from C");
    h->end(&inner);
    h->end(&outer);
    return 0;
}

/* ---- R_UnwindProtect ---- */

static SEXP fun_outer(void *p)
{
    const rt_hooks *h = p;
    h->log("fun");
    h->error("outer");
    return 0;
}

static void inner_error(void *p)
{
    const rt_hooks *h = p;
    h->error("inner");
}

/* A cleanup that catches an error of its own (R_ToplevelExec around it). */
static void clean_catches(void *p, int jump)
{
    const rt_hooks *h = p;
    h->log(jump ? "cleanup: jump" : "cleanup: no jump");
    int ok = h->toplevel_exec(inner_error, p);
    h->log(ok ? "cleanup: inner returned" : "cleanup: inner caught");
}

SEXP rt_test_unwind_case(const rt_hooks *h, SEXP cont)
{
    h->unwind_protect(fun_outer, (void *) h, clean_catches, (void *) h, cont);
    h->log("after R_UnwindProtect");
    return 0;
}

/* cpp11's shape: the cleanup longjmps to the routine's own setjmp, and the
 * routine resumes the unwind later with R_ContinueUnwind. */
typedef struct {
    const rt_hooks *h;
    jmp_buf jb;
} cpp_data;

static void clean_longjmp(void *p, int jump)
{
    cpp_data *d = p;
    d->h->log(jump ? "cleanup: jump" : "cleanup: no jump");
    if (jump)
        longjmp(d->jb, 1);
}

SEXP rt_test_cpp11_case(const rt_hooks *h, SEXP cont)
{
    cpp_data d;
    d.h = h;
    if (setjmp(d.jb) == 0) {
        h->unwind_protect(fun_outer, (void *) h, clean_longjmp, &d, cont);
        h->log("after R_UnwindProtect");
        return 0;
    }
    /* destructors would run here */
    h->log("routine: unwinding C++ frames");
    h->continue_unwind(cont);
    h->log("after R_ContinueUnwind");
    return 0;
}

/* A normal return: fun's value is R_UnwindProtect's. */
static SEXP fun_returns(void *p)
{
    (void) p;
    return (SEXP) 0;
}

SEXP rt_test_unwind_returns(const rt_hooks *h, SEXP cont)
{
    h->unwind_protect(fun_returns, (void *) h, clean_catches, (void *) h, cont);
    h->log("after R_UnwindProtect");
    return 0;
}

/* ---- calls and contexts as R code sees them ---- */

/* error() from package code: reported against the R call. */
SEXP rt_test_error_here(const rt_hooks *h)
{
    h->error_here("from error()");
    return 0;
}

/* A callback GNU R makes no context for (an ALTREP method): what does R
 * code see as the current context? */
SEXP rt_test_look(const rt_hooks *h)
{
    h->look();
    return 0;
}

/* ---- R_ExecWithCleanup ---- */

typedef struct {
    const rt_hooks *h;
    SEXP value;
    int fail;
    int cleanups;
} exec_data;

SEXP rt_test_exec_fun(void *p)
{
    exec_data *d = p;
    d->h->log("fun");
    if (d->fail == 1)
        d->h->error("fun failed");
    return d->value;
}

void rt_test_exec_cleanup(void *p)
{
    exec_data *d = p;
    d->cleanups++;
    d->h->log("cleanup");
    if (d->fail == 2 && d->cleanups == 1)
        d->h->error("cleanup failed");
}

/* ---- R_ToplevelExec ---- */

void rt_test_toplevel_fun(void *p)
{
    exec_data *d = p;
    d->h->log("toplevel fun");
    if (d->fail)
        d->h->error("toplevel failed");
}

/* ---- six-argument entries return doubles and complexes ---- */

double rt_test_half(long a, long b)
{
    return (double) (a + b) / 2;
}

typedef struct {
    double r, i;
} rt_cplx;

rt_cplx rt_test_cplx(long a, long b)
{
    rt_cplx z = { (double) a, (double) b };
    return z;
}
