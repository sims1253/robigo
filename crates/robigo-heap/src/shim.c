/* robigo-heap's C shim: what Rust cannot express.
 *
 * - Native entries for non-local exits: Rust cannot call setjmp (it
 *   returns twice), so every entry into code that may longjmp (package C
 *   code, R_ToplevelExec's fun, R_UnwindProtect's fun, a cend, ...) goes
 *   through a robigo_heap_try_* trampoline, which sets the jump record it is
 *   given and calls the code with no Rust frame in between, and an exit
 *   leaves through robigo_heap_jump to such a record, exactly like GNU R's
 *   contexts (src/main/context.c). The record is a field of the runtime's
 *   context record (robigo-rtcore's RCNTXT, its cjmpbuf), so the runtime's
 *   one context chain is also the list of jump targets: the shim keeps no
 *   list and no thread-local of its own.
 * - Every native entry opens a C segment of the conservative scan
 *   (docs/g3.md §3): its top just above the callee's stack arguments
 *   (robigo_heap_try_call65) or at the call (robigo_heap_try_call6), and the
 *   previous top comes back as the callee returns or jumps. And the C frame
 *   the scan runs behind (robigo_heap_spill).
 *
 * Built without unwind tables (build.rs): nothing may unwind through these
 * frames, and without tables a C++ exception cannot. */
#include <setjmp.h>
#include <stddef.h>
#include <stdint.h>

/* The jump record: GNU R's JMP_BUF under HAVE_POSIX_SETJMP (Defn.h), set
 * with sigsetjmp(x, 0), so no signal mask is saved or restored. Its size is
 * fixed per platform on the Rust side too (unwind.rs, JmpBuf); the assertion
 * keeps the two in step. */
#if defined(__linux__) && defined(__x86_64__)
#define ROBIGO_HEAP_JMPBUF_SIZE 200
#elif defined(__linux__) && defined(__aarch64__)
#define ROBIGO_HEAP_JMPBUF_SIZE 312
#elif defined(__APPLE__) && defined(__x86_64__)
#define ROBIGO_HEAP_JMPBUF_SIZE 152
#elif defined(__APPLE__) && defined(__aarch64__)
#define ROBIGO_HEAP_JMPBUF_SIZE 196
#else
#error "robigo-heap: no sigjmp_buf size for this platform (shim.c and unwind.rs)"
#endif
_Static_assert(sizeof(sigjmp_buf) == ROBIGO_HEAP_JMPBUF_SIZE, "sigjmp_buf differs from unwind.rs's JmpBuf");

size_t robigo_heap_jmpbuf_size(void)
{
    return sizeof(sigjmp_buf);
}

size_t robigo_heap_jmpbuf_align(void)
{
    return _Alignof(sigjmp_buf);
}

/* Jump to the record jb, which a robigo_heap_try_* trampoline still running
 * set: that trampoline returns 1. */
_Noreturn void robigo_heap_jump(sigjmp_buf *jb)
{
    siglongjmp(*jb, 1);
}

/* Call cb behind a C frame: the collector's scan of the C segments, whose
 * reads of other frames' memory are then opaque to the Rust compiler. (The
 * spill no longer matters to the scan: C's registers are in the segments,
 * spilled by the exports that closed them, and the collector's own frames
 * are not read.) */
void robigo_heap_spill(void (*cb)(void *), void *ctx)
{
    __builtin_unwind_init();
    cb(ctx);
    __asm__ volatile("" ::: "memory");
}

#if !defined(__x86_64__)
/* (x86-64: robigo_heap_call65_stub zeroes in place) */
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
__attribute__((ROBIGO_NO_CANARY noinline)) static void robigo_heap_clear_stack(void)
{
    char buf[1024];
    __builtin_memset(buf, 0, sizeof buf);
    __asm__ volatile("" : : "r"(buf) : "memory");
}
#endif

/* ---- C trampolines: native code runs with no Rust frame between the
 * record's sigsetjmp and the code that may longjmp (council item 17). Each
 * returns 0 on a normal return, 1 if robigo_heap_jump(jb) unwound to it. */

typedef void *SX;

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

