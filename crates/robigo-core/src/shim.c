/* robigo-core's C shim: what Rust cannot express.
 *
 * - Contexts for non-local exits: Rust cannot call setjmp (it returns
 *   twice), so every entry into code that may longjmp (package C code,
 *   R_ToplevelExec, R_UnwindProtect, ...) goes through a robigo_try_* trampoline, and
 *   Rf_error & co. leave through robigo_jump, exactly like GNU R's
 *   contexts (src/main/context.c).
 * - The register spill for the conservative stack scan.
 *
 * Built without unwind tables (build.rs): nothing may unwind through these
 * frames, and without tables a C++ exception cannot. */
#include <setjmp.h>
#include <stdlib.h>

typedef struct robigo_ctx {
    struct robigo_ctx *prev;
    jmp_buf jb;
} robigo_ctx;

static __thread robigo_ctx *robigo_top = 0;

/* Unwind to the innermost context. */
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

#if !defined(__x86_64__)
/* (x86-64: robigo_call65_stub zeroes in place) */
/* Zero the 1 KiB of stack below the caller's frame, where the trampoline
 * and the first frames of the package code it calls will be: what they
 * leave unwritten (a jmp_buf's signal mask, which _setjmp never writes,
 * alignment slots, unused locals) then reads as zero, not as a stale
 * object pointer of earlier, deeper frames, while the conservative scan
 * reads those frames. A separate frame, so the stores cannot be elided or
 * reach the caller's own. */
#if defined(__has_attribute)
#if __has_attribute(no_stack_protector)
/* (no canary: the buffer then reaches the saved frame pointer, and the
 * trampoline's own return address and frame pointer overwrite those) */
#define ROBIGO_NO_CANARY no_stack_protector,
#endif
#endif
#ifndef ROBIGO_NO_CANARY
#define ROBIGO_NO_CANARY
#endif
__attribute__((ROBIGO_NO_CANARY noinline)) static void robigo_clear_stack(void)
{
    char buf[1024];
    __builtin_memset(buf, 0, sizeof buf);
    __asm__ volatile("" : : "r"(buf) : "memory");
}
#endif

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

#include <stdint.h>

#if defined(__x86_64__)
/* f(args[0], ..., args[64]) with an exact stack layout (SysV x86-64:
 * six register arguments, 59 on the stack), which a C call does not give:
 * the compiler chooses where the trampoline's frame ends and the outgoing
 * arguments begin. The stub zeroes the 1 KiB below its saved registers,
 * reserves exactly the 59 stack-argument slots (472 bytes, which keeps the
 * call 16-byte aligned), fills them, and, if scan_top is not null, sets
 * the conservative scan's limit to the first word above them. The routine's
 * incoming parameters are then scanned, whatever it stores in them (a C
 * local, §3), and nothing above is: the C trampoline's frame (its jmp_buf
 * has a signal mask _setjmp never writes) and the Rust frames hold objects
 * only through precise roots. rdi = f, rsi = args, rdx = scan_top; the
 * caller restores the limit. */
#define CALL65_STUB_ENTRY                \
        "push %rbp\n\t"                   \
        "mov %rsp, %rbp\n\t"              \
        "push %rbx\n\t"                   \
        "mov %rdi, %r11\n\t"              \
        "mov %rdx, %rbx\n\t"
/* zero the 1 KiB below */
#define CALL65_STUB_ZERO \
        "pxor %xmm0, %xmm0\n\t" \
        "lea -1024(%rsp), %rdi\n\t" \
        "mov $8, %ecx\n\t" \
        "1:\n\t" \
        "movups %xmm0, (%rdi)\n\t" \
        "movups %xmm0, 16(%rdi)\n\t" \
        "movups %xmm0, 32(%rdi)\n\t" \
        "movups %xmm0, 48(%rdi)\n\t" \
        "movups %xmm0, 64(%rdi)\n\t" \
        "movups %xmm0, 80(%rdi)\n\t" \
        "movups %xmm0, 96(%rdi)\n\t" \
        "movups %xmm0, 112(%rdi)\n\t" \
        "add $128, %rdi\n\t" \
        "dec %ecx\n\t" \
        "jnz 1b\n\t"
