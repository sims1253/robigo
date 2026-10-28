/* robigo-core's C shim: what Rust cannot express.
 *
 * - Contexts for non-local exits: Rust cannot call setjmp (it returns
 *   twice), so every entry into code that may longjmp (package C code,
 *   R_ToplevelExec, R_UnwindProtect, ...) goes through robigo_try, and
 *   Rf_error & co. leave through robigo_jump, exactly like GNU R's
 *   contexts (src/main/context.c).
 * - The register spill for the conservative stack scan. */
#include <setjmp.h>
#include <stdlib.h>

typedef struct robigo_ctx {
    struct robigo_ctx *prev;
    jmp_buf jb;
} robigo_ctx;

static __thread robigo_ctx *robigo_top = 0;

/* Run body(data). Returns 0 if it returned normally, 1 if robigo_jump()
 * unwound to this context. */
int robigo_try(void (*body)(void *), void *data)
{
    robigo_ctx c;
    c.prev = robigo_top;
    robigo_top = &c;
    if (setjmp(c.jb) == 0) {
        body(data);
        robigo_top = c.prev;
        return 0;
    }
    robigo_top = c.prev;
    return 1;
}

/* Unwind to the innermost robigo_try. */
void robigo_jump(void)
{
    robigo_ctx *c = robigo_top;
    if (!c)
        abort();
    longjmp(c->jb, 1);
}

int robigo_try_depth(void)
{
    int n = 0;
    for (robigo_ctx *c = robigo_top; c; c = c->prev)
        n++;
    return n;
}

/* Spill the callee-saved registers into this frame, then call cb: every
 * register-held root is then on the stack below the scan's upper bound
 * and above cb's frame. */
void robigo_spill(void (*cb)(void *), void *ctx)
{
    __builtin_unwind_init();
    cb(ctx);
    __asm__ volatile("" ::: "memory");
}

/* ---- C trampolines: native code runs with no Rust frame between the
 * context's setjmp and the code that may longjmp (council item 17). Each
 * returns 0 on a normal return, 1 if robigo_jump() unwound to it. */

typedef void *SX;

#define TRY(body)                         \
    robigo_ctx c;                         \
    c.prev = robigo_top;                  \
    robigo_top = &c;                      \
    if (setjmp(c.jb) == 0) {              \
        body;                             \
        robigo_top = c.prev;              \
        return 0;                         \
    }                                     \
    robigo_top = c.prev;                  \
    return 1;

/* SEXP fun(void *) */
int robigo_try_sexp_ptr(SX (*f)(void *), void *data, SX *out)
{
    TRY(*out = f(data))
}

/* void fun(void *) — also void fun(SEXP) and R_init_<pkg>(DllInfo *) */
int robigo_try_void_ptr(void (*f)(void *), void *data)
{
    TRY(f(data))
}

/* void fun(void *, Rboolean) — R_UnwindProtect's cleanfun */
int robigo_try_void_ptr_int(void (*f)(void *, int), void *data, int jump)
{
    TRY(f(data, jump))
}

/* .Call and .External: SEXP fun(SEXP, ...) with up to 65 arguments.
 * On the SysV ABIs a callee ignores arguments it does not declare, so
 * one 65-argument call shape serves every arity (dotcode.c switches over
 * 65 casts instead). */
typedef SX (*F65)(SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX,
                  SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX,
                  SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX,
                  SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX,
                  SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX, SX);

#define A65(a) a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], \
    a[10], a[11], a[12], a[13], a[14], a[15], a[16], a[17], a[18], a[19], \
    a[20], a[21], a[22], a[23], a[24], a[25], a[26], a[27], a[28], a[29], \
    a[30], a[31], a[32], a[33], a[34], a[35], a[36], a[37], a[38], a[39], \
    a[40], a[41], a[42], a[43], a[44], a[45], a[46], a[47], a[48], a[49], \
    a[50], a[51], a[52], a[53], a[54], a[55], a[56], a[57], a[58], a[59], \
    a[60], a[61], a[62], a[63], a[64]

/* args has 65 slots (the unused ones R_NilValue / NULL) */
int robigo_try_call65(void *f, SX *args, SX *out)
{
    TRY(*out = ((F65) f)(A65(args)))
}