#if defined(__x86_64__)
/* f(args[0], ..., args[5 + nstack]) with an exact stack layout (SysV
 * x86-64: six register arguments, the rest on the stack), which a C call
 * does not give: the compiler chooses where the trampoline's frame ends and
 * the outgoing arguments begin. The stub reserves the nstack (at most 59)
 * stack-argument slots the call passes, and one zero word above them when
 * nstack is even, which keeps the call 16-byte aligned; fills them; and, if
 * scan_top is not null, opens the C segment at the first word above them.
 * The routine's incoming parameters are then scanned, whatever it stores in
 * them (a C local, §3), and nothing above is: the C trampoline's frame (its
 * jmp_buf has a signal mask _setjmp never writes) and the Rust frames keep
 * objects only through precise roots. The stack below is not zeroed: under
 * pins a stale word there keeps memory, never decides a weak reference or a
 * finalizer (docs/g3.md §3 step 3), and the probes and the stress script
 * showed no difference without it (ledger 2026-11-14, A1's capi cases).
 * The Rust caller's callee-saved registers are saved above the segment and
 * zeroed before the call: C code that leaves one alone would otherwise hand
 * a Rust value to the export it calls, whose entry pushes those registers
 * into the segment as C's. rdi = f, rsi = args, rdx = scan_top, rcx =
 * nstack; the caller restores the top. */
__attribute__((naked, noinline)) static SX robigo_heap_call65_stub(__attribute__((unused)) void *f, __attribute__((unused)) SX *args,
                                                             __attribute__((unused)) uintptr_t *scan_top,
                                                             __attribute__((unused)) uintptr_t nstack)
{
    __asm__ volatile(
        "push %rbp\n\t"
        "mov %rsp, %rbp\n\t"
        /* the caller's callee-saved registers: saved here, above the
         * segment, and zeroed before the call */
        "push %rbx\n\t"
        "push %r12\n\t"
        "push %r13\n\t"
        "push %r14\n\t"
        "push %r15\n\t"
        "mov %rdi, %r11\n\t"
        "mov %rdx, %rbx\n\t"
        /* (nstack | 1) slots: rax = their bytes; the highest one zeroed,
         * which the last argument overwrites when nstack is odd */
        "mov %rcx, %rax\n\t"
        "or $1, %rax\n\t"
        "shl $3, %rax\n\t"
        "sub %rax, %rsp\n\t"
        "movq $0, -8(%rsp,%rax)\n\t"
        /* args[6..6 + nstack] into them */
        "xor %edx, %edx\n\t"
        "jmp 2f\n\t"
        "1:\n\t"
        "mov 48(%rsi,%rdx,8), %rdi\n\t"
        "mov %rdi, (%rsp,%rdx,8)\n\t"
        "inc %rdx\n\t"
        "2:\n\t"
        "cmp %rcx, %rdx\n\t"
        "jb 1b\n\t"
        /* the segment's top: the first word above them */
        "test %rbx, %rbx\n\t"
        "jz 3f\n\t"
        "add %rsp, %rax\n\t"
        "mov %rax, (%rbx)\n\t"
        "3:\n\t"
        "mov (%rsi), %rdi\n\t"
        "mov 16(%rsi), %rdx\n\t"
        "mov 24(%rsi), %rcx\n\t"
        "mov 32(%rsi), %r8\n\t"
        "mov 40(%rsi), %r9\n\t"
        "mov 8(%rsi), %rsi\n\t"
        /* the routine starts with no register of the Rust caller: an
         * export it calls pushes the callee-saved ones into the segment as
         * C's values */
        "xor %ebx, %ebx\n\t"
        "xor %r12d, %r12d\n\t"
        "xor %r13d, %r13d\n\t"
        "xor %r14d, %r14d\n\t"
        "xor %r15d, %r15d\n\t"
        "call *%r11\n\t"
        "lea -40(%rbp), %rsp\n\t"
        "pop %r15\n\t"
        "pop %r14\n\t"
        "pop %r13\n\t"
        "pop %r12\n\t"
        "pop %rbx\n\t"
        "pop %rbp\n\t"
        "ret\n\t");
}
#endif

/* A package routine (.Call, .External, .C) of nargs arguments, args has 65
 * slots (the unused ones R_NilValue / NULL), which the caller protected: a
 * native entry. The stub passes the first nargs (at least six, in registers:
 * a callee ignores what it does not declare), as dotcode.c's switch over
 * nargs does; the plain call passes all 65.
 *
 * scan_top, if not null, is the core's open segment (Core::scan_top_slot):
 * its top (0 while no C code runs), then the routine it holds, which the
 * pin report names; both come back as the callee returns or jumps. While
 * the routine runs, a C segment of the
 * conservative scan is open from the routine's stack arguments down
 * (robigo_heap_call65_stub): the routine's frames and those of the C code it
 * calls, up to an export, which closes the segment before it may collect.
 * The frames above, this one's included, keep objects only through precise
 * roots, and the words they leave unwritten are stale ones of earlier frames
 * at that depth. The top has to be exact at the call: taken in Rust it left
 * an inlined frame's slots in the scan (a dead object kept alive in
 * optimized builds), and from a sampled stack pointer in C it left the
 * routine's stack parameters out with Clang, which reserves the outgoing
 * arguments in the prologue (a live object freed). Elsewhere than x86-64 the
 * segment opens at this frame's frame address, above the outgoing arguments
 * (GCC puts the locals above it, Clang below: then the jmp_buf's stale words
 * are scanned too), and the stack below is zeroed (ledger 2026-11-11). The
 * previous top comes back as the routine returns or jumps. */