#define CALL65_STUB_CALL \
        /* args[6..65] into the 59 stack-argument slots */ \
        "sub $472, %rsp\n\t" \
        "movups 48(%rsi), %xmm0\n\t" \
        "movups %xmm0, 0(%rsp)\n\t" \
        "movups 64(%rsi), %xmm0\n\t" \
        "movups %xmm0, 16(%rsp)\n\t" \
        "movups 80(%rsi), %xmm0\n\t" \
        "movups %xmm0, 32(%rsp)\n\t" \
        "movups 96(%rsi), %xmm0\n\t" \
        "movups %xmm0, 48(%rsp)\n\t" \
        "movups 112(%rsi), %xmm0\n\t" \
        "movups %xmm0, 64(%rsp)\n\t" \
        "movups 128(%rsi), %xmm0\n\t" \
        "movups %xmm0, 80(%rsp)\n\t" \
        "movups 144(%rsi), %xmm0\n\t" \
        "movups %xmm0, 96(%rsp)\n\t" \
        "movups 160(%rsi), %xmm0\n\t" \
        "movups %xmm0, 112(%rsp)\n\t" \
        "movups 176(%rsi), %xmm0\n\t" \
        "movups %xmm0, 128(%rsp)\n\t" \
        "movups 192(%rsi), %xmm0\n\t" \
        "movups %xmm0, 144(%rsp)\n\t" \
        "movups 208(%rsi), %xmm0\n\t" \
        "movups %xmm0, 160(%rsp)\n\t" \
        "movups 224(%rsi), %xmm0\n\t" \
        "movups %xmm0, 176(%rsp)\n\t" \
        "movups 240(%rsi), %xmm0\n\t" \
        "movups %xmm0, 192(%rsp)\n\t" \
        "movups 256(%rsi), %xmm0\n\t" \
        "movups %xmm0, 208(%rsp)\n\t" \
        "movups 272(%rsi), %xmm0\n\t" \
        "movups %xmm0, 224(%rsp)\n\t" \
        "movups 288(%rsi), %xmm0\n\t" \
        "movups %xmm0, 240(%rsp)\n\t" \
        "movups 304(%rsi), %xmm0\n\t" \
        "movups %xmm0, 256(%rsp)\n\t" \
        "movups 320(%rsi), %xmm0\n\t" \
        "movups %xmm0, 272(%rsp)\n\t" \
        "movups 336(%rsi), %xmm0\n\t" \
        "movups %xmm0, 288(%rsp)\n\t" \
        "movups 352(%rsi), %xmm0\n\t" \
        "movups %xmm0, 304(%rsp)\n\t" \
        "movups 368(%rsi), %xmm0\n\t" \
        "movups %xmm0, 320(%rsp)\n\t" \
        "movups 384(%rsi), %xmm0\n\t" \
        "movups %xmm0, 336(%rsp)\n\t" \
        "movups 400(%rsi), %xmm0\n\t" \
        "movups %xmm0, 352(%rsp)\n\t" \
        "movups 416(%rsi), %xmm0\n\t" \
        "movups %xmm0, 368(%rsp)\n\t" \
        "movups 432(%rsi), %xmm0\n\t" \
        "movups %xmm0, 384(%rsp)\n\t" \
        "movups 448(%rsi), %xmm0\n\t" \
        "movups %xmm0, 400(%rsp)\n\t" \
        "movups 464(%rsi), %xmm0\n\t" \
        "movups %xmm0, 416(%rsp)\n\t" \
        "movups 480(%rsi), %xmm0\n\t" \
        "movups %xmm0, 432(%rsp)\n\t" \
        "movups 496(%rsi), %xmm0\n\t" \
        "movups %xmm0, 448(%rsp)\n\t" \
        "mov 512(%rsi), %rax\n\t" \
        "mov %rax, 464(%rsp)\n\t" \
        /* the scan's limit: the first word above them */ \
        "test %rbx, %rbx\n\t" \
        "jz 2f\n\t" \
        "lea 472(%rsp), %rax\n\t" \
        "mov %rax, (%rbx)\n\t" \
        "2:\n\t" \
        "mov (%rsi), %rdi\n\t" \
        "mov 16(%rsi), %rdx\n\t" \
        "mov 24(%rsi), %rcx\n\t" \
        "mov 32(%rsi), %r8\n\t" \
        "mov 40(%rsi), %r9\n\t" \
        "mov 8(%rsi), %rsi\n\t" \
        "call *%r11\n\t" \
        "lea -8(%rbp), %rsp\n\t" \
        "pop %rbx\n\t" \
        "pop %rbp\n\t" \
        "ret\n\t"

