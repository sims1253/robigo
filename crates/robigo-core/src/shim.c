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