/* Experiment A (docs/g3.md): plain (Core::call65_plain, ROBIGO_CALL65_PLAIN=1
 * when the core was made) calls the routine directly, without the stub's
 * narrowing (and its argument count): the segment opens at this frame's
 * frame address.
 *
 * jb is the jump record of the native entry's context: an exit out of the
 * routine (robigo_heap_jump(jb)) returns 1 here. */
int robigo_heap_try_call65(sigjmp_buf *jb, void *f, SX *args, SX *out, uintptr_t *scan_top, int plain, int nargs)
{
    /* scan_top[0] is the open segment's top, scan_top[1] its routine */
    uintptr_t volatile old = scan_top ? scan_top[0] : 0;
    uintptr_t volatile old_routine = scan_top ? scan_top[1] : 0;
    int volatile r = 1;
    if (scan_top)
        scan_top[1] = (uintptr_t) f;
    if (sigsetjmp(*jb, 0) == 0) {
        if (plain) {
            if (scan_top)
                *scan_top = (uintptr_t) __builtin_frame_address(0);
            *out = ((F65) f)(A65(args));
        } else
#if defined(__x86_64__)
            /* (the stub opens the segment below this frame) */
            *out = robigo_heap_call65_stub(f, args, scan_top,
                                           nargs > 65 ? 59 : nargs > 6 ? (uintptr_t) nargs - 6 : 0);
#else
        {
            if (scan_top)
                *scan_top = (uintptr_t) __builtin_frame_address(0);
            robigo_heap_clear_stack();
            *out = ((F65) f)(A65(args));
        }
#endif
        r = 0;
    }
    if (scan_top) {
        scan_top[0] = old;
        scan_top[1] = old_routine;
    }
    return r;
}

/* ---- Native entries of up to six word arguments: package ALTREP methods,
 * R_ToplevelExec's and R_UnwindProtect's fun, C finalizers, cend cleanups.
 * On the SysV ABIs these pass every argument in a register (x86-64 has six,
 * AArch64 eight), so one call shape serves them all, and a callee ignores
 * arguments it does not declare. */

typedef struct { double r, i; } robigo_heap_cplx;

/* What the callee returns: in rax (a word or nothing), xmm0 (a double),
 * xmm0:xmm1 (Rcomplex, a struct of two doubles). */
enum { ROBIGO_HEAP_RET_WORD = 0, ROBIGO_HEAP_RET_DOUBLE = 1, ROBIGO_HEAP_RET_COMPLEX = 2 };

#if defined(__x86_64__)
/* f(a[0], ..., a[5]) as a native entry: the mirror of
 * robigo_heap_call65_stub with no stack arguments. Zeroes the 1 KiB below
 * its saved registers and, if scan_top is not null, opens the C segment at
 * its stack pointer at the call: the callee's return address and frames are
 * scanned, the stub's saved registers (the caller's callee-saved ones too,
 * zeroed before the call, as in robigo_heap_call65_stub) and everything
 * above are not. The
 * epilogue touches neither rax/rdx nor xmm0/xmm1, so the stub returns
 * whatever the callee returned there: the trampoline below calls it with
 * the callee's return type. rdi = f, rsi = a, rdx = scan_top; the caller
 * restores the top. */