__attribute__((naked, noinline)) static SX robigo_call65_stub(__attribute__((unused)) void *f, __attribute__((unused)) SX *args,
                                                             __attribute__((unused)) uintptr_t *scan_top)
{
    __asm__ volatile(CALL65_STUB_ENTRY CALL65_STUB_ZERO CALL65_STUB_CALL);
}

/* Experiment A (docs/g3.md): the stub without the zeroing
 * (ROBIGO_CALL65_NO_ZERO=1). */
__attribute__((naked, noinline)) static SX robigo_call65_stub_nozero(__attribute__((unused)) void *f,
                                                                    __attribute__((unused)) SX *args,
                                                                    __attribute__((unused)) uintptr_t *scan_top)
{
    __asm__ volatile(CALL65_STUB_ENTRY CALL65_STUB_CALL);
}
#endif

/* A package routine (.Call, .External, .C), args has 65 slots (the unused
 * ones R_NilValue / NULL), which the caller protected.
 *
 * scan_top, if not null, is the core's conservative-scan limit. While the
 * routine runs, an open segment set above this frame (not 0, no scan, nor
 * UINTPTR_MAX, an embedder scanning all its frames) ends at the routine's
 * stack arguments instead (robigo_call65_stub): the frames above, this
 * one's included, hold objects only through precise roots, and the words
 * they leave unwritten are stale ones of earlier frames at that depth. The
 * narrowing has to be exact at the call: taken in Rust it left an inlined
 * frame's slots in the scan (a dead object kept alive in optimized
 * builds), and from a sampled stack pointer in C it left the routine's
 * stack parameters out with Clang, which reserves the outgoing arguments
 * in the prologue (a live object freed). Elsewhere than x86-64 the limit
 * stays where it is and only the stack below is zeroed: every live word
 * is scanned, stale words above may keep garbage alive (ledger
 * 2026-11-11). */
/* Experiment A (docs/g3.md §15.1), diagnostic switches read once per
 * thread: ROBIGO_CALL65_NO_NARROW=1 leaves the scan's limit where it is,
 * ROBIGO_CALL65_NO_ZERO=1 skips the zeroing, ROBIGO_CALL65_PLAIN=1 is both
 * (the routine is called directly, without the stub). ROBIGO_GC_PIN_REPORT
 * or ROBIGO_GC_STRONG_DLL set: each call is recorded on a stack the
 * collector reads to tell which routine's frames a stack word is in. */
enum { CALL65_NO_NARROW = 1, CALL65_NO_ZERO = 2, CALL65_PLAIN = 3, CALL65_DIAG = 4 };

static int env_is_1(const char *name)
{
    const char *e = getenv(name);
    return e && e[0] == '1' && e[1] == 0;
}

static int robigo_call65_mode(void)
{
    static _Thread_local int v = -1;
    if (v < 0) {
        const char *strong = getenv("ROBIGO_GC_STRONG_DLL");
        const char *report = getenv("ROBIGO_GC_PIN_REPORT");
        v = (env_is_1("ROBIGO_CALL65_PLAIN") ? CALL65_PLAIN : 0)
            | (env_is_1("ROBIGO_CALL65_NO_NARROW") ? CALL65_NO_NARROW : 0)
            | (env_is_1("ROBIGO_CALL65_NO_ZERO") ? CALL65_NO_ZERO : 0)
            | ((strong && strong[0]) || (report && report[0] && !(report[0] == '0' && !report[1])) ? CALL65_DIAG : 0);
    }
    return v;
}

/* The routines running, outermost first: the routine and the highest
 * address its frames (and, unnarrowed, the bridge frames below the scan's
 * limit) can be at (robigo_diag_calls). */
typedef struct robigo_diag_call {
    uintptr_t hi;
    void *f;
} robigo_diag_call;

#define DIAG_CALLS 4096
static __thread robigo_diag_call robigo_diag_stack[DIAG_CALLS];
static __thread int robigo_diag_n = 0;

const robigo_diag_call *robigo_diag_calls(int *n)
{
    *n = robigo_diag_n < DIAG_CALLS ? robigo_diag_n : DIAG_CALLS;
    return robigo_diag_stack;
}

/* robigo_try_call65 under a switch (a frame of its own, so the default
 * path's stays as it was). */
__attribute__((noinline)) static int robigo_try_call65_exp(void *f, SX *args, SX *out, uintptr_t *scan_top, int mode)
{
    robigo_ctx c;
    uintptr_t volatile old = scan_top ? *scan_top : 0;
#if defined(__x86_64__)
    int volatile narrow = !(mode & CALL65_NO_NARROW) && old > (uintptr_t) &c && old != UINTPTR_MAX;
#else
    int volatile narrow = 0;
#endif
    int volatile depth = robigo_diag_n;
    if (mode & CALL65_DIAG) {
        if (depth < DIAG_CALLS) {
            uintptr_t hi = (uintptr_t) __builtin_frame_address(0);
            /* unnarrowed, the open segment's frames above this one are
             * scanned too: the bridge's, for this call */
            if (!narrow && old > hi && old != UINTPTR_MAX)
                hi = old;
            robigo_diag_stack[depth].hi = hi;
            robigo_diag_stack[depth].f = f;
        }
        robigo_diag_n = depth + 1;
    }
    c.prev = robigo_top;
    robigo_top = &c;
    int volatile r = 1;
    if (setjmp(c.jb) == 0) {
        if ((mode & CALL65_PLAIN) == CALL65_PLAIN)
            *out = ((F65) f)(A65(args));
#if defined(__x86_64__)
        else if (mode & CALL65_NO_ZERO)
            *out = robigo_call65_stub_nozero(f, args, narrow ? scan_top : 0);
        else
            *out = robigo_call65_stub(f, args, narrow ? scan_top : 0);
#else
        else {
            if (!(mode & CALL65_NO_ZERO))
                robigo_clear_stack();
            *out = ((F65) f)(A65(args));
        }
#endif
        r = 0;
    }
    robigo_top = c.prev;
    robigo_diag_n = depth;
    if (narrow)
        *scan_top = old;
    return r;
}

int robigo_try_call65(void *f, SX *args, SX *out, uintptr_t *scan_top)
{
    int mode = robigo_call65_mode();
    if (mode)
        return robigo_try_call65_exp(f, args, out, scan_top, mode);
    robigo_ctx c;
    uintptr_t volatile old = scan_top ? *scan_top : 0;
#if defined(__x86_64__)
    /* (the stub sets the limit below this frame) */
    int volatile narrow = old > (uintptr_t) &c && old != UINTPTR_MAX;
#else
    int volatile narrow = 0;
#endif
    c.prev = robigo_top;
    robigo_top = &c;
    int r = 1;
    if (setjmp(c.jb) == 0) {
#if defined(__x86_64__)
        *out = robigo_call65_stub(f, args, narrow ? scan_top : 0);
#else
        robigo_clear_stack();
        *out = ((F65) f)(A65(args));
#endif
        r = 0;
    }
    robigo_top = c.prev;
    if (narrow)
        *scan_top = old;
    return r;
}

/* Package callbacks of up to six word arguments returning a word or
 * nothing (ALTREP methods, TRE): the same SysV rule as robigo_try_call65
 * lets one shape serve them all. */
typedef SX (*F6)(SX, SX, SX, SX, SX, SX);

int robigo_try_w6(void *f, SX *a, SX *out)
{
    TRY(*out = ((F6) f)(a[0], a[1], a[2], a[3], a[4], a[5]))
}

/* double f(SX, SX): ALTREAL's Elt */
int robigo_try_d2(void *f, SX a0, SX a1, double *out)
{
    TRY(*out = ((double (*)(SX, SX)) f)(a0, a1))
}

/* Rcomplex f(SX, SX): ALTCOMPLEX's Elt */
typedef struct { double r, i; } robigo_cplx;
int robigo_try_c2(void *f, SX a0, SX a1, robigo_cplx *out)
{
    TRY(*out = ((robigo_cplx (*)(SX, SX)) f)(a0, a1))
}