__attribute__((naked, noinline)) static uintptr_t robigo_heap_call6_stub(__attribute__((unused)) const void *f,
                                                                         __attribute__((unused)) const uintptr_t *a,
                                                                         __attribute__((unused)) uintptr_t *scan_top)
{
    __asm__ volatile(
        "push %rbp\n\t"
        "mov %rsp, %rbp\n\t"
        /* the caller's callee-saved registers: saved here, above the
         * segment, and zeroed before the call */
        "push %rbx\n\t"
        "push %r12\n\t"
        "push %r13\n\t"
        "push %r14\n\t"
        "push %r15\n\t"
        /* (16-byte alignment at the call) */
        "sub $8, %rsp\n\t"
        "mov %rdi, %r11\n\t"
        "mov %rdx, %rbx\n\t"
        /* zero the 1 KiB below */
        "pxor %xmm0, %xmm0\n\t"
        "lea -1024(%rsp), %rdi\n\t"
        "mov $8, %ecx\n\t"
        "1:\n\t"
        "movups %xmm0, (%rdi)\n\t"
        "movups %xmm0, 16(%rdi)\n\t"
        "movups %xmm0, 32(%rdi)\n\t"
        "movups %xmm0, 48(%rdi)\n\t"
        "movups %xmm0, 64(%rdi)\n\t"
        "movups %xmm0, 80(%rdi)\n\t"
        "movups %xmm0, 96(%rdi)\n\t"
        "movups %xmm0, 112(%rdi)\n\t"
        "add $128, %rdi\n\t"
        "dec %ecx\n\t"
        "jnz 1b\n\t"
        /* the segment's top: the stack pointer at the call */
        "test %rbx, %rbx\n\t"
        "jz 2f\n\t"
        "mov %rsp, (%rbx)\n\t"
        "2:\n\t"
        "mov (%rsi), %rdi\n\t"
        "mov 16(%rsi), %rdx\n\t"
        "mov 24(%rsi), %rcx\n\t"
        "mov 32(%rsi), %r8\n\t"
        "mov 40(%rsi), %r9\n\t"
        "mov 8(%rsi), %rsi\n\t"
        /* the routine starts with no register of the Rust caller: an
         * export it calls pushes the callee-saved ones into the segment as
         * C's values */
        "xor %ebx, %ebx\n\t"
        "xor %r12d, %r12d\n\t"
        "xor %r13d, %r13d\n\t"
        "xor %r14d, %r14d\n\t"
        "xor %r15d, %r15d\n\t"
        "call *%r11\n\t"
        "lea -40(%rbp), %rsp\n\t"
        "pop %r15\n\t"
        "pop %r14\n\t"
        "pop %r13\n\t"
        "pop %r12\n\t"
        "pop %rbx\n\t"
        "pop %rbp\n\t"
        "ret\n\t");
}
#endif

/* *out = f(a[0], ..., a[5]) as a native entry, ret saying how f returns
 * (out may be null for a word or nothing); 1 if robigo_heap_jump(jb)
 * unwound to it, and then *out is not written. scan_top as for
 * robigo_heap_try_call65: while f runs a C segment is open from the call
 * down (on x86-64 exactly; elsewhere from this frame's frame address, with
 * the stack below zeroed), and the previous top comes back as f returns or
 * jumps. */
int robigo_heap_try_call6(sigjmp_buf *jb, const void *f, const uintptr_t *a, int ret, void *out, uintptr_t *scan_top)
{
    uintptr_t volatile old = scan_top ? scan_top[0] : 0;
    uintptr_t volatile old_routine = scan_top ? scan_top[1] : 0;
    int volatile r = 1;
    if (scan_top)
        scan_top[1] = (uintptr_t) f;
    if (sigsetjmp(*jb, 0) == 0) {
#if defined(__x86_64__)
        /* (the stub opens the segment below this frame; it returns in the
         * registers f returned in, so it is called with f's return type,
         * through an address the compiler cannot trace back to the
         * definition) */
        const void *stub = (const void *) robigo_heap_call6_stub;
        __asm__("" : "+r"(stub));
        switch (ret) {
        case ROBIGO_HEAP_RET_DOUBLE:
            *(double *) out = ((double (*)(const void *, const uintptr_t *, uintptr_t *)) stub)(f, a, scan_top);
            break;
        case ROBIGO_HEAP_RET_COMPLEX:
            *(robigo_heap_cplx *) out = ((robigo_heap_cplx (*)(const void *, const uintptr_t *, uintptr_t *)) stub)(f, a, scan_top);
            break;
        default: {
            uintptr_t w = ((uintptr_t (*)(const void *, const uintptr_t *, uintptr_t *)) stub)(f, a, scan_top);
            if (out)
                *(uintptr_t *) out = w;
        }
        }
#else
        if (scan_top)
            *scan_top = (uintptr_t) __builtin_frame_address(0);
        robigo_heap_clear_stack();
        switch (ret) {
        case ROBIGO_HEAP_RET_DOUBLE:
            *(double *) out = ((double (*)(uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t)) f)(a[0], a[1], a[2], a[3], a[4], a[5]);
            break;
        case ROBIGO_HEAP_RET_COMPLEX:
            *(robigo_heap_cplx *) out = ((robigo_heap_cplx (*)(uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t)) f)(a[0], a[1], a[2], a[3], a[4], a[5]);
            break;
        default: {
            uintptr_t w = ((uintptr_t (*)(uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t, uintptr_t)) f)(a[0], a[1], a[2], a[3], a[4], a[5]);
            if (out)
                *(uintptr_t *) out = w;
        }
        }
#endif
        r = 0;
    }
    if (scan_top) {
        scan_top[0] = old;
        scan_top[1] = old_routine;
    }
    return r;
}
